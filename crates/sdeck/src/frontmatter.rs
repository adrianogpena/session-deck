//! Pulls fields out of a Markdown file's YAML frontmatter, port of `frontmatter.ts`. Not a real
//! YAML parser: it handles a plain scalar or a folded (`>`) / literal (`|`) block scalar, which
//! covers every skill and agent file in practice.

use std::sync::LazyLock;

use regex::Regex;

static QUOTES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"^['"]|['"]$"#).expect("static pattern"));
static BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)^---\r?\n(.*?)\r?\n---").expect("static pattern"));

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameAndDescription {
    pub name: Option<String>,
    pub description: Option<String>,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

/// One field of a frontmatter `body` (the text between the `---` lines).
pub fn extract_frontmatter_field(body: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = body.lines().collect();
    let prefix = format!("{key}:");
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix(&prefix) else {
            continue;
        };
        let rest = rest.trim();
        if matches!(rest, ">" | ">-" | "|" | "|-") {
            // A folded/literal block scalar: the more-indented lines that follow, folded onto one line.
            let indent = indent_of(line);
            let mut collected = Vec::new();
            for next in &lines[i + 1..] {
                if next.trim().is_empty() {
                    continue;
                }
                if indent_of(next) <= indent {
                    break;
                }
                collected.push(next.trim());
            }
            return Some(collected.join(" "));
        }
        let value = QUOTES.replace_all(rest, "").into_owned();
        return (!value.is_empty()).then_some(value);
    }
    None
}

/// `name` and `description` from a Markdown file's `---`-delimited frontmatter block.
pub fn parse_name_and_description(text: &str) -> NameAndDescription {
    let Some(body) = BLOCK.captures(text).and_then(|c| c.get(1)) else {
        return NameAndDescription::default();
    };
    NameAndDescription {
        name: extract_frontmatter_field(body.as_str(), "name"),
        description: extract_frontmatter_field(body.as_str(), "description"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_scalars_are_read_and_quotes_stripped() {
        let parsed =
            parse_name_and_description("---\nname: \"a-skill\"\ndescription: Does a thing.\n---\nBody");
        assert_eq!(parsed.name.as_deref(), Some("a-skill"));
        assert_eq!(parsed.description.as_deref(), Some("Does a thing."));
    }

    #[test]
    fn block_scalars_fold_onto_one_line_and_stop_at_the_next_key() {
        let text = "---\nname: x\ndescription: >\n  First line.\n\n  Second line.\ntools: Bash\n---\n";
        assert_eq!(
            parse_name_and_description(text).description.as_deref(),
            Some("First line. Second line.")
        );
    }

    #[test]
    fn missing_block_or_field_gives_none() {
        assert_eq!(
            parse_name_and_description("# Heading"),
            NameAndDescription::default()
        );
        assert_eq!(
            parse_name_and_description("---\nname: only\n---").description,
            None
        );
    }
}
