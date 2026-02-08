use std::collections::HashMap;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::message::Message;

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
    pub args: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ToolName {
    Read,
    Write,
    Bash,
}

mod deserialize_string_to_hashmap {
    use std::collections::HashMap;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(table: &HashMap<String, String>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let s = format!("{:?}", table);
        serializer.serialize_str(&s)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let table = serde_json::from_str(&s).expect("must parse as a hash table");

        Ok(table)
    }
}

impl Tool {
    pub async fn exec(self, messages: &mut Vec<Message>) -> anyhow::Result<()> {
        match self.ty {
            ToolTy::Function {
                function: FunctionCall { name, args },
            } => match name {
                ToolName::Read => {
                    let path = args.get("file_path").expect("there must be a file_path");

                    let content = tokio::fs::read_to_string(path)
                        .await
                        .with_context(|| format!("read `{path}`'s content"))?;

                    messages.push(Message::Tool {
                        tool_call_id: self.id,
                        content,
                    });
                }
                ToolName::Write => {
                    let path = args.get("file_path").expect("there must be a file_path");
                    let content = args.get("content").expect("there must be a content");

                    tokio::fs::write(path, content)
                        .await
                        .with_context(|| format!("write content to `{path}`"))?;

                    messages.push(Message::Tool {
                        tool_call_id: self.id,
                        content: content.clone(),
                    });
                }
                ToolName::Bash => {
                    let com = args.get("command").expect("there must be a command");
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
            },
        }

        Ok(())
    }
}

#[test]
fn test_de() {
    let mut table = HashMap::new();
    table.insert("file_path".to_string(), "/path/to/file.txt".to_string());

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
