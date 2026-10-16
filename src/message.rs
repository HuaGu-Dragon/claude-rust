use async_openai::{Client, config::OpenAIConfig};
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::{Deserialize, Serialize};

use crate::{skills::Skill, tools::Tool};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    User {
        content: String,
    },
    Assistant {
        content: Option<String>,
        tool_calls: Vec<Tool>,
    },
    Tool {
        tool_call_id: String,
        content: String,
    },
    System {
        content: String,
    },
}

pub async fn execute_user_input(
    client: &Client<OpenAIConfig>,
    mut input: &str,
    skills: &[Skill],
    is_sub: bool,
) -> Vec<Message> {
    let mut calling_skills = vec![];
    while let Some(arguments) = input.strip_prefix("/") {
        let (skill_name, rest) = match arguments.split_once(' ') {
            Some(x) => x,
            None => (arguments, ""),
        };
        let Some(skill) = skills.iter().find(|s| s.name == skill_name) else {
            break;
        };
        calling_skills.push(skill);
        input = rest;
    }

    if calling_skills.is_empty() {
        vec![Message::User {
            content: input.to_string(),
        }]
    } else {
        calling_skills
            .into_iter()
            .map(|s| s.execute(client, input, is_sub))
            .collect::<FuturesUnordered<_>>()
            .map(|s| Message::User { content: s })
            .collect::<Vec<_>>()
            .await
    }
}

#[test]
fn test_de() {
    let input = r#"
        {
            "role": "user",
            "content": "hello"
        }
    "#;

    let m = serde_json::from_str(input).unwrap();

    let msg = Message::User {
        content: "hello".to_string(),
    };

    assert_eq!(msg, m);
}
