use std::time::UNIX_EPOCH;

use crate::{
    app::{BlockKind, BlockStatus, CommandBlock},
    format::compact_command,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportPart {
    Command,
    Output,
    Both,
}

pub fn format_block_json(block: &CommandBlock, part: ExportPart) -> String {
    let mut fields: Vec<String> = Vec::new();
    fields.push(format!("\"schema_version\":\"{}\"", "block_export.v1"));
    fields.push(format!("\"id\":{}", block.id.0));
    fields.push(format!("\"kind\":{}", json_string(block.kind.as_str())));
    fields.push(format!("\"status\":{}", json_string(block.status.as_str())));
    fields.push(format!(
        "\"output_semantics\":{}",
        json_string(output_semantics(block.kind.clone()))
    ));
    fields.push(format!(
        "\"output_truncated\":{}",
        if block.output_truncated { "true" } else { "false" }
    ));
    fields.push(format!("\"cwd\":{}", json_string(&block.cwd.display().to_string())));
    fields.push(format!(
        "\"started_at_ms\":{}",
        system_time_ms(block.started_at)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));
    fields.push(format!(
        "\"finished_at_ms\":{}",
        block
            .finished_at
            .and_then(system_time_ms)
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));
    fields.push(format!(
        "\"duration_ms\":{}",
        block
            .duration_ms
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));
    fields.push(format!(
        "\"exit_code\":{}",
        block
            .exit_code
            .map(|v| v.to_string())
            .unwrap_or_else(|| "null".to_string())
    ));
    fields.push(format!("\"output_stored_bytes\":{}", block.output_raw.len()));

    match part {
        ExportPart::Command => {
            fields.push(format!("\"command\":{}", json_string(&block.command)));
        }
        ExportPart::Output => {
            fields.push(format!("\"output_text\":{}", json_string(&block.output_text)));
        }
        ExportPart::Both => {
            fields.push(format!("\"command\":{}", json_string(&block.command)));
            fields.push(format!("\"output_text\":{}", json_string(&block.output_text)));
            fields.push(format!("\"views\":{}", build_views_json(block)));
        }
    }

    format!("{{{}}}", fields.join(","))
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
        if block.output_truncated { "true" } else { "false" }
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
    format!("[{}]", items.join(","))
}

fn build_context_json(block: &CommandBlock) -> String {
    let excerpt = if matches!(block.kind, BlockKind::RawProgram | BlockKind::TuiSession) {
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
    if matches!(kind, BlockKind::RawProgram | BlockKind::TuiSession) {
        "non_linear_tui"
    } else {
        "line_oriented"
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
