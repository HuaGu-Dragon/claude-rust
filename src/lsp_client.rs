use anyhow::{Context, bail};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Default)]
struct DiagnosticState {
    /// Cursor for the server's notification queue.
    cursor: i64,
    /// Diagnostics most recently shown to the model, per document URI.
    shown: HashMap<String, Vec<Value>>,
}

pub struct LspClient {
    address: String,
    path: String,
    pub extension_to_language: HashMap<String, String>,
    /// Documents open on this connection: URI -> last version sent.
    documents: Mutex<HashMap<String, i64>>,
    diagnostics: Mutex<DiagnosticState>,
    /// Set when a Write synchronized a document, so the caller knows to give
    /// the server time to analyse it before reading diagnostics.
    awaiting_diagnostics: AtomicBool,
}

impl LspClient {
    pub fn from_environment() -> anyhow::Result<Option<Self>> {
        if std::env::var_os("CODECRAFTERS_LSP_URL").is_none() {
            return Ok(None);
        }
        Ok(Some(Self::new()?))
    }

    pub fn new() -> anyhow::Result<Self> {
        let url = std::env::var("CODECRAFTERS_LSP_URL")?;
        let local = url
            .strip_prefix("http://127.0.0.1:")
            .context("Expected local LSP transport")?;
        let (port, path) = local.split_once('/').unwrap_or((local, ""));
        let mut client = Self {
            address: format!("127.0.0.1:{port}"),
            path: if path.is_empty() {
                String::new()
            } else {
                format!("/{path}")
            },
            extension_to_language: HashMap::new(),
            documents: Mutex::new(HashMap::new()),
            diagnostics: Mutex::new(DiagnosticState::default()),
            awaiting_diagnostics: AtomicBool::new(false),
        };
        let configuration = client.call("configuration", &serde_json::json!({}))?;
        client.extension_to_language =
            serde_json::from_value(configuration["extensionToLanguage"].clone())?;
        Ok(client)
    }

    pub fn uri_for(file_path: &str) -> anyhow::Result<String> {
        let abs = std::path::absolute(file_path)
            .with_context(|| format!("could not resolve path `{file_path}`"))?;
        Ok(format!("file://{}", abs.display()))
    }

    pub fn language_id(&self, file_path: &str) -> Option<&str> {
        let ext = Path::new(file_path).extension()?.to_str()?;
        self.extension_to_language
            .get(&format!(".{ext}"))
            .map(String::as_str)
    }

    pub fn ensure_open(&self, file_path: &str) -> anyhow::Result<String> {
        let uri = Self::uri_for(file_path)?;
        let mut documents = self.documents.lock().unwrap();
        if documents.contains_key(&uri) {
            return Ok(uri);
        }
        let language_id = self
            .language_id(file_path)
            .with_context(|| format!("no language server configured for `{file_path}`"))?;
        let text = std::fs::read_to_string(file_path)
            .with_context(|| format!("could not open file `{file_path}`"))?;
        self.send_did_open(&uri, language_id, 1, &text)?;
        documents.insert(uri.clone(), 1);
        Ok(uri)
    }

    pub fn sync_document(&self, file_path: &str, text: &str) -> anyhow::Result<bool> {
        let Some(language_id) = self.language_id(file_path) else {
            return Ok(false);
        };
        let uri = Self::uri_for(file_path)?;
        let mut documents = self.documents.lock().unwrap();
        let version = match documents.get(&uri) {
            Some(previous) => {
                let version = previous + 1;
                self.call(
                    "notify",
                    &serde_json::json!({
                        "method": "textDocument/didChange",
                        "params": {
                            "textDocument": { "uri": uri, "version": version },
                            "contentChanges": [{ "text": text }],
                        }
                    }),
                )?;
                version
            }
            None => {
                self.send_did_open(&uri, language_id, 1, text)?;
                1
            }
        };
        documents.insert(uri, version);
        self.awaiting_diagnostics.store(true, Ordering::SeqCst);
        Ok(true)
    }

    /// Returns true (and resets the flag) if a document was synchronized
    /// since the last call, meaning diagnostics may still be on their way.
    pub fn take_awaiting_diagnostics(&self) -> bool {
        self.awaiting_diagnostics.swap(false, Ordering::SeqCst)
    }

    /// Reads queued `publishDiagnostics` notifications and returns a message
    /// describing diagnostics that weren't already shown for their file.
    ///
    /// A report is current if its version matches the last version sent for
    /// that document, or if it carries no version. The latest current report
    /// for a file replaces the previous one, so an empty report clears it.
    pub fn new_diagnostics(&self) -> anyhow::Result<Option<String>> {
        let mut state = self.diagnostics.lock().unwrap();
        let response = self.call(
            "notifications",
            &serde_json::json!({ "after": state.cursor, "timeoutMs": 0 }),
        )?;
        if let Some(cursor) = response["cursor"].as_i64() {
            state.cursor = cursor;
        }
        let notifications = response["notifications"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let mut latest: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        {
            let documents = self.documents.lock().unwrap();
            for notification in &notifications {
                if notification["method"] != "textDocument/publishDiagnostics" {
                    continue;
                }
                let params = &notification["params"];
                let Some(uri) = params["uri"].as_str() else {
                    continue;
                };
                if let Some(version) = params["version"].as_i64()
                    && documents.get(uri) != Some(&version)
                {
                    continue;
                }
                let diagnostics = params["diagnostics"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default();
                latest.insert(uri.to_string(), diagnostics);
            }
        }

        let mut report = String::new();
        for (uri, current) in latest {
            let previous = state.shown.get(&uri).map(Vec::as_slice).unwrap_or_default();
            let mut issues: Vec<(i64, i64, &str)> = current
                .iter()
                .filter(|d| !previous.contains(d))
                .filter_map(|d| {
                    Some((
                        d["range"]["start"]["line"].as_i64()? + 1,
                        d["range"]["start"]["character"].as_i64()? + 1,
                        d["message"].as_str()?,
                    ))
                })
                .collect();
            issues.sort();

            if !issues.is_empty() {
                report.push_str(&format!("\n{}:\n", Self::display_path(&uri)));
                for (line, character, message) in issues {
                    report.push_str(&format!("  [Line {line}:{character}] {message}\n"));
                }
            }
            state.shown.insert(uri, current);
        }

        if report.is_empty() {
            return Ok(None);
        }
        Ok(Some(format!(
            "<new-diagnostics>The following new diagnostic issues were detected. \
             Treat them as feedback on your recent edits and fix any that your changes caused:\n{}</new-diagnostics>",
            report.trim_end()
        )))
    }

    pub fn display_path(uri: &str) -> String {
        let raw = uri.strip_prefix("file://").unwrap_or(uri);
        match std::env::current_dir() {
            Ok(cwd) => Path::new(raw)
                .strip_prefix(&cwd)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| raw.to_string()),
            Err(_) => raw.to_string(),
        }
    }

    fn send_did_open(
        &self,
        uri: &str,
        language_id: &str,
        version: i64,
        text: &str,
    ) -> anyhow::Result<()> {
        self.call(
            "notify",
            &serde_json::json!({
                "method": "textDocument/didOpen",
                "params": {
                    "textDocument": {
                        "uri": uri,
                        "languageId": language_id,
                        "version": version,
                        "text": text,
                    }
                }
            }),
        )?;
        Ok(())
    }

    pub fn call(&self, action: &str, payload: &Value) -> anyhow::Result<Value> {
        let body = serde_json::to_string(payload)?;
        let mut stream = TcpStream::connect(&self.address)?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        write!(
            stream,
            "POST {}/{} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            self.path,
            action,
            self.address,
            body.len(),
            body
        )?;
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        let (headers, result) = response
            .split_once("\r\n\r\n")
            .context("Invalid transport response")?;
        if headers.split_whitespace().nth(1) != Some("200") {
            bail!("{}", result)
        }
        Ok(serde_json::from_str(result)?)
    }
}
