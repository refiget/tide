use std::path::Path;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{BlockStatus, CommandBlock};

// ─── compact_command ─────────────────────────────────────────────────────────

/// Strip ANSI escapes, normalize whitespace, right-truncate with `…`.
pub fn compact_command(command: &str, max_width: usize) -> String {
    if max_width == 0 || command.is_empty() {
        return String::new();
    }

    let stripped_bytes = strip_ansi_escapes::strip(command.as_bytes());
    let stripped = String::from_utf8_lossy(&stripped_bytes);

    let normalized: String = stripped.split_whitespace().collect::<Vec<_>>().join(" ");

    if normalized.is_empty() {
        return String::new();
    }

    let w = UnicodeWidthStr::width(normalized.as_str());
    if w <= max_width {
        return normalized;
    }

    let mut result = String::new();
    let mut used = 0usize;
    for ch in normalized.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw + 1 > max_width {
            break;
        }
        result.push(ch);
        used += cw;
    }
    result.push('…');
    result
}

// ─── compact_cwd ─────────────────────────────────────────────────────────────

/// Substitute home, middle-compress long paths, right-truncate as last resort.
pub fn compact_cwd(path: &Path, home: Option<&Path>, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }

    let display: String = match home {
        Some(h) if path.starts_with(h) => match path.strip_prefix(h) {
            Ok(rest) if rest == Path::new("") => "~".to_string(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        },
        _ => path.display().to_string(),
    };

    if UnicodeWidthStr::width(display.as_str()) <= max_width {
        return display;
    }

    let (prefix, components): (&str, Vec<&str>) = if display.starts_with("~/") {
        (
            "~",
            display[2..].split('/').filter(|s| !s.is_empty()).collect(),
        )
    } else if display.starts_with('/') {
        (
            "/",
            display[1..].split('/').filter(|s| !s.is_empty()).collect(),
        )
    } else {
        let parts: Vec<&str> = display.split('/').filter(|s| !s.is_empty()).collect();
        if parts.is_empty() {
            return cwd_right_truncate(&display, max_width);
        }
        (parts[0], parts[1..].to_vec())
    };

    if components.len() < 2 {
        return cwd_right_truncate(&display, max_width);
    }

    for tail_count in [2usize, 1] {
        if components.len() <= tail_count {
            continue;
        }
        let tail = components[components.len() - tail_count..].join("/");
        let candidate = format!("{prefix}/…/{tail}");
        if UnicodeWidthStr::width(candidate.as_str()) <= max_width {
            return candidate;
        }
    }

    cwd_right_truncate(&display, max_width)
}

fn cwd_right_truncate(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let w = UnicodeWidthStr::width(s);
    if w <= max_width {
        return s.to_string();
    }
    let mut result = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw + 1 > max_width {
            break;
        }
        result.push(ch);
        used += cw;
    }
    result.push('…');
    result
}

// ─── TopLabel ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TopLabel {
    pub id_marker: String,
    pub command: String,
    pub cwd: Option<String>,
    pub status: BlockStatus,
}

/// Build structured top border label parts for a CommandBlock.
pub fn build_top_label_parts(
    block: &CommandBlock,
    home: Option<&Path>,
    available_width: usize,
) -> TopLabel {
    let id_str = format!("[{}]", block.id);
    let id_w = UnicodeWidthStr::width(id_str.as_str());

    let marker = match block.status {
        BlockStatus::Failed => "✗",
        BlockStatus::Running => "…",
        _ => "",
    };

    let status = block.status.clone();

    if available_width <= id_w {
        return TopLabel {
            id_marker: truncate_label(&id_str, available_width),
            command: String::new(),
            cwd: None,
            status,
        };
    }

    let marker_w = UnicodeWidthStr::width(marker);
    let base_cost = id_w + 2;
    let mut remaining = available_width.saturating_sub(base_cost);
    let include_marker = marker_w > 0 && remaining >= marker_w + 1;

    if !include_marker && remaining == 0 {
        return TopLabel {
            id_marker: id_str,
            command: String::new(),
            cwd: None,
            status,
        };
    }

    if include_marker {
        remaining = remaining.saturating_sub(marker_w + 1);
    }

    // Attempt 1: id marker + cmd + cwd
    let cwd_budget = (remaining / 3).min(32);
    let cmd_budget_with_cwd = remaining.saturating_sub(2).saturating_sub(cwd_budget);

    if cwd_budget >= 4 && cmd_budget_with_cwd >= 1 {
        let cwd_str = compact_cwd(&block.cwd, home, cwd_budget);
        if !cwd_str.is_empty() {
            let cmd_str = compact_command(&block.command, cmd_budget_with_cwd);
            let id_marker = if include_marker {
                format!("{id_str} {marker}")
            } else {
                id_str.clone()
            };
            let candidate = format!("{id_marker}  {cmd_str}  {cwd_str}");
            if UnicodeWidthStr::width(candidate.as_str()) <= available_width {
                return TopLabel {
                    id_marker,
                    command: cmd_str,
                    cwd: Some(cwd_str),
                    status,
                };
            }
        }
    }

    // Attempt 2: id marker + cmd (no cwd)
    if remaining >= 1 {
        let cmd_str = compact_command(&block.command, remaining);
        let id_marker = if include_marker {
            format!("{id_str} {marker}")
        } else {
            id_str.clone()
        };
        let candidate = format!("{id_marker}  {cmd_str}");
        if UnicodeWidthStr::width(candidate.as_str()) <= available_width {
            return TopLabel {
                id_marker,
                command: cmd_str,
                cwd: None,
                status,
            };
        }
    }

    TopLabel {
        id_marker: if include_marker {
            format!("{id_str} {marker}")
        } else {
            id_str
        },
        command: String::new(),
        cwd: None,
        status,
    }
}

// ─── build_top_label (flat string, kept for tests and Detail View) ───────────

/// Build the top border label string for a CommandBlock.
pub fn build_top_label(
    block: &CommandBlock,
    home: Option<&Path>,
    available_width: usize,
) -> String {
    let p = build_top_label_parts(block, home, available_width);
    if p.command.is_empty() {
        return p.id_marker;
    }
    match p.cwd {
        Some(ref c) => format!("{}  {}  {}", p.id_marker, p.command, c),
        None => format!("{}  {}", p.id_marker, p.command),
    }
}

/// Truncate a string to a given display width (no ellipsis).
pub fn truncate_str(s: &str, max_width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w <= max_width {
        return s.to_string();
    }
    let mut result = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw > max_width {
            break;
        }
        result.push(ch);
        used += cw;
    }
    result
}

fn truncate_label(s: &str, max_width: usize) -> String {
    let w = UnicodeWidthStr::width(s);
    if w <= max_width {
        return s.to_string();
    }
    let mut result = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + cw > max_width {
            break;
        }
        result.push(ch);
        used += cw;
    }
    result
}

// ─── CopyFormat ──────────────────────────────────────────────────────────────

/// Which part of a block to include in a copy operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyPart {
    Command,
    Output,
    Both,
}

/// Serialization format for copied block content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CopyFormat {
    #[default]
    Plaintext,
    Markdown,
    ShellTranscript,
    Json,
}

impl CopyFormat {
    pub fn name(self) -> &'static str {
        match self {
            CopyFormat::Plaintext => "plaintext",
            CopyFormat::Markdown => "markdown",
            CopyFormat::ShellTranscript => "transcript",
            CopyFormat::Json => "json",
        }
    }
}

/// Serialize one or more blocks into a string ready for the clipboard.
///
/// Multiple blocks are joined with a per-format separator:
/// - Plaintext: `\n\n---\n\n`
/// - Markdown / ShellTranscript: `\n\n`
/// - Json: wrapped in a JSON array `[…]`
pub fn format_blocks(blocks: &[&CommandBlock], part: CopyPart, fmt: CopyFormat) -> String {
    let entries: Vec<String> = blocks
        .iter()
        .map(|b| format_one(b, part, fmt))
        .collect();

    if entries.is_empty() {
        return String::new();
    }

    match fmt {
        CopyFormat::Json if entries.len() > 1 => {
            format!("[{}]", entries.join(","))
        }
        CopyFormat::Plaintext => entries.join("\n\n---\n\n"),
        CopyFormat::Markdown | CopyFormat::ShellTranscript | CopyFormat::Json => {
            entries.join("\n\n")
        }
    }
}

fn format_one(block: &CommandBlock, part: CopyPart, fmt: CopyFormat) -> String {
    let cmd = &block.command;
    let out = &block.output_text;
    match fmt {
        CopyFormat::Plaintext => match part {
            CopyPart::Command => cmd.clone(),
            CopyPart::Output => out.clone(),
            CopyPart::Both => format!("{cmd}\n\n{out}"),
        },
        CopyFormat::Markdown => match part {
            CopyPart::Command => format!("`{cmd}`"),
            CopyPart::Output => format!("```\n{out}\n```"),
            CopyPart::Both => format!("## `{cmd}`\n\n```\n{out}\n```"),
        },
        CopyFormat::ShellTranscript => match part {
            CopyPart::Command => format!("$ {cmd}"),
            CopyPart::Output => out.clone(),
            CopyPart::Both => format!("$ {cmd}\n{out}"),
        },
        CopyFormat::Json => match part {
            CopyPart::Command => format!("{{\"command\":{}}}", json_string(cmd)),
            CopyPart::Output => format!("{{\"output\":{}}}", json_string(out)),
            CopyPart::Both => format!(
                "{{\"command\":{},\"output\":{}}}",
                json_string(cmd),
                json_string(out)
            ),
        },
    }
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
                let _ = std::fmt::Write::write_fmt(
                    &mut out,
                    format_args!("\\u{:04x}", c as u32),
                );
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{BlockId, BlockStatus, CommandBlock};
    use std::path::PathBuf;

    // ── compact_command ──────────────────────────────────────────────────────

    #[test]
    fn compact_command_empty() {
        assert_eq!(compact_command("", 20), "");
    }

    #[test]
    fn compact_command_zero_width() {
        assert_eq!(compact_command("ls", 0), "");
    }

    #[test]
    fn compact_command_short_fits() {
        assert_eq!(compact_command("ls", 20), "ls");
        assert_eq!(compact_command("cargo test", 20), "cargo test");
    }

    #[test]
    fn compact_command_exactly_max() {
        assert_eq!(compact_command("abc", 3), "abc");
    }

    #[test]
    fn compact_command_long_truncated() {
        let result = compact_command("cargo test --workspace --all-features -- --nocapture", 24);
        assert!(result.ends_with('…'));
        assert!(UnicodeWidthStr::width(result.as_str()) <= 24);
        assert!(result.starts_with("cargo test"));
    }

    #[test]
    fn compact_command_multiline_normalized() {
        assert_eq!(
            compact_command("git\ncommit -m foo", 30),
            "git commit -m foo"
        );
    }

    #[test]
    fn compact_command_collapse_spaces() {
        assert_eq!(
            compact_command("git   commit  -m foo", 30),
            "git commit -m foo"
        );
    }

    #[test]
    fn compact_command_ansi_stripped() {
        let result = compact_command("\x1b[31mfoo\x1b[0m", 20);
        assert_eq!(result, "foo");
    }

    #[test]
    fn compact_command_unicode_width() {
        let result = compact_command("目录检查", 3);
        assert!(UnicodeWidthStr::width(result.as_str()) <= 3);
        assert!(result.ends_with('…'));
    }

    // ── compact_cwd ──────────────────────────────────────────────────────────

    fn home() -> PathBuf {
        PathBuf::from("/Users/bob")
    }

    #[test]
    fn compact_cwd_home_root() {
        assert_eq!(compact_cwd(&home(), Some(&home()), 20), "~");
    }

    #[test]
    fn compact_cwd_home_short() {
        let p = PathBuf::from("/Users/bob/Projects/tide");
        assert_eq!(compact_cwd(&p, Some(&home()), 30), "~/Projects/tide");
    }

    #[test]
    fn compact_cwd_home_long_two_tail() {
        let p = PathBuf::from("/Users/bob/Projects/very-long/packages/frontend/src");
        let result = compact_cwd(&p, Some(&home()), 28);
        assert_eq!(result, "~/…/frontend/src");
    }

    #[test]
    fn compact_cwd_home_long_one_tail() {
        let p = PathBuf::from("/Users/bob/Projects/very-long/packages/frontend/src");
        let result = compact_cwd(&p, Some(&home()), 12);
        assert!(UnicodeWidthStr::width(result.as_str()) <= 12);
    }

    #[test]
    fn compact_cwd_very_narrow_right_truncate() {
        let p = PathBuf::from("/Users/bob/Projects/tide");
        let result = compact_cwd(&p, Some(&home()), 6);
        assert!(UnicodeWidthStr::width(result.as_str()) <= 6);
        assert!(result.ends_with('…'));
    }

    #[test]
    fn compact_cwd_absolute_no_home() {
        let p = PathBuf::from("/opt/homebrew/lib/ruby/gems");
        let result = compact_cwd(&p, None, 20);
        assert!(UnicodeWidthStr::width(result.as_str()) <= 20);
    }

    #[test]
    fn compact_cwd_short_absolute_fits() {
        let p = PathBuf::from("/tmp");
        assert_eq!(compact_cwd(&p, Some(&home()), 10), "/tmp");
    }

    #[test]
    fn compact_cwd_unicode_path() {
        let p = PathBuf::from("/Users/bob/项目/前端/src");
        let result = compact_cwd(&p, Some(&home()), 16);
        assert!(UnicodeWidthStr::width(result.as_str()) <= 16);
    }

    #[test]
    fn compact_cwd_zero_width() {
        let p = PathBuf::from("/Users/bob");
        assert_eq!(compact_cwd(&p, Some(&home()), 0), "");
    }

    // ── build_top_label ──────────────────────────────────────────────────────

    fn make_block(id: u64, command: &str, status: BlockStatus, cwd: &str) -> CommandBlock {
        CommandBlock {
            id: BlockId(id),
            command: command.to_string(),
            cwd: PathBuf::from(cwd),
            status,
            // fill remaining fields with defaults
            ..CommandBlock::default()
        }
    }

    #[test]
    fn build_top_label_full() {
        let b = make_block(
            42,
            "cargo test",
            BlockStatus::Success,
            "/Users/bob/Projects/tide",
        );
        let label = build_top_label(&b, Some(&home()), 60);
        assert!(label.contains("[42]"));
        assert!(label.contains("cargo test"));
        assert!(label.contains("~/Projects/tide"));
    }

    #[test]
    fn build_top_label_failed_marker_before_cwd() {
        let b = make_block(37, "al", BlockStatus::Failed, "/Users/bob");
        let label = build_top_label(&b, Some(&home()), 40);
        assert!(label.contains("✗"));
        if let (Some(m), Some(c)) = (label.find('✗'), label.find('~')) {
            assert!(m < c, "marker should appear before cwd: {label}");
        }
    }

    #[test]
    fn build_top_label_hide_cwd_when_narrow() {
        let b = make_block(
            88,
            "npm run build",
            BlockStatus::Success,
            "/Users/bob/very-long-project/frontend",
        );
        let label = build_top_label(&b, Some(&home()), 16);
        assert!(label.contains("[88]"));
        // At 16-width budget, cwd is dropped or heavily truncated
        assert!(UnicodeWidthStr::width(label.as_str()) <= 16);
    }

    #[test]
    fn build_top_label_command_truncated_when_very_narrow() {
        let b = make_block(1, "cargo test --workspace", BlockStatus::Success, "/tmp");
        let label = build_top_label(&b, None, 18);
        assert!(UnicodeWidthStr::width(label.as_str()) <= 18);
    }

    #[test]
    fn build_top_label_id_only_when_extremely_narrow() {
        let b = make_block(42, "cargo test", BlockStatus::Failed, "/Users/bob");
        let label = build_top_label(&b, Some(&home()), 4);
        assert_eq!(label, "[42]");
    }

    #[test]
    fn label_width_never_exceeds_available() {
        let cases = [
            (5usize, "ls", BlockStatus::Success, "/tmp"),
            (
                15,
                "cargo test --workspace",
                BlockStatus::Failed,
                "/Users/bob/Projects",
            ),
            (
                40,
                "npm run build --production",
                BlockStatus::Running,
                "/Users/bob/projects/frontend",
            ),
            (
                80,
                "git commit -m 'fix: long message'",
                BlockStatus::Success,
                "/Users/bob/work/repo",
            ),
        ];
        for (width, cmd, status, cwd) in cases {
            let b = make_block(99, cmd, status, cwd);
            let label = build_top_label(&b, Some(&home()), width);
            let lw = UnicodeWidthStr::width(label.as_str());
            assert!(lw <= width, "width {lw} > {width} for label: {label:?}");
        }
    }

    // ── format_blocks ────────────────────────────────────────────────────────

    fn copy_block(command: &str, output: &str) -> CommandBlock {
        CommandBlock {
            command: command.to_string(),
            output_text: output.to_string(),
            ..CommandBlock::default()
        }
    }

    // -- Plaintext --

    #[test]
    fn plaintext_command_only() {
        let b = copy_block("ls -la", "file1\nfile2");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Command, CopyFormat::Plaintext),
            "ls -la"
        );
    }

    #[test]
    fn plaintext_output_only() {
        let b = copy_block("ls -la", "file1\nfile2");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Output, CopyFormat::Plaintext),
            "file1\nfile2"
        );
    }

    #[test]
    fn plaintext_both() {
        let b = copy_block("ls -la", "file1\nfile2");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Both, CopyFormat::Plaintext),
            "ls -la\n\nfile1\nfile2"
        );
    }

    #[test]
    fn plaintext_multi_record_separator() {
        let b1 = copy_block("ls", "a");
        let b2 = copy_block("pwd", "/home");
        let result = format_blocks(&[&b1, &b2], CopyPart::Both, CopyFormat::Plaintext);
        assert_eq!(result, "ls\n\na\n\n---\n\npwd\n\n/home");
    }

    // -- Markdown --

    #[test]
    fn markdown_command_only() {
        let b = copy_block("cargo test", "ok");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Command, CopyFormat::Markdown),
            "`cargo test`"
        );
    }

    #[test]
    fn markdown_output_only() {
        let b = copy_block("cargo test", "ok");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Output, CopyFormat::Markdown),
            "```\nok\n```"
        );
    }

    #[test]
    fn markdown_both() {
        let b = copy_block("cargo test", "ok");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Both, CopyFormat::Markdown),
            "## `cargo test`\n\n```\nok\n```"
        );
    }

    #[test]
    fn markdown_multi_record_separator() {
        let b1 = copy_block("ls", "a");
        let b2 = copy_block("pwd", "/home");
        let result = format_blocks(&[&b1, &b2], CopyPart::Both, CopyFormat::Markdown);
        assert_eq!(
            result,
            "## `ls`\n\n```\na\n```\n\n## `pwd`\n\n```\n/home\n```"
        );
    }

    // -- ShellTranscript --

    #[test]
    fn transcript_command_only() {
        let b = copy_block("echo hi", "hi");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Command, CopyFormat::ShellTranscript),
            "$ echo hi"
        );
    }

    #[test]
    fn transcript_output_only() {
        let b = copy_block("echo hi", "hi");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Output, CopyFormat::ShellTranscript),
            "hi"
        );
    }

    #[test]
    fn transcript_both() {
        let b = copy_block("echo hi", "hi");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Both, CopyFormat::ShellTranscript),
            "$ echo hi\nhi"
        );
    }

    #[test]
    fn transcript_multi_record() {
        let b1 = copy_block("ls", "a");
        let b2 = copy_block("pwd", "/home");
        let result = format_blocks(&[&b1, &b2], CopyPart::Both, CopyFormat::ShellTranscript);
        assert_eq!(result, "$ ls\na\n\n$ pwd\n/home");
    }

    // -- Json --

    #[test]
    fn json_single_both() {
        let b = copy_block("ls", "a\nb");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Both, CopyFormat::Json),
            r#"{"command":"ls","output":"a\nb"}"#
        );
    }

    #[test]
    fn json_command_only() {
        let b = copy_block("ls", "a");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Command, CopyFormat::Json),
            r#"{"command":"ls"}"#
        );
    }

    #[test]
    fn json_output_only() {
        let b = copy_block("ls", "a");
        assert_eq!(
            format_blocks(&[&b], CopyPart::Output, CopyFormat::Json),
            r#"{"output":"a"}"#
        );
    }

    #[test]
    fn json_multi_record_array() {
        let b1 = copy_block("ls", "a");
        let b2 = copy_block("pwd", "/");
        let result = format_blocks(&[&b1, &b2], CopyPart::Both, CopyFormat::Json);
        assert_eq!(
            result,
            r#"[{"command":"ls","output":"a"},{"command":"pwd","output":"/"}]"#
        );
    }

    #[test]
    fn json_escapes_special_chars() {
        let b = copy_block("echo \"hi\"", "line1\nline2\ttab");
        let result = format_blocks(&[&b], CopyPart::Both, CopyFormat::Json);
        assert!(result.contains(r#"\"hi\""#), "quotes should be escaped");
        assert!(result.contains(r#"\n"#), "newlines should be escaped");
        assert!(result.contains(r#"\t"#), "tabs should be escaped");
    }

    // -- edge cases --

    #[test]
    fn empty_block_list_returns_empty() {
        let result = format_blocks(&[], CopyPart::Both, CopyFormat::Plaintext);
        assert_eq!(result, "");
    }

    #[test]
    fn single_record_json_is_not_wrapped_in_array() {
        let b = copy_block("ls", "a");
        let result = format_blocks(&[&b], CopyPart::Both, CopyFormat::Json);
        assert!(!result.starts_with('['), "single record should not be a JSON array");
    }
}
