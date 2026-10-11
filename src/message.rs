use serde::{Deserialize, Serialize};

use crate::{
    skills::{self, Skill},
    tools::Tool,
};

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

pub fn parse_user_input(prompt: &str, skills: &[Skill]) -> String {
    if let Some(arguments) = prompt.strip_prefix("/") {
        let mut invoke = arguments.split_ascii_whitespace();
        if let Some(skill_name) = invoke.next()
            && let Some(skill) = skills.iter().find(|s| s.name == skill_name)
        {
            let args = invoke.collect::<Vec<_>>();
            return skills::parse_skill_content(&skill.content, &args);
        }
    }

    prompt.to_string()
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

#[test]
fn test_content_parse() {
    let skill = Skill {
        name: "apple".to_string(),
        description: "".to_string(),
        content: "Deploy to $0 in region $1. Full request was: $ARGUMENTS".to_string(),
    };

    assert_eq!(
        parse_user_input("/apple staging eu-west", &[skill]),
        "Deploy to staging in region eu-west. Full request was: staging eu-west"
    );
}
