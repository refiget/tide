use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute, queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    ansi::{StyledText, TextStyle, styled_width, truncate_styled_to_width},
    app::{BlockStatus, ViewKind},
    compositor::{FooterSegment, VisualLine},
    config::{BlockLayoutConfig, BlockViewConfig},
    theme::{CatppuccinFrappe, Theme},
};

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
    last_rendered_rows: usize,
) -> io::Result<usize> {
    let height = rows as usize;
    let start = viewport_start(visual_lines, view, height);

    let rendered = visual_lines.len().saturating_sub(start).min(height);

    for (row, line) in visual_lines.iter().skip(start).take(height).enumerate() {
        queue!(w, MoveTo(0, row as u16))?;
        render_line(w, line, cols as usize, layout, block_view)?;
        queue!(w, Clear(ClearType::UntilNewLine))?;
    }

    // Clear tail lines from previous frame that are no longer covered.
    for row in rendered..last_rendered_rows {
        queue!(w, MoveTo(0, row as u16), Clear(ClearType::CurrentLine))?;
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

    w.flush()?;
    Ok(rendered)
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
            render_top_border(w, label, *selected, width, block_view)?;
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
            use_detail_border,
        } => {
            let _ = block_id;
            render_block_detail_line(
                w,
                text,
                *selected,
                *use_detail_border,
                width,
                layout,
                block_view,
            )?;
        }
        VisualLine::DetailTopBorder { label, .. } => {
            queue!(w, SetForegroundColor(Theme::DETAIL_BORDER_FG))?;
            queue!(
                w,
                Print(with_margin(
                    &titled_border('╭', '╮', label, block_width(width, block_view)),
                    block_view,
                ))
            )?;
            queue!(w, ResetColor)?;
        }
        VisualLine::DetailBottomBorder { label, .. } => {
            queue!(w, SetForegroundColor(Theme::DETAIL_BORDER_FG))?;
            queue!(
                w,
                Print(with_margin(
                    &titled_border('╰', '╯', label, block_width(width, block_view)),
                    block_view,
                ))
            )?;
            queue!(w, ResetColor)?;
        }

        VisualLine::StyledBlockBodyLine {
            styled,
            plain_text,
            selected,
            border_selected,
            ..
        } => {
            let border_fg = if *border_selected {
                Theme::BORDER_SELECTED_FG
            } else {
                Theme::BORDER_NORMAL_FG
            };
            let bg = selected.then_some(Theme::BODY_SELECTED_BG);
            render_styled_framed_text(
                w, styled, plain_text, bg, border_fg, width, layout, block_view,
            )?;
        }
        VisualLine::StyledDetailBodyLine {
            styled,
            plain_text,
            is_cursor,
            ..
        } => {
            let bg = is_cursor.then_some(Theme::CURSOR_BG);
            render_styled_framed_text(
                w,
                styled,
                plain_text,
                bg,
                Theme::DETAIL_BORDER_FG,
                width,
                layout,
                block_view,
            )?;
        }
        VisualLine::Footer { segments } => {
            render_footer(w, segments, width)?;
        }
    }

    Ok(())
}

fn render_top_border<W: Write>(
    w: &mut W,
    label: &crate::format::TopLabel,
    selected: bool,
    width: usize,
    block_view: &BlockViewConfig,
) -> io::Result<()> {
    let bw = block_width(width, block_view);
    if bw < 2 {
        return Ok(());
    }
    let inner_w = bw.saturating_sub(2);
    let margin = block_view.horizontal_margin;

    let left = if selected { '╭' } else { '┌' };
    let right = if selected { '╮' } else { '┐' };
    let border_fg = if selected {
        Theme::BORDER_SELECTED_FG
    } else {
        Theme::BORDER_NORMAL_FG
    };
    let command_fg = match label.status {
        BlockStatus::Success => Theme::STATUS_OK_FG,
        BlockStatus::Failed => Theme::STATUS_FAILED_FG,
        _ => {
            if selected {
                CatppuccinFrappe::TEXT
            } else {
                CatppuccinFrappe::SUBTEXT1
            }
        }
    };
    let bg = selected.then_some(Theme::BODY_SELECTED_BG);

    // Build content segments
    let mut segments: Vec<(crossterm::style::Color, String)> = Vec::new();
    segments.push((border_fg, format!("{left}─ ")));
    segments.push((border_fg, label.id_marker.clone()));

    if !label.command.is_empty() {
        segments.push((border_fg, "  ".to_string()));
        segments.push((command_fg, label.command.clone()));
    }

    if let Some(ref cwd) = label.cwd {
        segments.push((border_fg, "  ".to_string()));
        segments.push((Theme::META_PATH_FG, cwd.clone()));
    }

    // Calculate total used width (excluding fill and right corner)
    let mut used = 0usize;
    for (_, text) in &segments {
        used += UnicodeWidthStr::width(text.as_str());
    }
    let fill = inner_w.saturating_sub(used); // used includes left corner; " " + fill + "╮" must fit in inner_w

    // Render
    if let Some(bg) = bg {
        queue!(w, SetBackgroundColor(bg))?;
    }
    queue!(w, Print(" ".repeat(margin)))?;
    for (fg, text) in &segments {
        queue!(w, SetForegroundColor(*fg))?;
        queue!(w, Print(text))?;
    }
    queue!(w, SetForegroundColor(border_fg))?;
    queue!(w, Print(" "))?;
    queue!(w, Print("─".repeat(fill)))?;
    queue!(w, Print(right.to_string()))?;
    if selected {
        queue!(w, Print(" ".repeat(width.saturating_sub(bw + margin))))?;
    }
    queue!(w, ResetColor)?;
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
        let content = with_margin(
            &titled_border(left, right, label, block_width(width, block_view)),
            block_view,
        );
        queue!(w, SetForegroundColor(Theme::BORDER_SELECTED_FG))?;
        queue!(w, SetBackgroundColor(Theme::BODY_SELECTED_BG))?;
        queue!(w, Print(pad_to_width(&content, width)))?;
        queue!(w, ResetColor)?;
    } else {
        let content = with_margin(
            &titled_border(left, right, label, block_width(width, block_view)),
            block_view,
        );
        queue!(w, SetForegroundColor(Theme::BORDER_NORMAL_FG))?;
        queue!(w, Print(content))?;
        queue!(w, ResetColor)?;
    }

    Ok(())
}

fn parse_actions(value: &str) -> Vec<(String, String)> {
    value
        .split("   ")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|seg| {
            let (key, text) = seg.split_once(' ')?;
            Some((key.to_string(), text.to_string()))
        })
        .collect()
}

fn render_block_detail_line<W: Write>(
    w: &mut W,
    text: &str,
    selected: bool,
    use_detail_border: bool,
    width: usize,
    layout: &BlockLayoutConfig,
    block_view: &BlockViewConfig,
) -> io::Result<()> {
    // Border color: Detail View uses DETAIL_BORDER_FG; Block View tracks selection
    let border_fg = if use_detail_border {
        Theme::DETAIL_BORDER_FG
    } else if selected {
        Theme::BORDER_SELECTED_FG
    } else {
        Theme::BORDER_NORMAL_FG
    };
    // Background: only Block View selected rows get BODY_SELECTED_BG
    let bg = if selected && !use_detail_border {
        Some(Theme::BODY_SELECTED_BG)
    } else {
        None
    };

    if text.is_empty() {
        // Empty separator: render a plain framed empty line with correct border/bg
        if let Some(bg) = bg {
            queue!(w, SetBackgroundColor(bg))?;
        }
        let bw = block_width(width, block_view);
        if bw >= 4 {
            let inner_w = bw - 2;
            let margin = block_view.horizontal_margin;
            let body = " ".repeat(inner_w);
            queue!(w, SetForegroundColor(border_fg))?;
            queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
            queue!(w, ResetColor)?;
            if let Some(bg) = bg {
                queue!(w, SetBackgroundColor(bg))?;
            }
            queue!(w, Print(&body))?;
            queue!(w, SetForegroundColor(border_fg))?;
            queue!(w, Print("│"))?;
            if bg.is_some() {
                queue!(w, Print(" ".repeat(width.saturating_sub(bw + margin))))?;
            }
            queue!(w, ResetColor)?;
        }
        return Ok(());
    }

    let bw = block_width(width, block_view);
    if bw < 4 {
        return render_framed_text(w, text, selected, width, layout, block_view);
    }

    let margin = block_view.horizontal_margin;
    let inner_w = bw - 2;
    let padding = block_view.body_padding;
    let pad_str = " ".repeat(padding);

    if text == "Detail" {
        let content_w = inner_w.saturating_sub(padding);
        let label = truncate_to_width(text, content_w);
        let fill = content_w.saturating_sub(UnicodeWidthStr::width(label.as_str()));

        if let Some(bg) = bg {
            queue!(w, SetBackgroundColor(bg))?;
        }
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
        queue!(w, Print(&pad_str))?;
        queue!(w, SetAttribute(Attribute::Bold))?;
        queue!(w, SetForegroundColor(Theme::META_HEADER_FG))?;
        queue!(w, Print(&label))?;
        queue!(w, SetAttribute(Attribute::Reset))?;
        if let Some(bg) = bg {
            queue!(w, SetBackgroundColor(bg))?;
        }
        queue!(w, Print(" ".repeat(fill)))?;
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print("│"))?;
        if bg.is_some() {
            queue!(w, Print(" ".repeat(width.saturating_sub(bw + margin))))?;
        }
        queue!(w, ResetColor)?;
        return Ok(());
    }

    if let Some((label, value)) = text.split_once(": ") {
        let content_w = inner_w.saturating_sub(padding);
        let label_colon = format!("{label}: ");
        let label_w = UnicodeWidthStr::width(label_colon.as_str()).min(content_w);
        let value_w = content_w.saturating_sub(label_w);
        let label_display = truncate_to_width(&label_colon, label_w);
        let value_display = truncate_to_width(value, value_w);
        let fill_base = content_w
            .saturating_sub(UnicodeWidthStr::width(label_display.as_str()))
            .saturating_sub(UnicodeWidthStr::width(value_display.as_str()));

        if label == "actions" {
            if let Some(bg) = bg {
                queue!(w, SetBackgroundColor(bg))?;
            }
            queue!(w, SetForegroundColor(border_fg))?;
            queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
            queue!(w, Print(&pad_str))?;
            queue!(w, SetForegroundColor(Theme::META_LABEL_FG))?;
            queue!(w, Print(&label_display))?;

            let mut used_w = UnicodeWidthStr::width(label_display.as_str());
            let actions = parse_actions(value);
            for (key, action_text) in &actions {
                let seg = format!("{key} {action_text}");
                let seg_w = UnicodeWidthStr::width(seg.as_str());
                let remaining = content_w.saturating_sub(used_w);
                if seg_w > remaining {
                    break;
                }
                if used_w > 0 {
                    queue!(w, Print("   "))?;
                }
                queue!(w, SetAttribute(Attribute::Bold))?;
                queue!(w, SetForegroundColor(Theme::META_ACTION_KEY_FG))?;
                queue!(w, Print(key))?;
                queue!(w, SetAttribute(Attribute::Reset))?;
                if let Some(bg) = bg {
                    queue!(w, SetBackgroundColor(bg))?;
                }
                queue!(w, SetForegroundColor(Theme::META_ACTION_TEXT_FG))?;
                queue!(w, Print(" "))?;
                queue!(w, Print(action_text))?;
                used_w += seg_w + 3;
            }

            let remaining = content_w.saturating_sub(used_w);
            queue!(w, Print(" ".repeat(remaining)))?;
            queue!(w, SetForegroundColor(border_fg))?;
            queue!(w, Print("│"))?;
            if bg.is_some() {
                queue!(w, Print(" ".repeat(width.saturating_sub(bw + margin))))?;
            }
            queue!(w, ResetColor)?;
            return Ok(());
        }

        let value_fg = match label {
            "status" => match value {
                "ok" => Some(Theme::STATUS_OK_FG),
                "failed" => Some(Theme::STATUS_FAILED_FG),
                "running" => Some(Theme::STATUS_RUNNING_FG),
                _ => None,
            },
            "cwd" => Some(Theme::META_PATH_FG),
            "exit code" => {
                if value == "0" {
                    Some(Theme::STATUS_OK_FG)
                } else if value == "-" {
                    None
                } else {
                    Some(Theme::STATUS_FAILED_FG)
                }
            }
            "duration" => Some(Theme::STATUS_RUNNING_FG),
            "capture" | "type" => Some(Theme::STATUS_RUNNING_FG),
            _ => None,
        };

        if let Some(bg) = bg {
            queue!(w, SetBackgroundColor(bg))?;
        }
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
        queue!(w, Print(&pad_str))?;
        queue!(w, SetForegroundColor(Theme::META_LABEL_FG))?;
        queue!(w, Print(&label_display))?;
        if let Some(fg) = value_fg {
            queue!(w, SetForegroundColor(fg))?;
        } else {
            queue!(w, ResetColor)?;
            if let Some(bg) = bg {
                queue!(w, SetBackgroundColor(bg))?;
            }
        }
        queue!(w, Print(&value_display))?;
        queue!(w, Print(" ".repeat(fill_base)))?;
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print("│"))?;
        if bg.is_some() {
            queue!(w, Print(" ".repeat(width.saturating_sub(bw + margin))))?;
        }
        queue!(w, ResetColor)?;
        return Ok(());
    }

    let body = truncate_to_width(&format!("{pad_str}{text}"), inner_w);
    let fill = inner_w.saturating_sub(UnicodeWidthStr::width(body.as_str()));
    let text_fg = if use_detail_border {
        Theme::FOOTER_FG
    } else if selected {
        Theme::BODY_SELECTED_FG
    } else {
        Color::Reset
    };

    if let Some(bg) = bg {
        queue!(w, SetBackgroundColor(bg))?;
    }
    queue!(w, SetForegroundColor(border_fg))?;
    queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
    if let Some(bg) = bg {
        queue!(w, SetBackgroundColor(bg))?;
    }
    queue!(w, SetForegroundColor(text_fg))?;
    queue!(w, Print(format!("{body}{}", " ".repeat(fill))))?;
    queue!(w, SetForegroundColor(border_fg))?;
    queue!(w, Print("│"))?;
    if bg.is_some() {
        queue!(w, Print(" ".repeat(width.saturating_sub(bw + margin))))?;
    }
    queue!(w, ResetColor)?;
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
    let bw = block_width(width, block_view);
    let margin = block_view.horizontal_margin;

    if selected {
        let content = with_margin(&framed_text(text, bw, block_view.body_padding), block_view);
        queue!(w, SetForegroundColor(Theme::BODY_SELECTED_FG))?;
        queue!(w, SetBackgroundColor(Theme::BODY_SELECTED_BG))?;
        queue!(w, Print(pad_to_width(&content, width)))?;
        queue!(w, ResetColor)?;
    } else if bw < 4 {
        queue!(w, Print(truncate_to_width(text, bw)))?;
    } else {
        let inner_w = bw - 2;
        let padding = " ".repeat(block_view.body_padding);
        let body = truncate_to_width(&format!("{padding}{text}"), inner_w);
        let fill = inner_w.saturating_sub(UnicodeWidthStr::width(body.as_str()));
        queue!(w, SetForegroundColor(Theme::BORDER_NORMAL_FG))?;
        queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
        queue!(w, ResetColor)?;
        queue!(w, Print(format!("{body}{}", " ".repeat(fill))))?;
        queue!(w, SetForegroundColor(Theme::BORDER_NORMAL_FG))?;
        queue!(w, Print("│"))?;
        queue!(w, ResetColor)?;
    }

    Ok(())
}

fn render_styled_framed_text<W: Write>(
    w: &mut W,
    styled: &StyledText,
    plain_text: &str,
    bg: Option<crossterm::style::Color>,
    border_fg: crossterm::style::Color,
    width: usize,
    layout: &BlockLayoutConfig,
    block_view: &BlockViewConfig,
) -> io::Result<()> {
    let block_w = block_width(width, block_view);
    if block_w < 2 {
        return Ok(());
    }
    let margin = block_view.horizontal_margin;
    let padding = block_view.body_padding;
    let inner_w = block_w - 2; // subtract two │ chars
    let content_w = inner_w.saturating_sub(padding);

    let clipped = truncate_styled_to_width(styled, content_w);
    let used = styled_width(&clipped);
    let fill = content_w.saturating_sub(used);
    let pad_str = " ".repeat(padding);

    if let Some(bg) = bg {
        // ANSI fg colors preserved; bg applied to whole line.
        // After each span, reset attrs but restore bg so it persists across spans.
        queue!(w, SetBackgroundColor(bg))?;
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
        queue!(w, Print(&pad_str))?;
        for span in &clipped.spans {
            apply_span_style(w, &span.style)?;
            queue!(w, Print(&span.text))?;
            queue!(w, SetAttribute(Attribute::Reset))?;
            queue!(w, SetBackgroundColor(bg))?;
        }
        queue!(w, Print(" ".repeat(fill)))?;
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print("│"))?;
        queue!(w, Print(" ".repeat(width.saturating_sub(block_w + margin))))?;
        queue!(w, ResetColor)?;
        return Ok(());
    }

    // Left margin + border
    queue!(w, SetForegroundColor(border_fg))?;
    queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
    queue!(w, ResetColor)?;
    queue!(w, Print(&pad_str))?;

    // Styled spans
    for span in &clipped.spans {
        apply_span_style(w, &span.style)?;
        queue!(w, Print(&span.text))?;
        reset_span_style(w)?;
    }

    // Fill + right border
    queue!(w, Print(" ".repeat(fill)))?;
    queue!(w, SetForegroundColor(border_fg))?;
    queue!(w, Print("│"))?;
    queue!(w, ResetColor)?;

    Ok(())
}

fn apply_span_style<W: Write>(w: &mut W, style: &TextStyle) -> io::Result<()> {
    if let Some(fg) = style.fg {
        queue!(w, SetForegroundColor(fg))?;
    }
    if let Some(bg) = style.bg {
        queue!(w, SetBackgroundColor(bg))?;
    }
    if style.bold {
        queue!(w, SetAttribute(Attribute::Bold))?;
    }
    if style.italic {
        queue!(w, SetAttribute(Attribute::Italic))?;
    }
    if style.underline {
        queue!(w, SetAttribute(Attribute::Underlined))?;
    }
    if style.reverse {
        queue!(w, SetAttribute(Attribute::Reverse))?;
    }
    Ok(())
}

fn reset_span_style<W: Write>(w: &mut W) -> io::Result<()> {
    queue!(w, SetAttribute(Attribute::Reset))
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

fn render_footer<W: Write>(w: &mut W, segments: &[FooterSegment], width: usize) -> io::Result<()> {
    queue!(w, SetBackgroundColor(Color::Reset))?;

    let mut used = 0usize;
    for (idx, seg) in segments.iter().enumerate() {
        match seg {
            FooterSegment::Spacer => {
                let tail_width: usize = segments[idx + 1..]
                    .iter()
                    .map(|s| match s {
                        FooterSegment::Plain(t)
                        | FooterSegment::Label(t)
                        | FooterSegment::Key(t) => UnicodeWidthStr::width(t.as_str()),
                        FooterSegment::Sep => 3,
                        FooterSegment::Spacer => 0,
                    })
                    .sum();
                let fill = width.saturating_sub((used + tail_width).min(width));
                if fill > 0 {
                    queue!(w, SetForegroundColor(Color::Reset))?;
                    queue!(w, Print(" ".repeat(fill)))?;
                    used += fill;
                }
            }
            FooterSegment::Plain(t) | FooterSegment::Label(t) => {
                let seg_w = UnicodeWidthStr::width(t.as_str());
                if used >= width {
                    break;
                }
                let room = width.saturating_sub(used);
                if seg_w > room {
                    queue!(w, SetForegroundColor(Theme::FOOTER_FG))?;
                    queue!(w, Print(truncate_to_width(t, room)))?;
                    used += room;
                    break;
                }
                queue!(w, SetForegroundColor(Theme::FOOTER_FG))?;
                queue!(w, Print(t))?;
                used += seg_w;
            }
            FooterSegment::Key(t) => {
                let seg_w = UnicodeWidthStr::width(t.as_str());
                if used >= width {
                    break;
                }
                let room = width.saturating_sub(used);
                if seg_w > room {
                    queue!(w, SetForegroundColor(Theme::FOOTER_KEY_FG))?;
                    queue!(w, Print(truncate_to_width(t, room)))?;
                    used += room;
                    break;
                }
                queue!(w, SetForegroundColor(Theme::FOOTER_KEY_FG))?;
                queue!(w, Print(t))?;
                used += seg_w;
            }
            FooterSegment::Sep => {
                if used >= width {
                    break;
                }
                let room = width.saturating_sub(used);
                if room < 3 {
                    break;
                }
                queue!(w, SetForegroundColor(Theme::FOOTER_SEP_FG))?;
                queue!(w, Print(" | "))?;
                used += 3;
            }
        }
    }

    let fill = width.saturating_sub(used.min(width));
    if fill > 0 {
        queue!(w, SetForegroundColor(Color::Reset))?;
        queue!(w, Print(" ".repeat(fill)))?;
    }
    queue!(w, ResetColor)?;
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
