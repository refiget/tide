use std::{env, path::PathBuf, time::SystemTime};

use anyhow::{Result, anyhow};

use crate::{
    app::{BlockId, BlockKind, BlockStatus, CommandBlock},
    block_export::{ExportPart, format_block_json},
};

pub enum CliCommand {
    RunShell,
    Export(ExportArgs),
}

#[derive(Debug)]
pub struct ExportArgs {
    pub part: ExportPart,
    pub command: String,
    pub output: String,
    pub cwd: PathBuf,
    pub kind: BlockKind,
    pub status: BlockStatus,
    pub exit_code: Option<i32>,
    pub truncated: bool,
}

pub fn parse_cli_command() -> Result<CliCommand> {
    let mut args = env::args().skip(1);
    let Some(first) = args.next() else {
        return Ok(CliCommand::RunShell);
    };

    if first != "export" {
        return Ok(CliCommand::RunShell);
    }

    let export_args = parse_export_args_from_iter(args.collect())?;
    Ok(CliCommand::Export(export_args))
}

pub fn run_export_command(args: ExportArgs) {
    let now = SystemTime::now();
    let block = CommandBlock {
        id: BlockId(1),
        command: args.command,
        cwd: args.cwd,
        started_at: now,
        finished_at: Some(now),
        duration_ms: Some(0),
        exit_code: args.exit_code,
        output_raw: args.output.as_bytes().to_vec(),
        output_text: args.output,
        kind: args.kind,
        status: args.status,
        output_truncated: args.truncated,
        ..CommandBlock::default()
    };

    println!("{}", format_block_json(&block, args.part));
}

fn parse_export_args_from_iter(raw: Vec<String>) -> Result<ExportArgs> {
    if raw.iter().any(|a| a == "--help" || a == "-h") {
        return Err(anyhow!(export_help()));
    }

    let mut part = ExportPart::Both;
    let mut command = String::new();
    let mut output = String::new();
    let mut cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut kind = BlockKind::NormalCommand;
    let mut status = BlockStatus::Success;
    let mut exit_code = Some(0);
    let mut truncated = false;

    let mut i = 0usize;
    while i < raw.len() {
        let key = &raw[i];
        let val = raw
            .get(i + 1)
            .ok_or_else(|| anyhow!("missing value for `{key}`\n\n{}", export_help()))?;
        match key.as_str() {
            "--part" => part = parse_part(val)?,
            "--command" => command = val.clone(),
            "--output" => output = val.clone(),
            "--cwd" => cwd = PathBuf::from(val),
            "--kind" => kind = parse_kind(val)?,
            "--status" => status = parse_status(val)?,
            "--exit-code" => {
                exit_code = Some(
                    val.parse::<i32>()
                        .map_err(|_| anyhow!("invalid --exit-code `{val}`"))?,
                )
            }
            "--truncated" => truncated = parse_bool(val)?,
            _ => {
                return Err(anyhow!(
                    "unknown option `{key}`\n\n{}",
                    export_help()
                ));
            }
        }
        i += 2;
    }

    if command.is_empty() {
        return Err(anyhow!("`--command` is required\n\n{}", export_help()));
    }

    if matches!(status, BlockStatus::Running) {
        exit_code = None;
    }

    Ok(ExportArgs {
        part,
        command,
        output,
        cwd,
        kind,
        status,
        exit_code,
        truncated,
    })
}

fn parse_part(s: &str) -> Result<ExportPart> {
    match s {
        "command" => Ok(ExportPart::Command),
        "output" => Ok(ExportPart::Output),
        "both" => Ok(ExportPart::Both),
        _ => Err(anyhow!("invalid --part `{s}` (expected command|output|both)")),
    }
}

fn parse_kind(s: &str) -> Result<BlockKind> {
    match s {
        "normal_command" => Ok(BlockKind::NormalCommand),
        "tui_session" => Ok(BlockKind::TuiSession),
        "raw_program" => Ok(BlockKind::RawProgram),
        "ai_generated" => Ok(BlockKind::AiGenerated),
        "system_event" => Ok(BlockKind::SystemEvent),
        _ => Err(anyhow!("invalid --kind `{s}`")),
    }
}

fn parse_status(s: &str) -> Result<BlockStatus> {
    match s {
        "running" => Ok(BlockStatus::Running),
        "success" => Ok(BlockStatus::Success),
        "failed" => Ok(BlockStatus::Failed),
        "interrupted" => Ok(BlockStatus::Interrupted),
        "unknown" => Ok(BlockStatus::Unknown),
        _ => Err(anyhow!("invalid --status `{s}`")),
    }
}

fn parse_bool(s: &str) -> Result<bool> {
    match s {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(anyhow!("invalid bool `{s}` (expected true|false)")),
    }
}

fn export_help() -> &'static str {
    "Usage: tide export --command <cmd> [--output <text>] [--part command|output|both] [--cwd <path>] [--kind normal_command|tui_session|raw_program|ai_generated|system_event] [--status running|success|failed|interrupted|unknown] [--exit-code <i32>] [--truncated true|false]"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_export_args_basics() {
        let args = parse_export_args_from_iter(vec![
            "--command".to_string(),
            "echo hi".to_string(),
            "--output".to_string(),
            "hi".to_string(),
            "--part".to_string(),
            "both".to_string(),
            "--kind".to_string(),
            "normal_command".to_string(),
            "--status".to_string(),
            "success".to_string(),
        ])
        .expect("args should parse");

        assert!(matches!(args.part, ExportPart::Both));
        assert_eq!(args.command, "echo hi");
        assert_eq!(args.output, "hi");
        assert!(matches!(args.kind, BlockKind::NormalCommand));
        assert!(matches!(args.status, BlockStatus::Success));
    }

    #[test]
    fn parse_export_args_running_clears_exit_code() {
        let args = parse_export_args_from_iter(vec![
            "--command".to_string(),
            "tail -f".to_string(),
            "--status".to_string(),
            "running".to_string(),
            "--exit-code".to_string(),
            "3".to_string(),
        ])
        .expect("args should parse");

        assert!(args.exit_code.is_none());
    }

    #[test]
    fn parse_export_args_requires_command() {
        let err = parse_export_args_from_iter(vec!["--output".to_string(), "x".to_string()])
            .expect_err("missing command should fail");
        assert!(err.to_string().contains("--command"));
    }
}
