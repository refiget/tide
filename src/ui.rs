use std::{
    io::{self, Read, Write},
    sync::{Arc, Mutex},
};

use anyhow::Result;
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute,
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};

use crate::{
    app::{BlockStatus, CommandBlock},
    block::{BlockStore, format_duration_ms, format_started_at},
};

pub fn run_block_mode(blocks: Arc<Mutex<BlockStore>>) -> Result<()> {
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, Hide)?;

    let result = run_block_mode_inner(&mut stdout, blocks);

    execute!(stdout, Show, LeaveAlternateScreen)?;
    result
}

fn run_block_mode_inner(stdout: &mut io::Stdout, blocks: Arc<Mutex<BlockStore>>) -> Result<()> {
    let mut selected = 0_usize;
    let mut stdin = io::stdin();

    loop {
        let snapshot = blocks
            .lock()
            .map(|store| store.blocks_newest_first())
            .unwrap_or_default();

        if selected >= snapshot.len() {
            selected = snapshot.len().saturating_sub(1);
        }

        render(stdout, &snapshot, selected)?;

        let mut byte = [0_u8; 1];
        stdin.read_exact(&mut byte)?;

        match byte[0] {
            b'\x1b' => break,
            b'j' => {
                if selected + 1 < snapshot.len() {
                    selected += 1;
                }
            }
            b'k' => {
                selected = selected.saturating_sub(1);
            }
            b'q' => break,
            _ => {}
        }
    }

    Ok(())
}

fn render(stdout: &mut io::Stdout, blocks: &[CommandBlock], selected: usize) -> Result<()> {
    let (cols, rows) = terminal::size().unwrap_or((100, 30));
    let width = cols.max(40) as usize;
    let list_height = (rows as usize / 3).clamp(6, 12);
    let detail_start = list_height + 2;

    execute!(stdout, MoveTo(0, 0), Clear(ClearType::All))?;

    write_line(stdout, &top_border(" Tide Blocks ", width))?;

    if blocks.is_empty() {
        write_framed(stdout, "  No command blocks captured yet.", width)?;
    } else {
        for index in 0..list_height.saturating_sub(2) {
            let Some(block) = blocks.get(index) else {
                write_framed(stdout, "", width)?;
                continue;
            };

            let marker = if index == selected { ">" } else { " " };
            let status = match block.status {
                BlockStatus::Running => "running",
                BlockStatus::Success => "success",
                BlockStatus::Failed => "failed",
                BlockStatus::Interrupted => "interrupted",
                BlockStatus::Unknown => "unknown",
            };
            let line = format!(
                "{marker} [{:>3}] {:<32} {:<11} {:>8}",
                block.id,
                truncate_single_line(&block.command, 32),
                status,
                format_duration_ms(block.duration_ms)
            );
            write_framed(stdout, &line, width)?;
        }
    }

    write_line(stdout, &middle_border(" Selected Block ", width))?;

    if let Some(block) = blocks.get(selected) {
        write_framed(stdout, &format!("command: {}", block.command), width)?;
        write_framed(stdout, &format!("cwd: {}", block.cwd.display()), width)?;
        write_framed(
            stdout,
            &format!(
                "exit: {}  duration: {}  started: {}",
                block
                    .exit_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                format_duration_ms(block.duration_ms),
                format_started_at(block.started_at)
            ),
            width,
        )?;
        write_line(stdout, &middle_border(" Output ", width))?;

        let output_height = (rows as usize).saturating_sub(detail_start + 7).max(3);
        let output_lines = block.output_text.lines().take(output_height);
        for line in output_lines {
            write_framed(
                stdout,
                &truncate_single_line(line, width.saturating_sub(4)),
                width,
            )?;
        }
    } else {
        write_framed(
            stdout,
            "Run commands in Shell Mode, then press Ctrl-X Ctrl-B.",
            width,
        )?;
    }

    write_line(stdout, &middle_border(" Keys ", width))?;
    write_framed(stdout, "j/k: select block    Esc/q: return to shell", width)?;
    write_line(stdout, &bottom_border(width))?;
    stdout.flush()?;

    Ok(())
}

fn top_border(title: &str, width: usize) -> String {
    titled_border('┌', '┐', title, width)
}

fn middle_border(title: &str, width: usize) -> String {
    titled_border('├', '┤', title, width)
}

fn bottom_border(width: usize) -> String {
    format!("└{}┘", "─".repeat(width.saturating_sub(2)))
}

fn titled_border(left: char, right: char, title: &str, width: usize) -> String {
    let available = width.saturating_sub(2);
    let title = if title.len() + 2 < available {
        title.to_string()
    } else {
        String::new()
    };
    let rest = available.saturating_sub(title.len());
    format!("{left}{title}{}{right}", "─".repeat(rest))
}

fn write_framed(stdout: &mut io::Stdout, content: &str, width: usize) -> Result<()> {
    let inner_width = width.saturating_sub(4);
    let content = truncate_single_line(content, inner_width);
    write_line(
        stdout,
        &format!("│ {:<inner_width$} │", content, inner_width = inner_width),
    )
}

fn write_line(stdout: &mut io::Stdout, line: &str) -> Result<()> {
    stdout.write_all(line.as_bytes())?;
    stdout.write_all(b"\r\n")?;
    Ok(())
}

fn truncate_single_line(value: &str, max_chars: usize) -> String {
    let value = value.replace(['\r', '\n'], " ");
    if value.chars().count() <= max_chars {
        return value;
    }

    let mut result = value
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    result.push('…');
    result
}
