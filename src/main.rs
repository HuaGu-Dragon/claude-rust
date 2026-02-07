use anyhow::Context;
use async_openai::{Client, config::OpenAIConfig};
use clap::Parser;
use serde_json::{Value, json};
use std::{env, process};

use crate::tools::{FunctionCall, Tool, ToolTy::Function};

mod tools;

#[derive(Parser)]
#[command(author, version, about)]
struct Args {
    #[arg(short = 'p', long)]
    prompt: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let base_url = env::var("OPENROUTER_BASE_URL")
        .unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());

    let api_key = env::var("OPENROUTER_API_KEY").unwrap_or_else(|_| {
        eprintln!("OPENROUTER_API_KEY is not set");
        process::exit(1);
    });

    let config = OpenAIConfig::new()
        .with_api_base(base_url)
        .with_api_key(api_key);

    let client = Client::with_config(config);

    let response: Value = client
        .chat()
        .create_byot(json!({
            "messages": [
                {
                    "role": "user",
                    "content": args.prompt
                }
            ],
            "model": "anthropic/claude-haiku-4.5",
            "tools": [
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
                }
            ]
        }))
        .await?;

    // You can use print statements as follows for debugging, they'll be visible when running tests.
    eprintln!("Logs from your program will appear here!");

    // I am just stupid
    if let Some(tools) = response["choices"][0]["message"]["tool_calls"].as_array() {
        for tool in tools {
            let tool: Tool =
                serde_json::from_value(tool.clone()).context("parse response into tool")?;

            match tool.ty {
                Function {
                    function: FunctionCall { name, args },
                } => match name.as_str() {
                    "Read" => {
                        let path = args.get("file_path").expect("there must be a file_path");

                        eprintln!("{path}");
                        // ???
                        let content = tokio::fs::read_to_string(path)
                            .await
                            .with_context(|| format!("read `{path}`'s content"))?;

                        println!("{content}")
                    }
                    _ => unreachable!(),
                },
            }
        }
    }

    if let Some(content) = response["choices"][0]["message"]["content"].as_str() {
        println!("{}", content);
    }

    Ok(())
}
