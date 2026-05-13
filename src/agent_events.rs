use std::{
    io::{Read, Seek, SeekFrom},
    path::Path,
    time::UNIX_EPOCH,
};

use serde::Deserialize;

use crate::app::{AgentLiveSnapshot, AgentLiveStatus};

const TAIL_BYTES: u64 = 65536;
const MAX_LINES: usize = 200;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Read the live status snapshot for an agent.
/// Hot path: reads events.jsonl (tail 64 KB) + session.json only.
/// history.json is NOT read here — it is not needed for Block View rendering.
pub fn read_agent_live_snapshot(agents_dir: &Path, pane_id: &str) -> Option<AgentLiveSnapshot> {
    if pane_id.is_empty() {
        return None;
    }
    let dir = agents_dir.join(pane_id);
    let status_snap = read_status_from_events(&dir);
    let title = read_session_title(&dir);

    if status_snap.is_none() && title.is_none() {
        return None;
    }

    let (status, at_ms, current_tool, current_command) = status_snap.unwrap_or_default();
    Some(AgentLiveSnapshot {
        status,
        at_ms,
        current_tool,
        current_command,
        title,
    })
}

/// Returns the most-recent mtime (milliseconds) across events.jsonl and session.json.
/// history.json is intentionally excluded — it is not part of the Block View hot path.
pub fn agent_events_mtime(agents_dir: &Path, pane_id: &str) -> Option<u64> {
    let dir = agents_dir.join(pane_id);
    [
        file_mtime_ms(&dir.join("events.jsonl")),
        file_mtime_ms(&dir.join("session.json")),
    ]
    .into_iter()
    .flatten()
    .max()
}

// ─── File helpers ─────────────────────────────────────────────────────────────

fn file_mtime_ms(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
}

// ─── Status from events.jsonl ─────────────────────────────────────────────────

type StatusTuple = (AgentLiveStatus, Option<u64>, Option<String>, Option<String>);

/// Some event types carry no live-status meaning and must be skipped during
/// the backwards status scan so they don't clobber the real running state.
///
/// - `session`     — metadata update only; does not change what the model is doing.
/// - `tool_result` — outcome of a tool that already fired; status stays at the
///                   previous tool_call or whatever follows.
fn carries_status(event_type: &str) -> bool {
    !matches!(event_type, "session" | "tool_result")
}

fn read_status_from_events(dir: &Path) -> Option<StatusTuple> {
    let path = dir.join("events.jsonl");
    let mut file = std::fs::File::open(&path).ok()?;

    let file_len = file.metadata().ok()?.len();
    let start = file_len.saturating_sub(TAIL_BYTES);
    if start > 0 {
        file.seek(SeekFrom::Start(start)).ok()?;
    }

    let mut buf = String::new();
    file.read_to_string(&mut buf).ok()?;

    let lines: Vec<&str> = buf.lines().collect();
    let tail: &[&str] = if lines.len() > MAX_LINES {
        &lines[lines.len() - MAX_LINES..]
    } else {
        &lines
    };

    let mut status = AgentLiveStatus::Unknown;
    let mut at_ms: Option<u64> = None;
    let mut current_tool: Option<String> = None;
    let mut current_command: Option<String> = None;
    let mut status_found = false;
    let mut tool_found = false;

    for line in tail.iter().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Skip partial lines that may result from a write-in-progress at EOF.
        let Ok(val) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        let event_type = val.get("type").and_then(|v| v.as_str()).unwrap_or("");

        // Status: first status-bearing event wins (scanning newest→oldest).
        if !status_found && carries_status(event_type) {
            at_ms = val.get("at_ms").and_then(|v| v.as_u64());
            status = event_to_status(&val, event_type);
            status_found = true;
        }

        // Tool info: from the most recent tool_call (independent of status scan).
        if event_type == "tool_call" && !tool_found {
            current_tool = val
                .get("tool_name")
                .and_then(|v| v.as_str())
                .map(String::from);
            if is_exec_tool(current_tool.as_deref().unwrap_or("")) {
                current_command = val
                    .get("command")
                    .and_then(|v| v.as_str())
                    .map(String::from);
            }
            tool_found = true;
        }

        if status_found && tool_found {
            break;
        }
    }

    if !status_found {
        return None;
    }

    Some((status, at_ms, current_tool, current_command))
}

fn event_to_status(val: &serde_json::Value, event_type: &str) -> AgentLiveStatus {
    match event_type {
        "thinking" => AgentLiveStatus::Thinking,
        "tool_call" => {
            let tool = val.get("tool_name").and_then(|v| v.as_str()).unwrap_or("");
            if is_exec_tool(tool) {
                AgentLiveStatus::ExecutingCommand
            } else {
                AgentLiveStatus::ToolCall
            }
        }
        "reply" | "replying" => AgentLiveStatus::Replying,
        "question" => AgentLiveStatus::Question,
        // permission_request is the canonical name; legacy "request" kept for compat.
        "permission_request" | "request" => AgentLiveStatus::Request,
        // user_message / prompt: user just sent input, model hasn't started yet.
        "user_message" | "prompt" => AgentLiveStatus::Idle,
        "idle" | "started" => AgentLiveStatus::Idle,
        "exit" => AgentLiveStatus::Exited,
        "error" => AgentLiveStatus::Error,
        _ => AgentLiveStatus::Unknown,
    }
}

fn is_exec_tool(name: &str) -> bool {
    matches!(name, "bash" | "shell" | "execute" | "run" | "exec")
}

// ─── Session title from session.json ─────────────────────────────────────────

#[derive(Deserialize)]
struct SessionFile {
    title: Option<String>,
}

fn read_session_title(dir: &Path) -> Option<String> {
    let path = dir.join("session.json");
    let Ok(data) = std::fs::read_to_string(&path) else {
        return None;
    };
    let Ok(parsed) = serde_json::from_str::<SessionFile>(&data) else {
        return None;
    };
    parsed.title.filter(|t| !t.trim().is_empty())
}

// ─── History from history.json (not on Block View hot path) ──────────────────
// Kept for future use (e.g. Detail View). Not called by read_agent_live_snapshot.

#[allow(dead_code)]
#[derive(Deserialize)]
struct HistoryFile {
    records: Vec<HistoryRecordJson>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct HistoryRecordJson {
    at_ms: u64,
    user_message: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ToolCallJson>,
    reply_summary: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ToolCallJson {
    tool_name: String,
    command: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_file(dir: &std::path::Path, name: &str, content: &str) {
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), content).unwrap();
    }

    #[test]
    fn reads_status_and_title_from_plugin_output() {
        let base = std::env::temp_dir().join(format!("tide-events-test-{}", std::process::id()));
        let pane = "%TEST99";
        let pane_dir = base.join(pane);

        write_file(
            &pane_dir,
            "events.jsonl",
            concat!(
                "{\"at_ms\":1000,\"type\":\"session\",\"title\":\"Test Session Title\"}\n",
                "{\"at_ms\":1001,\"type\":\"started\",\"cwd\":\"/test/repo\"}\n",
                "{\"at_ms\":1002,\"type\":\"user_message\",\"summary\":\"fix the bug\"}\n",
                "{\"at_ms\":1003,\"type\":\"reply\",\"tokens_out\":42}\n",
                "{\"at_ms\":1004,\"type\":\"tool_call\",\"tool_name\":\"bash\",\"command\":\"cargo test\"}\n",
            ),
        );
        write_file(
            &pane_dir,
            "session.json",
            r#"{"version":1,"updated_at_ms":1000,"provider":"opencode","title":"Test Session Title","session_id":"sess-001"}"#,
        );

        let snap = read_agent_live_snapshot(&base, pane).expect("should have snapshot");

        // Latest status event is tool_call(bash) → ExecutingCommand
        assert_eq!(snap.status, AgentLiveStatus::ExecutingCommand);
        assert_eq!(snap.current_tool.as_deref(), Some("bash"));
        assert_eq!(snap.current_command.as_deref(), Some("cargo test"));
        assert_eq!(snap.title.as_deref(), Some("Test Session Title"));

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn session_event_does_not_override_status() {
        let base = std::env::temp_dir().join(format!("tide-events-test2-{}", std::process::id()));
        let pane = "%TEST100";
        let pane_dir = base.join(pane);

        // tool_call happened, then session metadata update — status should stay ExecutingCommand
        write_file(
            &pane_dir,
            "events.jsonl",
            concat!(
                "{\"at_ms\":1000,\"type\":\"tool_call\",\"tool_name\":\"bash\",\"command\":\"cargo build\"}\n",
                "{\"at_ms\":1001,\"type\":\"session\",\"title\":\"Updated Title\"}\n",
            ),
        );
        write_file(
            &pane_dir,
            "session.json",
            r#"{"version":1,"updated_at_ms":1001,"provider":"opencode","title":"Updated Title"}"#,
        );

        let snap = read_agent_live_snapshot(&base, pane).expect("should have snapshot");

        // session event is skipped; tool_call is the effective status
        assert_eq!(snap.status, AgentLiveStatus::ExecutingCommand);
        assert_eq!(snap.title.as_deref(), Some("Updated Title"));

        fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn permission_request_maps_to_request_status() {
        let base = std::env::temp_dir().join(format!("tide-events-test3-{}", std::process::id()));
        let pane = "%TEST101";
        let pane_dir = base.join(pane);

        write_file(
            &pane_dir,
            "events.jsonl",
            "{\"at_ms\":1000,\"type\":\"permission_request\",\"text\":\"Allow bash?\"}\n",
        );

        let snap = read_agent_live_snapshot(&base, pane).expect("should have snapshot");
        assert_eq!(snap.status, AgentLiveStatus::Request);

        fs::remove_dir_all(&base).ok();
    }
}
