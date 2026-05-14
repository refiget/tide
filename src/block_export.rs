use std::time::UNIX_EPOCH;

use crate::{
    app::{BlockKind, BlockStatus, CommandBlock},
    format::compact_command,
};

pub const SCHEMA_VERSION: &str = "block_export.v1";
pub const KEY_SCHEMA_VERSION: &str = "schema_version";
pub const KEY_ID: &str = "id";
pub const KEY_KIND: &str = "kind";
pub const KEY_STATUS: &str = "status";
pub const KEY_OUTPUT_SEMANTICS: &str = "output_semantics";
pub const KEY_OUTPUT_TRUNCATED: &str = "output_truncated";
pub const KEY_CWD: &str = "cwd";
pub const KEY_STARTED_AT_MS: &str = "started_at_ms";
pub const KEY_FINISHED_AT_MS: &str = "finished_at_ms";
pub const KEY_DURATION_MS: &str = "duration_ms";
pub const KEY_EXIT_CODE: &str = "exit_code";
pub const KEY_OUTPUT_STORED_BYTES: &str = "output_stored_bytes";
pub const KEY_COMMAND: &str = "command";
pub const KEY_OUTPUT_TEXT: &str = "output_text";
pub const KEY_VIEWS: &str = "views";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPart {
    Command,
    Output,
    Both,
}

pub fn format_block_json(block: &CommandBlock, part: ExportPart) -> String {
    let mut fields: Vec<String> = Vec::new();
    fields.push(format!("\"{KEY_SCHEMA_VERSION}\":\"{SCHEMA_VERSION}\""));
    fields.push(format!("\"{KEY_ID}\":{}", block.id.0));
    fields.push(format!(
        "\"{KEY_KIND}\":{}",
        json_string(block.kind.as_str())
    ));
    fields.push(format!(
        "\"{KEY_STATUS}\":{}",
        json_string(block.status.as_str())
    ));
    fields.push(format!(
        "\"{KEY_OUTPUT_SEMANTICS}\":{}",
        json_string(output_semantics(block.kind.clone()))
    ));
    fields.push(format!(
        "\"{KEY_OUTPUT_TRUNCATED}\":{}",
        if block.output_truncated {
            "true"
        } else {
            "false"
        }
    ));
    fields.push(format!(
        "\"{KEY_CWD}\":{}",
        json_string(&block.cwd.display().to_string())
    ));
    fields.push(format!(
        "\"{KEY_STARTED_AT_MS}\":{}",
        system_time_ms(block.started_at)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));
    fields.push(format!(
        "\"{KEY_FINISHED_AT_MS}\":{}",
        block
            .finished_at
            .and_then(system_time_ms)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));
    fields.push(format!(
        "\"{KEY_DURATION_MS}\":{}",
        block
            .duration_ms
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));
    fields.push(format!(
        "\"{KEY_EXIT_CODE}\":{}",
        block
            .exit_code
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));
    fields.push(format!(
        "\"{KEY_OUTPUT_STORED_BYTES}\":{}",
        block.output_raw.len()
    ));

    match part {
        ExportPart::Command => {
            fields.push(format!("\"{KEY_COMMAND}\":{}", json_string(&block.command)));
        }
        ExportPart::Output => {
            fields.push(format!(
                "\"{KEY_OUTPUT_TEXT}\":{}",
                json_string(&block.output_text)
            ));
        }
        ExportPart::Both => {
            fields.push(format!("\"{KEY_COMMAND}\":{}", json_string(&block.command)));
            fields.push(format!(
                "\"{KEY_OUTPUT_TEXT}\":{}",
                json_string(&block.output_text)
            ));
            fields.push(format!("\"{KEY_VIEWS}\":{}", build_views_json(block)));
        }
    }

    format!("{{{}}}", fields.join(","))
}

pub fn format_blocks_json(blocks: &[&CommandBlock], part: ExportPart) -> String {
    let entries: Vec<String> = blocks.iter().map(|b| format_block_json(b, part)).collect();
    match entries.len() {
        0 => String::new(),
        1 => entries.into_iter().next().unwrap_or_default(),
        _ => format!("[{}]", entries.join(",")),
    }
}

fn build_views_json(block: &CommandBlock) -> String {
    let summary = build_summary_json(block);
    let error = build_error_json(block).unwrap_or_else(|| "null".to_string());
    let audit = build_audit_json(block);
    let context = build_context_json(block);
    format!(
        "{{\"summary\":{},\"error\":{},\"audit\":{},\"context\":{}}}",
        summary, error, audit, context
    )
}

fn build_summary_json(block: &CommandBlock) -> String {
    let headline = if block.command.is_empty() {
        "(empty command)".to_string()
    } else {
        compact_command(&block.command, 80)
    };
    format!(
        "{{\"headline\":{},\"status\":{},\"duration_ms\":{},\"exit_code\":{},\"truncated\":{}}}",
        json_string(&headline),
        json_string(block.status.as_str()),
        block
            .duration_ms
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string()),
        block
            .exit_code
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string()),
        if block.output_truncated {
            "true"
        } else {
            "false"
        }
    )
}

fn build_error_json(block: &CommandBlock) -> Option<String> {
    if !matches!(block.status, BlockStatus::Failed | BlockStatus::Interrupted) {
        return None;
    }
    let tail = output_tail(&block.output_text, 8);
    Some(format!(
        "{{\"status\":{},\"exit_code\":{},\"tail\":{}}}",
        json_string(block.status.as_str()),
        block
            .exit_code
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string()),
        json_string(&tail)
    ))
}

fn build_audit_json(block: &CommandBlock) -> String {
    let mut items: Vec<String> = Vec::new();
    if block.output_truncated {
        items.push(json_string("output_truncated"));
    }
    if matches!(block.status, BlockStatus::Failed) {
        items.push(json_string("command_failed"));
    }
    if matches!(block.status, BlockStatus::Interrupted) {
        items.push(json_string("command_interrupted"));
    }
    if matches!(block.kind, BlockKind::RawProgram) {
        items.push(json_string("raw_program_output_non_linear"));
    }
    if matches!(block.kind, BlockKind::TuiSession) {
        items.push(json_string("tui_session"));
    }
    if matches!(block.kind, BlockKind::Interactive) {
        items.push(json_string("interactive_repl"));
    }
    format!("[{}]", items.join(","))
}

fn build_context_json(block: &CommandBlock) -> String {
    let excerpt = if matches!(
        block.kind,
        BlockKind::RawProgram | BlockKind::TuiSession | BlockKind::Interactive
    ) {
        String::new()
    } else {
        output_tail(&block.output_text, 4)
    };
    format!(
        "{{\"command\":{},\"cwd\":{},\"status\":{},\"output_excerpt\":{}}}",
        json_string(&block.command),
        json_string(&block.cwd.display().to_string()),
        json_string(block.status.as_str()),
        json_string(&excerpt)
    )
}

fn output_tail(output: &str, lines: usize) -> String {
    if lines == 0 || output.is_empty() {
        return String::new();
    }
    let all: Vec<&str> = output.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

fn output_semantics(kind: BlockKind) -> &'static str {
    match kind {
        BlockKind::RawProgram | BlockKind::TuiSession => "non_linear_tui",
        BlockKind::Interactive => "interactive_repl",
        _ => "line_oriented",
    }
}

fn system_time_ms(t: std::time::SystemTime) -> Option<u128> {
    t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_millis())
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, time::UNIX_EPOCH};

    use super::*;
    use crate::app::{BlockId, CommandBlock};

    fn fixed_block() -> CommandBlock {
        CommandBlock {
            id: BlockId(7),
            command: "cargo test".to_string(),
            cwd: PathBuf::from("/repo"),
            started_at: UNIX_EPOCH + std::time::Duration::from_millis(1000),
            finished_at: Some(UNIX_EPOCH + std::time::Duration::from_millis(2500)),
            duration_ms: Some(1500),
            exit_code: Some(1),
            output_raw: b"line1\nline2\n".to_vec(),
            output_text: "line1\nline2\n".to_string(),
            kind: BlockKind::NormalCommand,
            status: BlockStatus::Failed,
            output_truncated: true,
            ..CommandBlock::default()
        }
    }

    fn assert_has_key(json: &str, key: &str) {
        assert!(
            json.contains(&format!("\"{key}\":")),
            "expected key `{key}` in: {json}"
        );
    }

    #[test]
    fn schema_constants_are_used_in_output() {
        let out = format_block_json(&fixed_block(), ExportPart::Both);
        assert!(out.contains(&format!("\"{KEY_SCHEMA_VERSION}\":\"{SCHEMA_VERSION}\"")));
        assert!(out.contains(&format!("\"{KEY_ID}\":7")));
        assert!(out.contains(&format!("\"{KEY_COMMAND}\":\"cargo test\"")));
    }

    #[test]
    fn snapshot_export_both_is_stable() {
        let out = format_block_json(&fixed_block(), ExportPart::Both);
        let expected = r#"{"schema_version":"block_export.v1","id":7,"kind":"normal_command","status":"failed","output_semantics":"line_oriented","output_truncated":true,"cwd":"/repo","started_at_ms":1000,"finished_at_ms":2500,"duration_ms":1500,"exit_code":1,"output_stored_bytes":12,"command":"cargo test","output_text":"line1\nline2\n","views":{"summary":{"headline":"cargo test","status":"failed","duration_ms":1500,"exit_code":1,"truncated":true},"error":{"status":"failed","exit_code":1,"tail":"line1\nline2"},"audit":["output_truncated","command_failed"],"context":{"command":"cargo test","cwd":"/repo","status":"failed","output_excerpt":"line1\nline2"}}}"#;
        assert_eq!(out, expected);
    }

    #[test]
    fn export_part_command_has_expected_shape() {
        let out = format_block_json(&fixed_block(), ExportPart::Command);
        for key in [
            KEY_SCHEMA_VERSION,
            KEY_ID,
            KEY_KIND,
            KEY_STATUS,
            KEY_OUTPUT_SEMANTICS,
            KEY_OUTPUT_TRUNCATED,
            KEY_CWD,
            KEY_STARTED_AT_MS,
            KEY_FINISHED_AT_MS,
            KEY_DURATION_MS,
            KEY_EXIT_CODE,
            KEY_OUTPUT_STORED_BYTES,
            KEY_COMMAND,
        ] {
            assert_has_key(&out, key);
        }
        assert!(!out.contains(&format!("\"{KEY_OUTPUT_TEXT}\":")));
        assert!(!out.contains(&format!("\"{KEY_VIEWS}\":")));
    }

    #[test]
    fn export_part_output_has_expected_shape() {
        let out = format_block_json(&fixed_block(), ExportPart::Output);
        for key in [
            KEY_SCHEMA_VERSION,
            KEY_ID,
            KEY_KIND,
            KEY_STATUS,
            KEY_OUTPUT_SEMANTICS,
            KEY_OUTPUT_TRUNCATED,
            KEY_CWD,
            KEY_STARTED_AT_MS,
            KEY_FINISHED_AT_MS,
            KEY_DURATION_MS,
            KEY_EXIT_CODE,
            KEY_OUTPUT_STORED_BYTES,
            KEY_OUTPUT_TEXT,
        ] {
            assert_has_key(&out, key);
        }
        assert!(!out.contains(&format!("\"{KEY_COMMAND}\":")));
        assert!(!out.contains(&format!("\"{KEY_VIEWS}\":")));
    }

    #[test]
    fn export_part_both_has_expected_shape_and_views() {
        let out = format_block_json(&fixed_block(), ExportPart::Both);
        for key in [
            KEY_SCHEMA_VERSION,
            KEY_ID,
            KEY_KIND,
            KEY_STATUS,
            KEY_OUTPUT_SEMANTICS,
            KEY_OUTPUT_TRUNCATED,
            KEY_CWD,
            KEY_STARTED_AT_MS,
            KEY_FINISHED_AT_MS,
            KEY_DURATION_MS,
            KEY_EXIT_CODE,
            KEY_OUTPUT_STORED_BYTES,
            KEY_COMMAND,
            KEY_OUTPUT_TEXT,
            KEY_VIEWS,
        ] {
            assert_has_key(&out, key);
        }
        for view_key in ["summary", "error", "audit", "context"] {
            assert_has_key(&out, view_key);
        }
    }
}
