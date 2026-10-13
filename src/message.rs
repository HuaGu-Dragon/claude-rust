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

pub fn parse_user_input(mut input: &str, skills: &[Skill]) -> Vec<Message> {
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
            .map(|s| {
                skills::parse_skill_content(
                    &s.content,
                    &input.split_ascii_whitespace().collect::<Vec<_>>(),
                )
            })
            .map(|message| Message::User { content: message })
            .collect()
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

#[test]
fn test_content_parse() {
    let skill = Skill {
        name: "apple".to_string(),
        description: "".to_string(),
        content: "Deploy to $0 in region $1. Full request was: $ARGUMENTS".to_string(),
    };

    assert_eq!(
        parse_user_input("/apple staging eu-west", &[skill]),
        vec![Message::User {
            content: "Deploy to staging in region eu-west. Full request was: staging eu-west"
                .to_string()
        }]
    );
}

#[test]
fn test_multiple_skills() {
    let apple = Skill {
        name: "apple".to_string(),
        description: "".to_string(),
        content: "Add this exact line to your response: blueberry-$ARGUMENTS".to_string(),
    };
    let grape = Skill {
        name: "grape".to_string(),
        description: "".to_string(),
        content: "Add this exact line to your response: cherry-$ARGUMENTS".to_string(),
    };

    assert_eq!(
        parse_user_input("/apple /grape 4127", &[apple, grape]),
        vec![
            Message::User {
                content: "Add this exact line to your response: blueberry-4127".to_string()
            },
            Message::User {
                content: "Add this exact line to your response: cherry-4127".to_string()
            }
        ]
    );
}
