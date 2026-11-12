use anyhow::{Context, bail};
// Only needed for the LSP extension.
// You can ignore this file until you reach that extension.
// Course-supplied transport. This client talks only to the local HTTP bridge.
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

pub struct LspClient {
    address: String,
    path: String,
    pub extension_to_language: HashMap<String, String>,
    /// Documents open on this connection: URI -> last version sent.
    documents: Mutex<HashMap<String, i64>>,
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
        Ok(true)
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
