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

pub fn parse_user_input<'s>(prompt: &'s str, skills: &'s [Skill]) -> &'s str {
    if let Some(skill) = prompt.strip_prefix("/")
        && let Some(skill) = skills.iter().find(|s| s.name == skill)
    {
        return skill.content.as_str();
    }

    prompt
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
