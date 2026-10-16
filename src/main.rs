use anyhow::Context;
use async_openai::{Client, config::OpenAIConfig};
use clap::Parser;
use serde_json::{Value, json};
use std::{env, process};

use crate::{
    message::Message,
    tools::{Tool, tools_define},
};

mod message;
mod skills;
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
    let (messages, skills) = {
        if let Ok(skills) = skills::load_skills().await {
            let mut messages = vec![Message::System {
                content: format!(
                    "You have access to the following skills:\n\n{}\nIf a skill matches the user's request, call the Skill tool with its name and follow the instructions it returns.",
                    skills
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            }];
            messages
                .extend(message::execute_user_input(&client, &args.prompt, &skills, false).await);
            (messages, skills)
        } else {
            (
                vec![Message::User {
                    content: args.prompt,
                }],
                vec![],
            )
        }
    };

    let msg = agent_loop(&client, messages, &skills, false).await?;
    println!("{msg}");

    Ok(())
}

async fn agent_loop(
    client: &Client<OpenAIConfig>,
    mut messages: Vec<Message>,
    skills: &[skills::Skill],
    is_sub: bool,
) -> anyhow::Result<String> {
    loop {
        let response: Value = client
            .chat()
            .create_byot(json!({
                "messages": messages,
                "model": "anthropic/claude-haiku-4.5",
                "tools": tools_define()
            }))
            .await?;

        let msg = &response["choices"][0]["message"]["content"];

        let Some(tool_calls) = response["choices"][0]["message"]["tool_calls"].as_array() else {
            let msg: String =
                serde_json::from_value(msg.clone()).context("parse content to string")?;
            break Ok(msg);
        };

        if tool_calls.is_empty() {
            let msg: String =
                serde_json::from_value(msg.clone()).context("parse content to string")?;
            break Ok(msg);
        }

        let tools: Vec<Tool> = tool_calls
            .iter()
            .map(|v| serde_json::from_value(v.clone()))
            .collect::<serde_json::Result<_>>()
            .context("parse response into tool")?;

        messages.push(Message::Assistant {
            content: if msg.is_null() {
                Option::None
            } else {
                serde_json::from_value(msg.clone()).context("parse content to string")?
            },
            tool_calls: tools.clone(),
        });

        for tool in tools {
            tool.exec(client, &mut messages, skills, is_sub)
                .await
                .context("tool running")?;
        }
    }
}
