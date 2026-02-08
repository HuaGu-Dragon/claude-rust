use anyhow::Context;
use async_openai::{Client, config::OpenAIConfig};
use clap::Parser;
use serde_json::{Value, json};
use std::{env, process};

use crate::{
    message::Message,
    tools::{FunctionCall, Tool, ToolName, ToolTy::Function, tools_define},
};

mod message;
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

    let messages = vec![Message::User {
        content: args.prompt,
    }];

    let response: Value = client
        .chat()
        .create_byot(json!({
            "messages": messages,
            "model": "anthropic/claude-haiku-4.5",
            "tools": tools_define()
        }))
        .await?;

    // You can use print statements as follows for debugging, they'll be visible when running tests.
    eprintln!("Logs from your program will appear here!");

    if let Some(tools) = response["choices"][0]["message"]["tool_calls"].as_array() {
        for tool in tools {
            let tool: Tool =
                serde_json::from_value(tool.clone()).context("parse response into tool")?;

            match tool.ty {
                Function {
                    function: FunctionCall { name, args },
                } => match name {
                    ToolName::Read => {
                        let path = args.get("file_path").expect("there must be a file_path");

                        let content = tokio::fs::read_to_string(path)
                            .await
                            .with_context(|| format!("read `{path}`'s content"))?;

                        println!("{content}")
                    }
                },
            }
        }
    }

    if let Some(content) = response["choices"][0]["message"]["content"].as_str() {
        println!("{}", content);
    }

    Ok(())
}
