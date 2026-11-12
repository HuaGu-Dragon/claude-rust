use std::collections::HashMap;

use anyhow::Context;
use async_openai::{Client, config::OpenAIConfig};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{lsp_client, message::Message, skills};

pub fn tools_define() -> Value {
    json!([
        {
            "type": "function",
            "function": {
                "name": "Read",
                "description": "Read and return the contents of a file",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "The path to the file to read"
                        }
                    },
                    "required": ["file_path"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "Write",
                "description": "Write content to a file",
                "parameters": {
                    "type": "object",
                    "required": ["file_path", "content"],
                    "properties": {
                        "file_path": {
                            "type": "string",
                            "description": "The path of the file to write to"
                        },
                        "content": {
                            "type": "string",
                            "description": "The content to write to the file"
                        }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "Bash",
                "description": "Execute a shell command",
                "parameters": {
                    "type": "object",
                    "required": ["command"],
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The command to execute"
                        }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "Skill",
                "description": "Load a skill's instructions into the conversation",
                "parameters": {
                    "type": "object",
                    "required": ["name"],
                    "properties": {
                        "name": { "type": "string", "description": "The name of the skill to use" },
                        "args": { "type": "string", "description": "Optional arguments for the skill" }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "LSP",
                "description": "Returns the file, line, and character where a symbol is defined.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "operation": {
                            "type": "string",
                            "enum": ["goToDefinition", "findReferences"]
                        },
                        "filePath": {
                            "type": "string",
                            "description": "Absolute path, or a path relative to the agent's current working directory."
                        },
                        "line": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "1-based line number of the symbol."
                        },
                        "character": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "1-based character offset in UTF-16 code units (same as character count for ASCII)."
                        }
                    },
                    "required": ["operation", "filePath", "line", "character"]
                }
            }
        }
    ])
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tool {
    pub id: String,
    #[serde(flatten)]
    pub ty: ToolTy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolTy {
    Function { function: FunctionCall },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionCall {
    pub name: ToolName,
    #[serde(rename = "arguments", with = "deserialize_string_to_hashmap")]
    pub args: HashMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolName {
    Read,
    Write,
    Bash,
    Skill,
    #[serde(rename = "LSP")]
    Lsp,
}

mod deserialize_string_to_hashmap {
    use std::collections::HashMap;

    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::Value;

    pub fn serialize<S>(table: &HashMap<String, Value>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = serde_json::to_string(table).map_err(serde::ser::Error::custom)?;
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<String, Value>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        serde_json::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl Tool {
    pub async fn exec(
        self,
        client: &Client<OpenAIConfig>,
        messages: &mut Vec<Message>,
        skills: &[skills::Skill],
        lsp: &Option<lsp_client::LspClient>,
        is_sub: bool,
    ) -> anyhow::Result<()> {
        match self.ty {
            ToolTy::Function {
                function: FunctionCall { name, args },
            } => match name {
                ToolName::Read => {
                    let path = str_arg(&args, "file_path");

                    match tokio::fs::read_to_string(path).await {
                        Ok(content) => messages.push(Message::Tool {
                            tool_call_id: self.id,
                            content,
                        }),
                        Err(e) => messages.push(Message::Tool {
                            tool_call_id: self.id,
                            content: e.to_string(),
                        }),
                    };
                }
                ToolName::Write => {
                    let path = str_arg(&args, "file_path");
                    let content = str_arg(&args, "content");

                    tokio::fs::write(path, content)
                        .await
                        .with_context(|| format!("write content to `{path}`"))?;

                    let content = match lsp {
                        Some(lsp) => match lsp.sync_document(path, content) {
                            Ok(_) => content.to_string(),
                            Err(e) => format!(
                                "Wrote `{path}`, but failed to sync it with the language server: {e:#}"
                            ),
                        },
                        None => content.to_string(),
                    };

                    messages.push(Message::Tool {
                        tool_call_id: self.id,
                        content,
                    });
                }
                ToolName::Bash => {
                    let com = str_arg(&args, "command");
                    let output = std::process::Command::new("bash")
                        .arg("-c")
                        .arg(com)
                        .output()
                        .context("spawn a child process to exec bash command")?;

                    let content = if output.status.success() {
                        output.stdout
                    } else {
                        output.stderr
                    };

                    messages.push(Message::Tool {
                        tool_call_id: self.id,
                        content: String::from_utf8_lossy(&content).to_string(),
                    });
                }
                ToolName::Skill => {
                    let skill_name = str_arg(&args, "name");
                    let args = args.get("args").and_then(Value::as_str).unwrap_or_default();
                    let skill = skills
                        .iter()
                        .find(|s| s.name == skill_name)
                        .with_context(|| format!("could not find skill `{skill_name}`"))?;

                    let ret = skill.execute(client, args, lsp, is_sub).await;
                    messages.push(Message::Tool {
                        tool_call_id: self.id,
                        content: ret,
                    });
                }
                ToolName::Lsp => {
                    if let Some(lsp) = lsp {
                        let file_path = str_arg(&args, "filePath");
                        let cur_dir = std::env::current_dir().unwrap();

                        // Opens the document only if Write or an earlier
                        // lookup hasn't already done so.
                        let file_uri = lsp.ensure_open(file_path)?;

                        let operation = str_arg(&args, "operation");
                        let line = args
                            .get("line")
                            .expect("there must be a line")
                            .as_i64()
                            .unwrap();
                        let character = args
                            .get("character")
                            .expect("there must be a character")
                            .as_i64()
                            .unwrap();

                        match operation {
                            "goToDefinition" => {
                                let payload = json!({
                                    "method": "textDocument/definition",
                                    "params": {
                                        "textDocument": {
                                            "uri": file_uri,
                                        },
                                        "position": {
                                            "line": line - 1,
                                            "character": character - 1,
                                        }
                                    }
                                });

                                let lsp_result = lsp.call("request", &payload)?;

                                let target_uri = lsp_result["uri"].as_str().unwrap();
                                let target_range_start_char =
                                    lsp_result["range"].as_object().unwrap()["start"]
                                        .as_object()
                                        .unwrap()["character"]
                                        .as_i64()
                                        .unwrap();
                                let target_range_start_line =
                                    lsp_result["range"].as_object().unwrap()["start"]
                                        .as_object()
                                        .unwrap()["line"]
                                        .as_i64()
                                        .unwrap();

                                let relative_file_path =
                                    &target_uri[&cur_dir.to_str().unwrap().len() + 8..];
                                messages.push(Message::Tool {
                                    tool_call_id: self.id,
                                    content: format!(
                                        "Defined in {}:{}:{}",
                                        relative_file_path,
                                        target_range_start_line + 1,
                                        target_range_start_char + 1
                                    ),
                                });
                            }
                            "findReferences" => {
                                let payload = json!({
                                    "method": "textDocument/references",
                                    "params": {
                                        "textDocument": {
                                            "uri": file_uri,
                                        },
                                        "position": {
                                            "line": line - 1,
                                            "character": character - 1,
                                        },
                                        "context": { "includeDeclaration": true }
                                    }
                                });

                                let lsp_result = lsp.call("request", &payload)?;

                                let locations: Vec<Value> = match lsp_result {
                                    Value::Array(a) => a,
                                    Value::Null => Vec::new(),
                                    other => vec![other],
                                };
                                let mut target_uris: std::collections::BTreeMap<
                                    String,
                                    Vec<(i64, i64)>,
                                > = std::collections::BTreeMap::new();
                                for v in &locations {
                                    let (Some(uri), Some(line), Some(character)) = (
                                        v["uri"].as_str(),
                                        v["range"]["start"]["line"].as_i64(),
                                        v["range"]["start"]["character"].as_i64(),
                                    ) else {
                                        continue;
                                    };
                                    target_uris
                                        .entry(uri.to_string())
                                        .or_default()
                                        .push((line + 1, character + 1));
                                }
                                for ranges in target_uris.values_mut() {
                                    ranges.sort();
                                }

                                let mut content = format!(
                                    "Found {} references across {} files:\n\n",
                                    target_uris.values().map(|v| v.len()).sum::<usize>(),
                                    target_uris.len()
                                );

                                for (target_uri, target_ranges) in target_uris.iter() {
                                    let raw =
                                        target_uri.strip_prefix("file://").unwrap_or(target_uri);
                                    let relative_file_path = std::path::Path::new(raw)
                                        .strip_prefix(&cur_dir)
                                        .map(|p| p.display().to_string())
                                        .unwrap_or_else(|_| raw.to_string());

                                    content.push_str(&format!("{}:\n", relative_file_path));

                                    for (line, character) in target_ranges.iter() {
                                        content
                                            .push_str(&format!("  Line {}:{}\n", line, character));
                                    }
                                }
                                messages.push(Message::Tool {
                                    tool_call_id: self.id,
                                    content,
                                });
                            }
                            s => panic!("{}: not yet implemented", s),
                        }
                    } else {
                        messages.push(Message::Tool {
                            tool_call_id: self.id,
                            content: "lsp is not available".to_string(),
                        });
                    }
                }
            },
        }

        Ok(())
    }
}

fn str_arg<'a>(args: &'a HashMap<String, Value>, key: &str) -> &'a str {
    args.get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("there must be a string `{key}` argument"))
}

#[test]
fn test_de() {
    let mut table = HashMap::new();
    table.insert("file_path".to_string(), json!("/path/to/file.txt"));

    let t = Tool {
        id: "call_abc123".to_string(),
        ty: ToolTy::Function {
            function: FunctionCall {
                name: ToolName::Read,
                args: table,
            },
        },
    };

    let input = r#" {
               "id": "call_abc123",
               "type": "function",
               "function": {
                 "name": "Read",
                 "arguments": "{\"file_path\": \"/path/to/file.txt\"}"
               }
             }"#;

    let res: Tool = serde_json::from_str(input).unwrap();

    assert_eq!(res, t);
}
