use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute, queue,
    style::{Attribute, Print, ResetColor, SetAttribute},
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    app::ViewKind,
    compositor::VisualLine,
    config::{BlockLayoutConfig, BlockViewConfig},
};

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct Theme;

/// Enter alternate screen and hide cursor for Block/Detail view rendering.
pub fn enter_block_render<W: Write>(w: &mut W) -> io::Result<()> {
    execute!(w, EnterAlternateScreen, Hide)
}

/// Leave alternate screen, reset SGR, and show cursor when returning to Plain view.
///
/// # Ordering
///
/// `LeaveAlternateScreen` MUST come first so that `ResetColor` and `Show` are
/// applied on the *main* screen (the restored terminal state), not the alt
/// screen that is about to be discarded.
pub fn leave_block_render<W: Write>(w: &mut W) -> io::Result<()> {
    execute!(w, LeaveAlternateScreen, ResetColor, Show)
}

pub fn render<W: Write>(
    w: &mut W,
    visual_lines: &[VisualLine],
    view: &crate::app::ViewState,
    cursor: Option<(usize, usize)>,
    layout: &BlockLayoutConfig,
    block_view: &BlockViewConfig,
    rows: u16,
    cols: u16,
) -> io::Result<()> {
    let height = rows as usize;
    let start = viewport_start(visual_lines, view, height);

    queue!(w, MoveTo(0, 0), Clear(ClearType::All))?;

    for (row, line) in visual_lines.iter().skip(start).take(height).enumerate() {
        queue!(w, MoveTo(0, row as u16))?;
        render_line(w, line, cols as usize, layout, block_view)?;
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
    block_view: &BlockViewConfig,
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
            render_framed_text(w, text, *selected, width, layout, block_view)?;
        }
        VisualLine::BlockTopBorder {
            block_id,
            selected,
            label,
        } => {
            let _ = block_id;
            render_border(w, label, *selected, true, width, block_view)?;
        }
        VisualLine::BlockBottomBorder {
            block_id,
            selected,
            label,
        } => {
            let _ = block_id;
            render_border(w, label, *selected, false, width, block_view)?;
        }
        VisualLine::BlockDetailLine {
            block_id,
            text,
            selected,
        } => {
            let _ = block_id;
            render_framed_text(w, text, *selected, width, layout, block_view)?;
        }
        VisualLine::Footer { text } => {
            render_footer(w, text, width)?;
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
    block_view: &BlockViewConfig,
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
    queue!(
        w,
        Print(with_margin(
            &titled_border(left, right, label, block_width(width, block_view)),
            block_view
        ))
    )?;
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
    _layout: &BlockLayoutConfig,
    block_view: &BlockViewConfig,
) -> io::Result<()> {
    if selected && block_view.selected_body_reverse {
        queue!(w, SetAttribute(Attribute::Reverse))?;
    }
    queue!(
        w,
        Print(with_margin(
            &framed_text(
                text,
                block_width(width, block_view),
                block_view.body_padding
            ),
            block_view
        ))
    )?;
    if selected && block_view.selected_body_reverse {
        queue!(w, SetAttribute(Attribute::Reset))?;
    }

    Ok(())
}

fn framed_text(text: &str, width: usize, body_padding: usize) -> String {
    if width < 4 {
        return truncate_to_width(text, width);
    }

    let inner_width = width - 2;
    let padding = " ".repeat(body_padding);
    let text = truncate_to_width(&format!("{padding}{text}"), inner_width);
    let fill = inner_width.saturating_sub(UnicodeWidthStr::width(text.as_str()));
    format!("│{text}{}│", " ".repeat(fill))
}

fn render_footer<W: Write>(w: &mut W, text: &str, width: usize) -> io::Result<()> {
    queue!(w, SetAttribute(Attribute::Reverse))?;
    queue!(w, Print(pad_to_width(text, width)))?;
    queue!(w, SetAttribute(Attribute::Reset))?;
    Ok(())
}

fn block_width(width: usize, block_view: &BlockViewConfig) -> usize {
    width.saturating_sub(block_view.horizontal_margin.saturating_mul(2))
}

fn with_margin(value: &str, block_view: &BlockViewConfig) -> String {
    format!("{}{}", " ".repeat(block_view.horizontal_margin), value)
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

fn pad_to_width(value: &str, width: usize) -> String {
    let value = truncate_to_width(value, width);
    let fill = width.saturating_sub(UnicodeWidthStr::width(value.as_str()));
    format!("{value}{}", " ".repeat(fill))
}

#[cfg(test)]
mod tests {
    use unicode_width::UnicodeWidthStr;

    use super::*;

    #[test]
    fn framed_text_exact_width_with_wide_chars() {
        let line = framed_text("目录 󰉍 Downloads", 32, 1);
        assert_eq!(UnicodeWidthStr::width(line.as_str()), 32);
        assert!(line.starts_with('│'));
        assert!(line.ends_with('│'));
    }

    #[test]
    fn titled_border_exact_width_with_long_command() {
        let line = titled_border('┌', '┐', "#37  very long command with 中文 ✗", 40);
        assert_eq!(UnicodeWidthStr::width(line.as_str()), 40);
        assert!(line.starts_with('┌'));
        assert!(line.ends_with('┐'));
    }
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
