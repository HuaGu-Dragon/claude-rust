use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum Message {
    User { content: String },
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
