use std::fmt::Display;

use anyhow::Context;

#[derive(Debug)]
pub struct Skill {
    pub name: String,
    description: String,
    pub content: String,
}

impl Display for Skill {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "-{}: {}", self.name, self.description)
    }
}

fn parse_skill(content: &str) -> Skill {
    let mut lines = content.lines();
    lines.next();
    let name = lines
        .next()
        .unwrap_or_default()
        .trim()
        .trim_start_matches("name: ")
        .to_string();
    let description = lines
        .next()
        .unwrap_or_default()
        .trim()
        .trim_start_matches("description: ")
        .to_string();
    lines.next();

    let content = lines
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    Skill {
        name,
        description,
        content,
    }
}

pub async fn load_skills() -> anyhow::Result<Vec<Skill>> {
    let mut skills = tokio::fs::read_dir(".claude/skills")
        .await
        .context("read skills directory")?;

    let mut results = Vec::new();
    // TODO: use `FutureUnordered` to load skills concurrently
    while let Ok(Some(entry)) = skills.next_entry().await {
        let name = entry.file_name();
        let path = entry.path();
        if path.is_dir() {
            let skill = path.join("SKILL.md");
            if let Ok(contents) = tokio::fs::read_to_string(&skill).await {
                let skill = parse_skill(&contents);

                if Some(skill.name.as_str()) == name.to_str() {
                    results.push(skill);
                }
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_skill() {
        let skill = r#"---
name: apple
description: Reports the deploy target for the apple service.
---

Respond with exactly one word: blueberry

Do not add any other text, punctuation, or formatting."#;
        let skill = parse_skill(skill);

        assert_eq!(skill.name, "apple");
        assert_eq!(
            skill.description,
            "Reports the deploy target for the apple service."
        );
        assert_eq!(
            skill.content,
            "Respond with exactly one word: blueberry\nDo not add any other text, punctuation, or formatting."
        );
    }
}
