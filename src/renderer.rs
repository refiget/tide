use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute, queue,
    style::{Attribute, Print, SetAttribute},
    terminal::{Clear, ClearType},
};
use unicode_width::UnicodeWidthStr;

use crate::{app::ViewKind, compositor::VisualLine, config::BlockLayoutConfig};

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct Theme;

pub fn render<W: Write>(
    w: &mut W,
    visual_lines: &[VisualLine],
    view: &crate::app::ViewState,
    cursor: Option<(usize, usize)>,
    layout: &BlockLayoutConfig,
    rows: u16,
    cols: u16,
) -> io::Result<()> {
    let height = rows as usize;
    let start = viewport_start(visual_lines, view, height);

    execute!(w, Hide, MoveTo(0, 0), Clear(ClearType::All))?;

    for (row, line) in visual_lines.iter().skip(start).take(height).enumerate() {
        queue!(w, MoveTo(0, row as u16))?;
        render_line(w, line, cols as usize, layout)?;
        queue!(w, Clear(ClearType::UntilNewLine))?;
    }

    if matches!(view.view, ViewKind::Plain) {
        let (cursor_line, cursor_col) = cursor.unwrap_or_else(|| {
            let row = visual_lines.len().saturating_sub(1);
            let col = visual_lines
                .last()
                .and_then(shell_text)
                .map(|text| UnicodeWidthStr::width(text.as_str()))
                .unwrap_or(0);
            (row, col)
        });
        let cursor_row = cursor_line
            .saturating_sub(start)
            .min(height.saturating_sub(1));
        let cursor_col = if layout.show_padding_in_plain {
            cursor_col.saturating_add(layout.horizontal_padding)
        } else {
            cursor_col
        }
        .min(cols as usize);
        queue!(w, MoveTo(cursor_col as u16, cursor_row as u16), Show)?;
    } else {
        queue!(w, Hide)?;
    }

    w.flush()
}

fn viewport_start(lines: &[VisualLine], view: &crate::app::ViewState, height: usize) -> usize {
    if height == 0 || lines.len() <= height {
        return 0;
    }

    if matches!(view.view, ViewKind::Plain) {
        return lines.len().saturating_sub(height);
    }

    0
}

fn shell_text(line: &VisualLine) -> Option<String> {
    match line {
        VisualLine::ShellText { text, .. } => Some(text.trim_end().to_string()),
        _ => None,
    }
}

fn render_line<W: Write>(
    w: &mut W,
    line: &VisualLine,
    width: usize,
    layout: &BlockLayoutConfig,
) -> io::Result<()> {
    match line {
        VisualLine::Empty => {}
        VisualLine::ShellText { text, block_id } => {
            let _ = block_id;
            let text = if layout.show_padding_in_plain {
                format!(
                    "{}{}",
                    " ".repeat(layout.horizontal_padding),
                    text.trim_end()
                )
            } else {
                text.trim_end().to_string()
            };
            queue!(w, Print(truncate_to_width(&text, width)))?;
        }
        VisualLine::BlockBodyLine {
            text,
            block_id,
            selected,
        } => {
            let _ = block_id;
            render_framed_text(w, text, *selected, width, layout)?;
        }
        VisualLine::BlockTopBorder {
            block_id,
            selected,
            label,
        } => {
            let _ = block_id;
            render_border(w, label, *selected, true, width)?;
        }
        VisualLine::BlockBottomBorder {
            block_id,
            selected,
            label,
        } => {
            let _ = block_id;
            render_border(w, label, *selected, false, width)?;
        }
        VisualLine::BlockDetailLine {
            block_id,
            text,
            selected,
        } => {
            let _ = block_id;
            render_framed_text(w, text, *selected, width, layout)?;
        }
    }

    Ok(())
}

fn render_border<W: Write>(
    w: &mut W,
    label: &str,
    selected: bool,
    top: bool,
    width: usize,
) -> io::Result<()> {
    let (left, right) = match (selected, top) {
        (true, true) => ('╭', '╮'),
        (true, false) => ('╰', '╯'),
        (false, true) => ('┌', '┐'),
        (false, false) => ('└', '┘'),
    };

    if selected {
        queue!(w, SetAttribute(Attribute::Reverse))?;
    }
    queue!(w, Print(titled_border(left, right, label, width)))?;
    if selected {
        queue!(w, SetAttribute(Attribute::Reset))?;
    }

    Ok(())
}

fn render_framed_text<W: Write>(
    w: &mut W,
    text: &str,
    selected: bool,
    width: usize,
    layout: &BlockLayoutConfig,
) -> io::Result<()> {
    if selected {
        queue!(w, SetAttribute(Attribute::Reverse))?;
    }
    queue!(w, Print(framed_text(text, width, layout)))?;
    if selected {
        queue!(w, SetAttribute(Attribute::Reset))?;
    }

    Ok(())
}

fn framed_text(text: &str, width: usize, layout: &BlockLayoutConfig) -> String {
    if width < 4 {
        return truncate_to_width(text, width);
    }

    let inner_width = width - 4;
    let padding = " ".repeat(layout.horizontal_padding);
    let text = truncate_to_width(&format!("{padding}{text}"), inner_width);
    let fill = inner_width.saturating_sub(UnicodeWidthStr::width(text.as_str()));
    format!("│{text}{} │", " ".repeat(fill.saturating_sub(1)))
}

fn titled_border(left: char, right: char, label: &str, width: usize) -> String {
    if width < 2 {
        return String::new();
    }

    let inner_width = width - 2;
    let label = truncate_to_width(&format!("─ {label} "), inner_width);
    let fill = inner_width.saturating_sub(UnicodeWidthStr::width(label.as_str()));
    format!("{left}{label}{}{right}", "─".repeat(fill))
}

fn truncate_to_width(value: &str, max_width: usize) -> String {
    let mut result = String::new();
    let mut width = 0;

    for ch in value.chars() {
        let ch_width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if width + ch_width > max_width {
            break;
        }
        result.push(ch);
        width += ch_width;
    }

    result
}
