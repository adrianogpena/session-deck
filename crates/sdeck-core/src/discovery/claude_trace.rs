use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde_json::Value;

use super::claude_storage::{extract_assistant_display_text, extract_text, is_displayable_user_prompt};
use crate::format::one_line;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceKind {
    User,
    Assistant,
    Tool,
}

/// One entry in a session's trace: a real user prompt, an assistant reply, or one tool call
/// (request + its result, once seen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceStep {
    pub kind: TraceKind,
    /// Short line for the step list: a truncated prompt/reply, or `toolName(primary argument)`.
    pub label: String,
    /// Full text shown once the step is selected: the prompt/reply, or the tool's name, input and result.
    pub detail: String,
    /// Set once a matching `tool_result` block arrives marked `is_error`.
    pub is_error: bool,
}

const LABEL_MAX: usize = 72;

fn label_line(text: &str) -> String {
    let line = one_line(text);
    if line.chars().count() > LABEL_MAX {
        let mut out: String = line.chars().take(LABEL_MAX - 1).collect();
        out.push('…');
        out
    } else {
        line
    }
}

/// The tool's one argument most worth showing next to its name, e.g. `Read(src/app.ts)`.
fn primary_arg(input: &Value) -> Option<&str> {
    let obj = input.as_object()?;
    // `??` only skips null/undefined, so a present non-string value ends the search with no arg.
    let value = [
        "file_path",
        "path",
        "command",
        "pattern",
        "query",
        "url",
        "description",
    ]
    .iter()
    .find_map(|k| obj.get(*k).filter(|v| !v.is_null()))?;
    value.as_str()
}

fn tool_label(name: &str, input: &Value) -> String {
    match primary_arg(input) {
        Some(arg) if !arg.is_empty() => format!("{name}({})", label_line(arg)),
        _ => name.to_string(),
    }
}

fn step(kind: TraceKind, text: &str) -> TraceStep {
    TraceStep {
        kind,
        label: label_line(text),
        detail: text.to_string(),
        is_error: false,
    }
}

/// Parses one Claude transcript's raw `.jsonl` lines into an ordered [`TraceStep`] list. A tool
/// call is paired with its result (matched by `tool_use_id`) into one step. Pure, so it is
/// unit-tested without the filesystem; [`read_session_trace`] is the file-reading wrapper.
pub fn build_trace_from_lines<S: AsRef<str>>(lines: &[S]) -> Vec<TraceStep> {
    let mut steps: Vec<TraceStep> = Vec::new();
    let mut tool_steps: HashMap<String, usize> = HashMap::new();

    for line in lines {
        let trimmed = line.as_ref().trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        let Some(message) = record.get("message") else {
            continue;
        };
        let role = message.get("role").and_then(Value::as_str);
        let content = message.get("content").unwrap_or(&Value::Null);

        match (record.get("type").and_then(Value::as_str), role) {
            (Some("user"), Some("user")) => {
                for block in content.as_array().into_iter().flatten() {
                    if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let Some(&index) = block
                        .get("tool_use_id")
                        .and_then(Value::as_str)
                        .and_then(|id| tool_steps.get(id))
                    else {
                        continue;
                    };
                    let result = extract_text(block.get("content").unwrap_or(&Value::Null));
                    let result = result.trim();
                    let result = if result.is_empty() { "(empty)" } else { result };
                    let failed = block.get("is_error").is_some_and(is_truthy);
                    let target = &mut steps[index];
                    if failed {
                        target.is_error = true;
                    }
                    target.detail.push_str(&format!(
                        "\n\n{}\n{result}",
                        if failed { "Error:" } else { "Result:" }
                    ));
                }
                let text = extract_text(content);
                let text = text.trim();
                if !text.is_empty() && is_displayable_user_prompt(text) {
                    steps.push(step(TraceKind::User, text));
                }
            }
            (Some("assistant"), Some("assistant")) => {
                let text = extract_assistant_display_text(content);
                let text = text.trim();
                if !text.is_empty() {
                    steps.push(step(TraceKind::Assistant, text));
                }
                for block in content.as_array().into_iter().flatten() {
                    if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("tool");
                    let input = block.get("input").unwrap_or(&Value::Null);
                    let pretty = match block.get("input") {
                        Some(v) => serde_json::to_string_pretty(v).unwrap_or_default(),
                        None => "undefined".to_string(),
                    };
                    steps.push(TraceStep {
                        kind: TraceKind::Tool,
                        label: tool_label(name, input),
                        detail: format!("{name}\n{pretty}"),
                        is_error: false,
                    });
                    if let Some(id) = block.get("id").and_then(Value::as_str) {
                        tool_steps.insert(id.to_string(), steps.len() - 1);
                    }
                }
            }
            _ => {}
        }
    }
    steps
}

/// JavaScript truthiness for the `is_error` flag.
fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// Reads and parses a whole Claude transcript file into its trace.
pub fn read_session_trace(path: &Path) -> std::io::Result<Vec<TraceStep>> {
    let bytes = fs::read(path)?;
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    Ok(build_trace_from_lines(&lines))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user_line(content: Value) -> String {
        json!({"type": "user", "message": {"role": "user", "content": content}}).to_string()
    }

    fn assistant_line(content: Value) -> String {
        json!({"type": "assistant", "message": {"role": "assistant", "content": content}}).to_string()
    }

    fn read_tool_use() -> Value {
        json!([{"type": "tool_use", "id": "tool-1", "name": "Read", "input": {"file_path": "src/app.ts"}}])
    }

    #[test]
    fn reads_a_plain_user_prompt_and_assistant_reply() {
        let steps = build_trace_from_lines(&[
            user_line(json!("fix the bug")),
            assistant_line(json!([{"type": "text", "text": "Fixed it."}])),
        ]);
        assert_eq!(
            steps,
            [
                TraceStep {
                    kind: TraceKind::User,
                    label: "fix the bug".into(),
                    detail: "fix the bug".into(),
                    is_error: false
                },
                TraceStep {
                    kind: TraceKind::Assistant,
                    label: "Fixed it.".into(),
                    detail: "Fixed it.".into(),
                    is_error: false
                },
            ]
        );
    }

    #[test]
    fn filters_out_a_hidden_slash_command_echo() {
        assert!(
            build_trace_from_lines(&[user_line(json!("<command-name>/clear</command-name>"))]).is_empty()
        );
    }

    #[test]
    fn tool_use_becomes_its_own_step_labeled_with_its_primary_argument() {
        let steps = build_trace_from_lines(&[assistant_line(read_tool_use())]);
        assert_eq!(
            steps,
            [TraceStep {
                kind: TraceKind::Tool,
                label: "Read(src/app.ts)".into(),
                detail: "Read\n{\n  \"file_path\": \"src/app.ts\"\n}".into(),
                is_error: false,
            }]
        );
    }

    #[test]
    fn attaches_a_matching_tool_result_to_its_tool_step() {
        let steps = build_trace_from_lines(&[
            assistant_line(read_tool_use()),
            user_line(
                json!([{"type": "tool_result", "tool_use_id": "tool-1", "content": "file contents here"}]),
            ),
        ]);
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].kind, TraceKind::Tool);
        assert!(steps[0].detail.ends_with("Result:\nfile contents here"));
        assert!(!steps[0].is_error);
    }

    #[test]
    fn marks_an_error_result_and_does_not_surface_it_as_a_user_step() {
        let steps = build_trace_from_lines(&[
            assistant_line(
                json!([{"type": "tool_use", "id": "tool-1", "name": "Bash", "input": {"command": "false"}}]),
            ),
            user_line(
                json!([{"type": "tool_result", "tool_use_id": "tool-1", "content": "command failed", "is_error": true}]),
            ),
        ]);
        assert_eq!(steps.len(), 1);
        assert!(steps[0].is_error);
        assert!(steps[0].detail.ends_with("Error:\ncommand failed"));
    }

    #[test]
    fn ignores_an_unmatched_tool_result_and_other_record_types() {
        let steps = build_trace_from_lines(&[
            user_line(json!([{"type": "tool_result", "tool_use_id": "missing", "content": "orphaned"}])),
            json!({"type": "custom-title", "customTitle": "Renamed"}).to_string(),
        ]);
        assert!(steps.is_empty());
    }

    #[test]
    fn tolerates_blank_and_malformed_lines() {
        assert!(build_trace_from_lines(&["", "   ", "{not json"]).is_empty());
    }

    #[test]
    fn long_labels_are_truncated() {
        let steps = build_trace_from_lines(&[user_line(json!("x".repeat(100)))]);
        assert_eq!(steps[0].label.chars().count(), LABEL_MAX);
        assert!(steps[0].label.ends_with('…'));
        assert_eq!(steps[0].detail.len(), 100);
    }
}
