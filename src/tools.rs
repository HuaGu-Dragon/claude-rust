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
                            "enum": ["goToDefinition"]
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

                    messages.push(Message::Tool {
                        tool_call_id: self.id,
                        content: content.to_string(),
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
                        // let operation = args.get("operation").expect("there must be an operation");
                        // let file_path = args.get("filePath").expect("there must be a file path");
                        // let line = args.get("line").expect("there must be a line");
                        // let character = args.get("character").expect("there must be a character");

                        // lsp.call(operation, &serde_json::t);
                        messages.push(Message::Tool {
                            tool_call_id: self.id,
                            content: "LSP is not implemented yet".to_string(),
                        });
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
