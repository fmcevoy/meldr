//! Stop-event status classification: is the agent finished, or waiting on you?
//!
//! `Waiting` when the turn used `AskUserQuestion`, ends in a `?`, or says
//! `needs input:`; `Done` otherwise. Anything unreadable falls back to `Done` so a
//! parse problem still produces a notification rather than silence.
use std::path::Path;

/// Whether the Claude agent is blocked waiting for user input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopStatus {
    Done,
    Waiting,
}

/// Classify a Stop event.
///
/// `message` is the payload's `last_assistant_message`, preferred for the text
/// heuristics because the transcript file is not always flushed by the time the
/// hook runs. The transcript is still consulted for the `AskUserQuestion` tool-use
/// signal, which the plain text cannot express.
pub fn classify_stop(message: Option<&str>, transcript: Option<&Path>) -> StopStatus {
    if transcript.is_some_and(transcript_has_ask_user_question) {
        return StopStatus::Waiting;
    }
    if let Some(msg) = message.filter(|m| !m.trim().is_empty()) {
        return classify_text(msg);
    }
    let Some(transcript) = transcript else {
        return StopStatus::Done;
    };
    classify_transcript(transcript)
}

/// True when the last assistant turn used `AskUserQuestion`.
fn transcript_has_ask_user_question(transcript: &Path) -> bool {
    let Some(items) = last_assistant_content(transcript) else {
        return false;
    };
    items.iter().any(|item| {
        item.get("type").and_then(|t| t.as_str()) == Some("tool_use")
            && item.get("name").and_then(|n| n.as_str()) == Some("AskUserQuestion")
    })
}

/// Text-only heuristics: a question or an explicit request for input.
fn classify_text(text: &str) -> StopStatus {
    let trimmed = text.trim_end();
    if trimmed.ends_with('?') || text.to_ascii_lowercase().contains("needs input:") {
        StopStatus::Waiting
    } else {
        StopStatus::Done
    }
}

/// Content blocks of the last assistant turn, from either transcript format.
fn last_assistant_content(transcript: &Path) -> Option<Vec<serde_json::Value>> {
    let text = std::fs::read_to_string(transcript).ok()?;
    let line = text
        .lines()
        .rfind(|line| line.contains("\"role\"") && line.contains("\"assistant\""))?;
    let val: serde_json::Value = serde_json::from_str(line).ok()?;
    val.pointer("/message/content")
        .or_else(|| val.pointer("/content"))
        .and_then(|v| v.as_array())
        .cloned()
}

fn classify_transcript(transcript: &Path) -> StopStatus {
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
        // `.message.content` — the current Claude transcript shape.
        format!(r#"{{"role":"assistant","message":{{"content":{content_json}}}}}"#)
    }

    fn asst_line_old(content_json: &str) -> String {
        format!(r#"{{"role":"assistant","content":{content_json}}}"#)
    }

    fn from_transcript(p: &std::path::Path) -> StopStatus {
        classify_stop(None, Some(p))
    }

    // ── payload text (preferred source) ───────────────────────────────────────

    #[test]
    fn message_plain_statement_is_done() {
        assert_eq!(
            classify_stop(Some("I have finished the task."), None),
            StopStatus::Done
        );
    }

    #[test]
    fn message_question_is_waiting() {
        assert_eq!(
            classify_stop(Some("Should I continue?"), None),
            StopStatus::Waiting
        );
    }

    #[test]
    fn message_trailing_whitespace_after_question_still_waiting() {
        assert_eq!(
            classify_stop(Some("Ready to proceed?\n\n"), None),
            StopStatus::Waiting
        );
    }

    #[test]
    fn message_needs_input_marker_is_waiting() {
        assert_eq!(
            classify_stop(Some("needs input: credentials please"), None),
            StopStatus::Waiting
        );
        assert_eq!(
            classify_stop(Some("NEEDS INPUT: shouting"), None),
            StopStatus::Waiting
        );
    }

    #[test]
    fn message_wins_over_a_stale_transcript() {
        // The transcript may lag the turn; the payload text is authoritative for
        // the text heuristics.
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line(r#"[{"type":"text","text":"older turn."}]"#),
        );
        assert_eq!(
            classify_stop(Some("But now: shall I go on?"), Some(&p)),
            StopStatus::Waiting
        );
    }

    #[test]
    fn blank_message_falls_back_to_the_transcript() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(&tmp, &asst_line(r#"[{"type":"text","text":"Well?"}]"#));
        assert_eq!(classify_stop(Some("   "), Some(&p)), StopStatus::Waiting);
    }

    // ── AskUserQuestion (transcript only) ─────────────────────────────────────

    #[test]
    fn ask_user_question_is_waiting_even_with_declarative_text() {
        // The tool use is the signal; the accompanying prose often reads as done.
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line(r#"[{"type":"tool_use","name":"AskUserQuestion","id":"x","input":{}}]"#),
        );
        assert_eq!(
            classify_stop(Some("Here are the options."), Some(&p)),
            StopStatus::Waiting
        );
    }

    #[test]
    fn ask_user_question_from_transcript_alone() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line(r#"[{"type":"tool_use","name":"AskUserQuestion","id":"x","input":{}}]"#),
        );
        assert_eq!(from_transcript(&p), StopStatus::Waiting);
    }

    #[test]
    fn other_tool_uses_do_not_mean_waiting() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line(r#"[{"type":"tool_use","name":"Bash","id":"x","input":{}}]"#),
        );
        assert_eq!(from_transcript(&p), StopStatus::Done);
    }

    // ── transcript fallback ───────────────────────────────────────────────────

    #[test]
    fn transcript_question_is_waiting() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(&tmp, &asst_line(r#"[{"type":"text","text":"Which one?"}]"#));
        assert_eq!(from_transcript(&p), StopStatus::Waiting);
    }

    #[test]
    fn legacy_content_field_still_works() {
        // Regression for the `.message.content` vs `.content` fix.
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line_old(r#"[{"type":"text","text":"Are you sure?"}]"#),
        );
        assert_eq!(from_transcript(&p), StopStatus::Waiting);
    }

    #[test]
    fn last_assistant_turn_wins() {
        let tmp = tempfile::TempDir::new().unwrap();
        let first = asst_line(r#"[{"type":"text","text":"First turn, done."}]"#);
        let second = asst_line(r#"[{"type":"text","text":"Second turn, what do you want?"}]"#);
        let p = write_transcript(&tmp, &format!("{first}\n{second}\n"));
        assert_eq!(from_transcript(&p), StopStatus::Waiting);
    }

    #[test]
    fn multiple_text_blocks_are_joined_before_judging() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            &asst_line(
                r#"[{"type":"text","text":"Almost there. "},{"type":"text","text":"Proceed?"}]"#,
            ),
        );
        assert_eq!(from_transcript(&p), StopStatus::Waiting);
    }

    // ── degenerate inputs all fall back to Done ───────────────────────────────

    #[test]
    fn no_inputs_at_all_is_done() {
        assert_eq!(classify_stop(None, None), StopStatus::Done);
    }

    #[test]
    fn missing_transcript_is_done() {
        let p = std::path::Path::new("/tmp/does-not-exist-meldr-test.jsonl");
        assert_eq!(from_transcript(p), StopStatus::Done);
    }

    #[test]
    fn unparseable_transcript_is_done() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(&tmp, "{\"role\":\"assistant\" broken json");
        assert_eq!(from_transcript(&p), StopStatus::Done);
    }

    #[test]
    fn transcript_without_an_assistant_turn_is_done() {
        let tmp = tempfile::TempDir::new().unwrap();
        let p = write_transcript(
            &tmp,
            r#"{"role":"user","message":{"content":[{"type":"text","text":"hi"}]}}"#,
        );
        assert_eq!(from_transcript(&p), StopStatus::Done);
    }
}
