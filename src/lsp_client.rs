use anyhow::{Context, bail};
// Only needed for the LSP extension.
// You can ignore this file until you reach that extension.
// Course-supplied transport. This client talks only to the local HTTP bridge.
use serde_json::Value;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct LspClient {
    address: String,
    path: String,
    pub extension_to_language: HashMap<String, String>,
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
        };
        let configuration = client.call("configuration", &serde_json::json!({}))?;
        client.extension_to_language =
            serde_json::from_value(configuration["extensionToLanguage"].clone())?;
        Ok(client)
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
