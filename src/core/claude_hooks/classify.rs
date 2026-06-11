/// Stop-event status classification.
///
/// Reads the last assistant turn from a Claude transcript JSONL file and
/// returns `Waiting` when the turn contains an `AskUserQuestion` tool use,
/// ends with a `?`, or contains `needs input:` — `Done` otherwise.
/// Falls back to `Done` on any parse error so the tab-flash always fires.
use std::path::Path;

/// Whether the Claude agent is blocked waiting for user input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopStatus {
    Done,
    Waiting,
}

impl StopStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StopStatus::Done => "done",
            StopStatus::Waiting => "waiting",
        }
    }
}

/// Classify a Stop event by inspecting the last assistant turn in `transcript`.
/// Returns `Done` when the transcript is absent, empty, or unparseable.
pub fn classify_stop(transcript: &Path) -> StopStatus {
    let text = match std::fs::read_to_string(transcript) {
        Ok(t) => t,
        Err(_) => return StopStatus::Done,
    };

    // Find the last line with `"role":"assistant"`.
    let last_asst = text
        .lines()
        .rfind(|line| line.contains("\"role\"") && line.contains("\"assistant\""));

    let Some(line) = last_asst else {
        return StopStatus::Done;
    };

    let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
        return StopStatus::Done;
    };

    // Extract the content array from `.message.content` (new format) or `.content` (old).
    let content = val
        .pointer("/message/content")
        .or_else(|| val.pointer("/content"))
        .and_then(|v| v.as_array());

    let Some(items) = content else {
        return StopStatus::Done;
    };

    // AskUserQuestion tool_use → always waiting.
    let has_ask = items.iter().any(|item| {
        item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
            && item.get("name").and_then(|n| n.as_str()) == Some("AskUserQuestion")
    });
    if has_ask {
        return StopStatus::Waiting;
    }

    // Collect all text blocks into one string.
    let full_text: String = items
        .iter()
        .filter(|item| item.get("type").and_then(|t| t.as_str()) == Some("text"))
        .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
        .collect::<Vec<_>>()
        .join("");

    if full_text.is_empty() {
        return StopStatus::Done;
    }

    let trimmed = full_text.trim_end();
    if trimmed.ends_with('?') || full_text.to_ascii_lowercase().contains("needs input:") {
        StopStatus::Waiting
    } else {
        StopStatus::Done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_transcript(tmp: &tempfile::TempDir, content: &str) -> std::path::PathBuf {
        let path = tmp.path().join("transcript.jsonl");
        std::fs::write(&path, content).unwrap();
        path
    }

    fn asst_line(content_json: &str) -> String {
        // Uses `.message.content` format (current Claude format, per commit 676e2b6)
        format!(r#"{{"role":"assistant","message":{{"content":{content_json}}}}}"#)
    }

    fn asst_line_old(content_json: &str) -> String {
        // Legacy `.content` format
        format!(r#"{{"role":"assistant","content":{content_json}}}"#)
    }

    #[test]
    fn done_on_plain_statement() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line(r#"[{"type":"text","text":"I have finished the task."}]"#),
        );
        assert_eq!(classify_stop(&p), StopStatus::Done);
    }

    #[test]
    fn waiting_on_trailing_question_mark() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line(r#"[{"type":"text","text":"Should I continue?"}]"#),
        );
        assert_eq!(classify_stop(&p), StopStatus::Waiting);
    }

    #[test]
    fn waiting_on_needs_input() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line(r#"[{"type":"text","text":"needs input: please provide credentials"}]"#),
        );
        assert_eq!(classify_stop(&p), StopStatus::Waiting);
    }

    #[test]
    fn waiting_on_ask_user_question_tool() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line(r#"[{"type":"tool_use","name":"AskUserQuestion","id":"x","input":{}}]"#),
        );
        assert_eq!(classify_stop(&p), StopStatus::Waiting);
    }

    #[test]
    fn done_on_missing_transcript() {
        let p = std::path::Path::new("/tmp/does-not-exist-meldr-test.jsonl");
        assert_eq!(classify_stop(p), StopStatus::Done);
    }

    #[test]
    fn legacy_content_field_still_works() {
        // Regression: commit 676e2b6 fixed .message.content vs .content — both must work.
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line_old(r#"[{"type":"text","text":"Are you sure?"}]"#),
        );
        assert_eq!(classify_stop(&p), StopStatus::Waiting);
    }

    #[test]
    fn last_assistant_turn_used() {
        // Two assistant turns; only the last one matters.
        let tmp = tempfile::TempDir::new().unwrap();
        let first = asst_line(r#"[{"type":"text","text":"First turn, done."}]"#);
        let second = asst_line(r#"[{"type":"text","text":"Second turn, what do you want?"}]"#);
        let p = write_transcript(&tmp, &format!("{first}\n{second}\n"));
        assert_eq!(classify_stop(&p), StopStatus::Waiting);
    }

    #[test]
    fn done_when_no_assistant_turn() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            r#"{"role":"user","message":{"content":[{"type":"text","text":"hi"}]}}"#,
        );
        assert_eq!(classify_stop(&p), StopStatus::Done);
    }
}
