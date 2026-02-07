use std::collections::HashMap;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Tool {
    id: String,
    #[serde(flatten)]
    pub ty: ToolTy,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ToolTy {
    Function { function: FunctionCall },
}

#[derive(Debug, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    #[serde(rename = "arguments", with = "deserialize_string_to_hashmap")]
    pub args: HashMap<String, String>,
}

mod deserialize_string_to_hashmap {
    use std::collections::HashMap;

    use serde::{Deserialize, Deserializer};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<HashMap<String, String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;

        eprintln!("{s}");
        let table = serde_json::from_str(&s).expect("must parse as a hash table");

        Ok(table)
    }
}

#[test]
fn test_de() {
    // let t = Tool {
    //     id: "2123".to_string(),
    //     ty: ToolTy::Function {
    //         function: FunctionCall {
    //             name: "Read".to_string(),
    //             args: HashMap::new(),
    //         },
    //     },
    // };

    let input = r#" {
               "id": "call_abc123",
               "type": "function",
               "function": {
                 "name": "Read",
                 "arguments": "{\"file_path\": \"/path/to/file.txt\"}"
               }
             }"#;

    let _res: Tool = serde_json::from_str(input).unwrap();
}
