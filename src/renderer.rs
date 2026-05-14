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
    app::{ConfirmKind, FooterSegment},
    compositor::VisualLine,
    config::{BlockLayoutConfig, BlockViewConfig},
    theme::{CatppuccinFrappe, Theme},
};

/// Centralised visual parameters for block selection state.
/// All Group-A render functions take this instead of a bare `selected: bool`.
/// To change selection appearance, edit `selected()` / `normal()` only.
struct BlockSelectionStyle {
    border_fg: Color,
    body_bg: Option<Color>,
    /// Fallback text color for content without its own semantic color.
    text_fg: Color,
}

impl BlockSelectionStyle {
    fn selected() -> Self {
        Self {
            border_fg: Theme::BORDER_SELECTED_FG,
            body_bg: None,
            text_fg: CatppuccinFrappe::TEXT,
        }
    }
    fn normal() -> Self {
        Self {
            border_fg: Theme::BORDER_NORMAL_FG,
            body_bg: None,
            text_fg: CatppuccinFrappe::SUBTEXT1,
        }
    }
    fn visual() -> Self {
        Self {
            border_fg: Theme::VISUAL_BORDER_FG,
            body_bg: None,
            text_fg: CatppuccinFrappe::SUBTEXT1,
        }
    }
    fn from_bool(selected: bool) -> Self {
        if selected {
            Self::selected()
        } else {
            Self::normal()
        }
    }
    fn from_state(selected: bool, in_visual: bool) -> Self {
        if in_visual {
            // Visual range wins over selection — cursor block gets same YELLOW as others.
            Self::visual()
        } else if selected {
            Self::selected()
        } else {
            Self::normal()
        }
    }
}

pub struct HelpEntry {
    pub key: &'static str,
    pub desc: &'static str,
}

pub const BLOCK_HELP_ENTRIES: &[HelpEntry] = &[
    HelpEntry {
        key: "j / k",
        desc: "navigate blocks",
    },
    HelpEntry {
        key: "Ctrl-u / Ctrl-d",
        desc: "scroll half screen",
    },
    HelpEntry {
        key: "Ctrl-b / Ctrl-f",
        desc: "scroll full screen",
    },
    HelpEntry {
        key: "g / G",
        desc: "top / bottom",
    },
    HelpEntry {
        key: "Enter",
        desc: "expand / collapse",
    },
    HelpEntry {
        key: "i",
        desc: "detail view",
    },
    HelpEntry {
        key: "v",
        desc: "visual select mode",
    },
    HelpEntry {
        key: "/",
        desc: "search commands",
    },
    HelpEntry {
        key: "n / N",
        desc: "next / prev result",
    },
    HelpEntry {
        key: "f",
        desc: "toggle failed filter",
    },
    HelpEntry {
        key: "c",
        desc: "copy command",
    },
    HelpEntry {
        key: "o",
        desc: "copy output",
    },
    HelpEntry {
        key: "y",
        desc: "copy command + output",
    },
    HelpEntry {
        key: "r",
        desc: "rerun command",
    },
    HelpEntry {
        key: "d",
        desc: "delete block",
    },
    HelpEntry {
        key: "?",
        desc: "close help",
    },
    HelpEntry {
        key: "q / Esc",
        desc: "return to shell",
    },
];

pub const DETAIL_HELP_ENTRIES: &[HelpEntry] = &[
    HelpEntry {
        key: "j / k",
        desc: "scroll output",
    },
    HelpEntry {
        key: "g / G",
        desc: "top / bottom",
    },
    HelpEntry {
        key: "v / V",
        desc: "visual line select",
    },
    HelpEntry {
        key: "c",
        desc: "copy command",
    },
    HelpEntry {
        key: "o",
        desc: "copy output / selection",
    },
    HelpEntry {
        key: "y",
        desc: "copy command + output",
    },
    HelpEntry {
        key: "r",
        desc: "rerun command",
    },
    HelpEntry {
        key: "?",
        desc: "close help",
    },
    HelpEntry {
        key: "q / Esc",
        desc: "back to blocks",
    },
];

/// Enter alternate screen and hide cursor for Block/Detail view rendering.
pub fn enter_block_render<W: Write>(w: &mut W) -> io::Result<()> {
    execute!(w, EnterAlternateScreen, Hide)
}

/// Leave alternate screen, reset SGR, and show cursor when returning to Plain view.
///
/// # Ordering
///
/// `LeaveAlternateScreen` MUST come first so that `Show` is applied on the *main*
/// screen (the restored terminal state), not the alt screen that is about to be
/// discarded.
pub fn leave_block_render<W: Write>(w: &mut W, was_alt_screen: bool) -> io::Result<()> {
    if was_alt_screen {
        // Most terminals restore the SGR state of the main buffer when leaving the alternate screen.
        // The shell prompt (zle reset-prompt) will also ensure the correct style is applied.
        execute!(w, LeaveAlternateScreen, Show)
    } else {
        execute!(w, ResetColor, Show)
    }
}

/// Returns `(rendered_rows, drew_underlying)`.
/// `drew_underlying` is true when the underlying Block/Detail view was
/// re-rendered this frame (as opposed to reusing the previous frame's pixels).
/// The caller uses this to mark `HelpState::underlying_rendered = true` so
/// subsequent Help navigations can skip the underlying re-render (no flicker).
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
) -> io::Result<(usize, bool)> {
    if matches!(view.view, ViewKind::Help) {
        let already_clean = view.help.as_ref().map_or(false, |h| h.underlying_rendered);
        if already_clean {
            // Underlying view already rendered with suppressed highlights.
            // Only redraw the floating Help box to avoid full-screen flicker on j/k.
            queue!(w, Hide)?;
            render_help_overlay(w, view, cols, rows)?;
            w.flush()?;
            return Ok((last_rendered_rows, false));
        }
        // First render since Help opened: fall through to render the underlying
        // view (compositor has already suppressed selection via unsel), then
        // draw the overlay on top — all in one flush, so no flicker.
    }

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

    if matches!(view.view, ViewKind::Help) {
        // Draw overlay on top of freshly-rendered underlying view (single flush).
        queue!(w, Hide)?;
        render_help_overlay(w, view, cols, rows)?;
        w.flush()?;
        return Ok((rendered, true));
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

    if view.confirm.is_some() {
        render_confirm_overlay(w, view, cols, rows)?;
    }

    w.flush()?;
    Ok((rendered, false))
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
            in_visual,
        } => {
            let _ = block_id;
            render_framed_text(
                w,
                text,
                &BlockSelectionStyle::from_state(*selected, *in_visual),
                width,
                layout,
                block_view,
            )?;
        }
        VisualLine::BlockTopBorder {
            block_id,
            selected,
            in_visual,
            label,
            match_query,
            is_agent,
        } => {
            let _ = block_id;
            render_top_border(
                w,
                label,
                match_query,
                &BlockSelectionStyle::from_state(*selected, *in_visual),
                width,
                block_view,
                *is_agent,
            )?;
        }
        VisualLine::BlockBottomBorder {
            block_id,
            selected,
            in_visual,
            label,
            is_agent,
        } => {
            let _ = block_id;
            render_border(
                w,
                label,
                &BlockSelectionStyle::from_state(*selected, *in_visual),
                false,
                width,
                block_view,
                *is_agent,
            )?;
        }
        VisualLine::AgentSectionHeader => {
            render_agent_section_header(w, width, block_view)?;
        }
        VisualLine::AgentSectionFooter => {
            render_agent_section_footer(w, width, block_view)?;
        }
        VisualLine::BlockDetailLine {
            block_id,
            text,
            selected,
            in_visual,
            in_detail_view,
        } => {
            let _ = block_id;
            render_block_detail_line(
                w,
                text,
                &BlockSelectionStyle::from_state(*selected, *in_visual),
                *in_detail_view,
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
            in_visual,
            ..
        } => {
            let style = BlockSelectionStyle::from_state(*selected, *in_visual);
            render_styled_framed_text(
                w,
                styled,
                plain_text,
                style.body_bg,
                style.border_fg,
                width,
                layout,
                block_view,
            )?;
        }
        VisualLine::StyledDetailBodyLine {
            styled,
            plain_text,
            is_cursor,
            is_visual,
            ..
        } => {
            let bg = if *is_cursor {
                Some(Theme::CURSOR_BG)
            } else if *is_visual {
                Some(Theme::VISUAL_LINE_BG)
            } else {
                None
            };
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
        VisualLine::ReturnPanelTopBorder { label, .. } => {
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
        VisualLine::ReturnPanelBodyLine { text, kind, .. } => {
            use crate::app::ReturnPanelLineKind;
            let bw = block_width(width, block_view);
            let margin = block_view.horizontal_margin;
            let padding = block_view.body_padding;
            let inner_w = bw.saturating_sub(2);

            if *kind == ReturnPanelLineKind::Separator {
                let sep = format!("{}", "─".repeat(inner_w.saturating_sub(padding * 2)));
                queue!(w, SetForegroundColor(Theme::DETAIL_BORDER_FG))?;
                queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
                queue!(w, SetForegroundColor(CatppuccinFrappe::SURFACE2))?;
                queue!(w, Print(format!(" {sep} ")))?;
                queue!(w, SetForegroundColor(Theme::DETAIL_BORDER_FG))?;
                queue!(w, Print("│"))?;
                queue!(w, ResetColor)?;
            } else {
                let body = truncate_to_width(&format!("{}{}", " ".repeat(padding), text), inner_w);
                let fill = inner_w.saturating_sub(UnicodeWidthStr::width(body.as_str()));
                let fg = match kind {
                    ReturnPanelLineKind::Title => Theme::META_HEADER_FG,
                    ReturnPanelLineKind::Field => CatppuccinFrappe::TEXT,
                    ReturnPanelLineKind::Hint => Theme::FOOTER_FG,
                    _ => CatppuccinFrappe::SUBTEXT1,
                };
                queue!(w, SetForegroundColor(Theme::DETAIL_BORDER_FG))?;
                queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
                queue!(w, SetForegroundColor(fg))?;
                queue!(w, Print(format!("{body}{}", " ".repeat(fill))))?;
                queue!(w, SetForegroundColor(Theme::DETAIL_BORDER_FG))?;
                queue!(w, Print("│"))?;
                queue!(w, ResetColor)?;
            }
        }
        VisualLine::ReturnPanelBottomBorder { label, .. } => {
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
        VisualLine::Footer { segments } => {
            render_footer(w, segments, width)?;
        }
    }

    Ok(())
}

fn render_top_border<W: Write>(
    w: &mut W,
    label: &crate::format::TopLabel,
    match_query: &str,
    style: &BlockSelectionStyle,
    width: usize,
    block_view: &BlockViewConfig,
    is_agent: bool,
) -> io::Result<()> {
    let bw = block_width(width, block_view);
    if bw < 2 {
        return Ok(());
    }
    let inner_w = bw.saturating_sub(2);
    let margin = block_view.horizontal_margin;

    let (left, right) = if is_agent {
        ('┌', '┐')
    } else {
        ('╭', '╮')
    };
    let border_fg = style.border_fg;

    if is_agent {
        queue!(w, Print(" ".repeat(margin)))?;
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print(left))?;
        queue!(w, Print("─".repeat(inner_w)))?;
        queue!(w, Print(right))?;
        queue!(w, ResetColor)?;
        return Ok(());
    }

    let command_fg = match label.status {
        BlockStatus::Success => Theme::STATUS_OK_FG,
        BlockStatus::Failed => Theme::STATUS_FAILED_FG,
        _ => style.text_fg,
    };

    // Build content segments
    let mut segments: Vec<(crossterm::style::Color, String)> = Vec::new();
    segments.push((border_fg, format!("{left}─ ")));
    segments.push((border_fg, label.id_marker.clone()));

    if !label.command.is_empty() {
        segments.push((border_fg, "  ".to_string()));
        if !match_query.is_empty() {
            let tokens = search_tokens(match_query);
            let spans = highlight_spans(&label.command, &tokens);
            for (highlighted, part) in spans {
                let fg = if highlighted {
                    Theme::SEARCH_MATCH_FG
                } else {
                    command_fg
                };
                segments.push((fg, part.to_string()));
            }
        } else {
            segments.push((command_fg, label.command.clone()));
        }
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

    // Border line has no bg — highlight lives inside body rows only
    queue!(w, Print(" ".repeat(margin)))?;
    for (fg, text) in &segments {
        queue!(w, SetForegroundColor(*fg))?;
        queue!(w, Print(text))?;
    }
    queue!(w, SetForegroundColor(border_fg))?;
    queue!(w, Print(" "))?;
    queue!(w, Print("─".repeat(fill)))?;
    queue!(w, Print(right.to_string()))?;
    queue!(w, ResetColor)?;
    Ok(())
}

fn render_border<W: Write>(
    w: &mut W,
    label: &str,
    style: &BlockSelectionStyle,
    top: bool,
    width: usize,
    block_view: &BlockViewConfig,
    is_agent: bool,
) -> io::Result<()> {
    let (left, right) = if is_agent {
        if top { ('┌', '┐') } else { ('└', '┘') }
    } else if top {
        ('╭', '╮')
    } else {
        ('╰', '╯')
    };
    let content = with_margin(
        &titled_border(left, right, label, block_width(width, block_view)),
        block_view,
    );
    queue!(w, SetForegroundColor(style.border_fg))?;
    queue!(w, Print(content))?;
    queue!(w, ResetColor)?;
    Ok(())
}

fn render_agent_section_header<W: Write>(
    w: &mut W,
    width: usize,
    block_view: &BlockViewConfig,
) -> io::Result<()> {
    let bw = block_width(width, block_view);
    if bw < 4 {
        return Ok(());
    }
    let margin = block_view.horizontal_margin;
    let label = "Agents";
    let border = titled_border('╭', '╮', label, bw);
    queue!(w, SetForegroundColor(Theme::META_HEADER_FG))?;
    queue!(w, Print(format!("{}{border}", " ".repeat(margin))))?;
    queue!(w, ResetColor)?;
    Ok(())
}

fn render_agent_section_footer<W: Write>(
    w: &mut W,
    width: usize,
    block_view: &BlockViewConfig,
) -> io::Result<()> {
    let bw = block_width(width, block_view);
    if bw < 4 {
        return Ok(());
    }
    let margin = block_view.horizontal_margin;
    let inner = bw.saturating_sub(2);
    let border = format!("╰{}╯", "─".repeat(inner));
    queue!(w, SetForegroundColor(Theme::META_HEADER_FG))?;
    queue!(w, Print(format!("{}{border}", " ".repeat(margin))))?;
    queue!(w, ResetColor)?;
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
    style: &BlockSelectionStyle,
    in_detail_view: bool,
    width: usize,
    layout: &BlockLayoutConfig,
    block_view: &BlockViewConfig,
) -> io::Result<()> {
    // Detail View overrides border color and always has no bg
    let border_fg = if in_detail_view {
        Theme::DETAIL_BORDER_FG
    } else {
        style.border_fg
    };
    let bg = if in_detail_view { None } else { style.body_bg };

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
        return render_framed_text(w, text, style, width, layout, block_view);
    }

    let margin = block_view.horizontal_margin;
    let inner_w = bw - 2;
    let padding = block_view.body_padding;
    let pad_str = " ".repeat(padding);

    // Nerd Font icons in this terminal render as 1 cell (standard unicode_width value).
    const NERD_W: usize = 1;
    // Fixed display width for the icon + label column in metadata rows.
    // icon(1) + space(1) + "duration"(8) + pad(2) = 12
    const DETAIL_LABEL_COL: usize = 12;

    if text == "Detail" {
        let content_w = inner_w.saturating_sub(padding);
        // "󰋼 Detail " occupies NERD_W + " Detail " = 2 + 8 = 10 display cells
        let header_text = " Detail "; // space + label + trailing space
        let header_w = NERD_W + UnicodeWidthStr::width(header_text);
        let fill = content_w.saturating_sub(header_w);

        if let Some(bg) = bg {
            queue!(w, SetBackgroundColor(bg))?;
        }
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
        queue!(w, Print(&pad_str))?;
        queue!(w, SetAttribute(Attribute::Bold))?;
        queue!(w, SetForegroundColor(Theme::ICON_SECTION_FG))?;
        queue!(w, Print("󰋼"))?;
        queue!(w, SetAttribute(Attribute::Reset))?;
        if let Some(bg) = bg {
            queue!(w, SetBackgroundColor(bg))?;
        }
        queue!(w, SetForegroundColor(Theme::META_HEADER_FG))?;
        queue!(w, Print(header_text))?;
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print("─".repeat(fill)))?;
        queue!(w, Print("│"))?;
        if bg.is_some() {
            queue!(w, Print(" ".repeat(width.saturating_sub(bw + margin))))?;
        }
        queue!(w, ResetColor)?;
        return Ok(());
    }

    if let Some((label, value)) = text.split_once(": ") {
        let content_w = inner_w.saturating_sub(padding);
        // Determine icon and colors for known metadata fields.
        let field_icon: &str = match label {
            "command" => "󰘧",
            "cwd" => "󰉋",
            "status" => {
                if value.starts_with("ok") {
                    "󰄬"
                } else if value.starts_with("fail") {
                    "󰅙"
                } else {
                    "󰔟"
                }
            }
            "duration" => "󰔟",
            "actions" => "󰘳",
            _ => "",
        };

        let icon_fg = match label {
            "status" => {
                if value.starts_with("ok") {
                    Theme::STATUS_OK_FG
                } else if value.starts_with("fail") {
                    Theme::STATUS_FAILED_FG
                } else {
                    Theme::STATUS_RUNNING_FG
                }
            }
            "command" => Theme::ICON_CMD_FG,
            "cwd" => Theme::ICON_PATH_FG,
            "duration" => Theme::ICON_TIME_FG,
            "actions" => Theme::ICON_ACTION_FG,
            _ => Theme::META_LABEL_FG,
        };

        let value_fg: Option<Color> = match label {
            "status" => {
                if value.starts_with("ok") {
                    Some(Theme::STATUS_OK_FG)
                } else if value.starts_with("fail") {
                    Some(Theme::STATUS_FAILED_FG)
                } else {
                    Some(Theme::STATUS_RUNNING_FG)
                }
            }
            "cwd" => Some(Theme::META_PATH_FG),
            "duration" => Some(Theme::STATUS_RUNNING_FG),
            "capture" | "type" => Some(Theme::STATUS_RUNNING_FG),
            _ => None,
        };

        if label == "actions" {
            // Fixed label column + icon-prefixed action items with 2-space separators.
            let label_text_w = UnicodeWidthStr::width(label);
            let label_pad = DETAIL_LABEL_COL.saturating_sub(NERD_W + 1 + label_text_w);
            let actions = parse_actions(value);

            if let Some(bg) = bg {
                queue!(w, SetBackgroundColor(bg))?;
            }
            queue!(w, SetForegroundColor(border_fg))?;
            queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
            queue!(w, Print(&pad_str))?;
            queue!(w, SetForegroundColor(Theme::ICON_ACTION_FG))?;
            queue!(w, Print(field_icon))?;
            queue!(w, SetForegroundColor(Theme::META_LABEL_FG))?;
            queue!(w, Print(format!(" {label}{}", " ".repeat(label_pad))))?;

            let mut used_w = DETAIL_LABEL_COL;
            let mut first = true;
            for (key, action_text) in &actions {
                let action_icon: &str = match key.as_str() {
                    "c" => "󰆏",
                    "o" => "󰉆",
                    "y" => "󰈚",
                    "r" => "󰑓",
                    _ => "",
                };
                let aicon_w: usize = if action_icon.is_empty() {
                    0
                } else {
                    NERD_W + 1
                };
                let key_w = UnicodeWidthStr::width(key.as_str());
                let text_w = UnicodeWidthStr::width(action_text.as_str());
                let seg_w = aicon_w + key_w + 1 + text_w;
                let sep_w: usize = if first { 0 } else { 2 };
                if used_w + sep_w + seg_w > content_w {
                    break;
                }

                if !first {
                    queue!(w, Print("  "))?;
                    used_w += 2;
                }
                first = false;

                if !action_icon.is_empty() {
                    let aicon_fg = match key.as_str() {
                        "c" => Theme::ICON_CMD_FG,
                        "o" => Theme::ICON_PATH_FG,
                        _ => Theme::ICON_ACTION_FG, // y, r
                    };
                    queue!(w, SetForegroundColor(aicon_fg))?;
                    queue!(w, Print(action_icon))?;
                    queue!(w, Print(" "))?;
                    used_w += NERD_W + 1;
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
                used_w += key_w + 1 + text_w;
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

        if !field_icon.is_empty() {
            // Known metadata field: fixed icon + label column, then value.
            let label_text_w = UnicodeWidthStr::width(label);
            let label_pad = DETAIL_LABEL_COL.saturating_sub(NERD_W + 1 + label_text_w);
            let value_w = content_w.saturating_sub(DETAIL_LABEL_COL);
            let value_display = truncate_to_width(value, value_w);
            let value_display_w = UnicodeWidthStr::width(value_display.as_str());
            let fill = content_w.saturating_sub(DETAIL_LABEL_COL + value_display_w);

            if let Some(bg) = bg {
                queue!(w, SetBackgroundColor(bg))?;
            }
            queue!(w, SetForegroundColor(border_fg))?;
            queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
            queue!(w, Print(&pad_str))?;
            queue!(w, SetForegroundColor(icon_fg))?;
            queue!(w, Print(field_icon))?;
            queue!(w, SetForegroundColor(Theme::META_LABEL_FG))?;
            queue!(w, Print(format!(" {label}{}", " ".repeat(label_pad))))?;
            if let Some(fg) = value_fg {
                queue!(w, SetForegroundColor(fg))?;
            } else {
                queue!(w, ResetColor)?;
                if let Some(bg) = bg {
                    queue!(w, SetBackgroundColor(bg))?;
                }
            }
            queue!(w, Print(&value_display))?;
            queue!(w, Print(" ".repeat(fill)))?;
            queue!(w, SetForegroundColor(border_fg))?;
            queue!(w, Print("│"))?;
            if bg.is_some() {
                queue!(w, Print(" ".repeat(width.saturating_sub(bw + margin))))?;
            }
            queue!(w, ResetColor)?;
            return Ok(());
        }

        // Fallback: generic label: value rendering for unknown/plain fields.
        let label_colon = format!("{label}: ");
        let label_w = UnicodeWidthStr::width(label_colon.as_str()).min(content_w);
        let value_w = content_w.saturating_sub(label_w);
        let label_display = truncate_to_width(&label_colon, label_w);
        let value_display = truncate_to_width(value, value_w);
        let fill_base = content_w
            .saturating_sub(UnicodeWidthStr::width(label_display.as_str()))
            .saturating_sub(UnicodeWidthStr::width(value_display.as_str()));

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
    let text_fg = if in_detail_view {
        Theme::FOOTER_FG
    } else {
        style.text_fg
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
    style: &BlockSelectionStyle,
    width: usize,
    _layout: &BlockLayoutConfig,
    block_view: &BlockViewConfig,
) -> io::Result<()> {
    let bw = block_width(width, block_view);
    let margin = block_view.horizontal_margin;
    if bw < 4 {
        queue!(w, Print(truncate_to_width(text, bw)))?;
        return Ok(());
    }
    let inner_w = bw - 2;
    let padding = " ".repeat(block_view.body_padding);
    let body = truncate_to_width(&format!("{padding}{text}"), inner_w);
    let fill = inner_w.saturating_sub(UnicodeWidthStr::width(body.as_str()));
    queue!(w, SetForegroundColor(style.border_fg))?;
    queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
    queue!(w, ResetColor)?;
    if let Some(bg) = style.body_bg {
        queue!(w, SetBackgroundColor(bg))?;
    }
    queue!(w, Print(format!("{body}{}", " ".repeat(fill))))?;
    queue!(w, ResetColor)?;
    queue!(w, SetForegroundColor(style.border_fg))?;
    queue!(w, Print("│"))?;
    queue!(w, ResetColor)?;
    Ok(())
}

fn render_styled_framed_text<W: Write>(
    w: &mut W,
    styled: &StyledText,
    _plain_text: &str,
    bg: Option<crossterm::style::Color>,
    border_fg: crossterm::style::Color,
    width: usize,
    _layout: &BlockLayoutConfig,
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
        // │ printed without bg; highlight confined to content area between │ chars
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print(format!("{}│", " ".repeat(margin))))?;
        queue!(w, SetBackgroundColor(bg))?;
        queue!(w, Print(&pad_str))?;
        for span in &clipped.spans {
            apply_span_style(w, &span.style)?;
            queue!(w, Print(&span.text))?;
            queue!(w, SetAttribute(Attribute::Reset))?;
            queue!(w, SetBackgroundColor(bg))?;
        }
        queue!(w, Print(" ".repeat(fill)))?;
        queue!(w, ResetColor)?;
        queue!(w, SetForegroundColor(border_fg))?;
        queue!(w, Print("│"))?;
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

fn titled_border_centered(left: char, right: char, label: &str, width: usize) -> String {
    if width < 2 {
        return String::new();
    }

    let inner_width = width - 2;
    let label_str = truncate_to_width(&format!(" {label} "), inner_width);
    let label_w = UnicodeWidthStr::width(label_str.as_str());
    let remaining = inner_width.saturating_sub(label_w);
    let left_fill = remaining / 2;
    let right_fill = remaining - left_fill;
    format!(
        "{left}{}{label_str}{}{right}",
        "─".repeat(left_fill),
        "─".repeat(right_fill)
    )
}

fn pad_to_width(value: &str, width: usize) -> String {
    let value = truncate_to_width(value, width);
    let fill = width.saturating_sub(UnicodeWidthStr::width(value.as_str()));
    format!("{value}{}", " ".repeat(fill))
}

fn render_help_overlay<W: Write>(
    w: &mut W,
    view: &crate::app::ViewState,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    use crate::app::ViewKind;

    let help = match &view.help {
        Some(h) => h,
        None => return Ok(()),
    };
    let entries = match &help.return_view {
        ViewKind::Detail => DETAIL_HELP_ENTRIES,
        _ => BLOCK_HELP_ENTRIES,
    };
    let n = entries.len();

    let box_w = 56_usize.min(cols as usize - 4).max(20);
    let inner_w = box_w - 2;
    let key_area = 20_usize;
    let desc_w = inner_w.saturating_sub(key_area);

    let visible_rows = n.min((rows as usize).saturating_sub(5));
    let box_h = visible_rows + 3; // top border + entries + footer row + bottom border

    let start_col = ((cols as usize).saturating_sub(box_w)) / 2;
    let start_row = ((rows as usize).saturating_sub(box_h)) / 2;

    let scroll = help.scroll;

    // Top border with title
    queue!(w, MoveTo(start_col as u16, start_row as u16))?;
    queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
    queue!(w, SetBackgroundColor(Color::Reset))?;
    queue!(
        w,
        Print(titled_border_centered('╭', '╮', "Keybindings", box_w))
    )?;
    queue!(w, ResetColor)?;

    // Entry rows
    for vis_i in 0..visible_rows {
        let entry_i = scroll + vis_i;
        let screen_row = start_row + 1 + vis_i;
        if screen_row >= rows as usize || entry_i >= n {
            break;
        }
        let entry = &entries[entry_i];
        let is_sel = entry_i == help.cursor;

        queue!(w, MoveTo(start_col as u16, screen_row as u16))?;

        queue!(w, SetBackgroundColor(Color::Reset))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, Print("│"))?;

        if is_sel {
            queue!(w, SetBackgroundColor(Theme::HELP_SEL_BG))?;
        }

        let key_inner = key_area - 4;
        let key_truncated = truncate_to_width(entry.key, key_inner);
        let key_pad = key_inner.saturating_sub(UnicodeWidthStr::width(key_truncated.as_str()));
        let key_str = format!("  {}{}  ", " ".repeat(key_pad), key_truncated);
        let key_fg = if is_sel {
            Theme::HELP_SEL_FG
        } else {
            Theme::HELP_KEY_FG
        };
        queue!(w, SetForegroundColor(key_fg))?;
        queue!(w, Print(&key_str))?;

        let text_fg = if is_sel {
            Theme::HELP_SEL_FG
        } else {
            Theme::HELP_TEXT_FG
        };
        queue!(w, SetForegroundColor(text_fg))?;

        let desc = truncate_to_width(entry.desc, desc_w);
        let fill = desc_w.saturating_sub(UnicodeWidthStr::width(desc.as_str()));
        queue!(w, Print(&desc))?;
        queue!(w, Print(" ".repeat(fill)))?;

        queue!(w, ResetColor)?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, Print("│"))?;
        queue!(w, ResetColor)?;
    }

    // Footer row: counter
    let footer_row = start_row + 1 + visible_rows;
    if footer_row < rows as usize {
        let counter = format!("{} of {}", help.cursor + 1, n);
        let counter_w = UnicodeWidthStr::width(counter.as_str());
        let fill = inner_w.saturating_sub(counter_w);
        queue!(w, MoveTo(start_col as u16, footer_row as u16))?;
        queue!(w, SetBackgroundColor(Color::Reset))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, Print("│"))?;
        queue!(w, SetForegroundColor(Theme::HELP_DIM_FG))?;
        queue!(w, Print(" ".repeat(fill)))?;
        queue!(w, Print(&counter))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, Print("│"))?;
        queue!(w, ResetColor)?;
    }

    // Bottom border
    let bottom_row = start_row + 2 + visible_rows;
    if bottom_row < rows as usize {
        queue!(w, MoveTo(start_col as u16, bottom_row as u16))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, SetBackgroundColor(Color::Reset))?;
        queue!(w, Print(format!("╰{}╯", "─".repeat(inner_w))))?;
        queue!(w, ResetColor)?;
    }

    Ok(())
}

fn render_confirm_overlay<W: Write>(
    w: &mut W,
    view: &crate::app::ViewState,
    cols: u16,
    rows: u16,
) -> io::Result<()> {
    let confirm = match &view.confirm {
        Some(c) => c,
        None => return Ok(()),
    };

    let message = match &confirm.kind {
        ConfirmKind::DeleteBlock => format!(
            "Delete block [{}]?",
            confirm.block_ids.first().map(|id| id.0).unwrap_or(0)
        ),
        ConfirmKind::DeleteBlocks => format!("Delete [{}] blocks?", confirm.block_ids.len()),
        ConfirmKind::RerunBlocks => format!("Rerun [{}] commands?", confirm.block_ids.len()),
    };
    let hint = "This cannot be undone.";

    let box_w = 44_usize.min(cols as usize - 4).max(24);
    let inner_w = box_w - 2;

    // 6 rows: top border + message + hint + blank + divider + actions + bottom border
    let box_h = 7_usize;
    let start_col = ((cols as usize).saturating_sub(box_w)) / 2;
    let start_row = ((rows as usize).saturating_sub(box_h)) / 2;

    // Top border
    queue!(w, MoveTo(start_col as u16, start_row as u16))?;
    queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
    queue!(w, SetBackgroundColor(Color::Reset))?;
    queue!(w, Print(titled_border_centered('╭', '╮', "Confirm", box_w)))?;
    queue!(w, ResetColor)?;

    // Message row
    {
        let row = start_row + 1;
        let text = truncate_to_width(&message, inner_w);
        let fill = inner_w.saturating_sub(UnicodeWidthStr::width(text.as_str()));
        queue!(w, MoveTo(start_col as u16, row as u16))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, SetBackgroundColor(Color::Reset))?;
        queue!(w, Print("│"))?;
        queue!(w, SetForegroundColor(CatppuccinFrappe::TEXT))?;
        queue!(w, Print(format!(" {text}")))?;
        queue!(w, Print(" ".repeat(fill.saturating_sub(1))))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, Print("│"))?;
        queue!(w, ResetColor)?;
    }

    // Hint row
    {
        let row = start_row + 2;
        let text = truncate_to_width(hint, inner_w);
        let fill = inner_w.saturating_sub(UnicodeWidthStr::width(text.as_str()));
        queue!(w, MoveTo(start_col as u16, row as u16))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, SetBackgroundColor(Color::Reset))?;
        queue!(w, Print("│"))?;
        queue!(w, SetForegroundColor(Theme::HELP_DIM_FG))?;
        queue!(w, Print(format!(" {text}")))?;
        queue!(w, Print(" ".repeat(fill.saturating_sub(1))))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, Print("│"))?;
        queue!(w, ResetColor)?;
    }

    // Blank row
    {
        let row = start_row + 3;
        queue!(w, MoveTo(start_col as u16, row as u16))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, SetBackgroundColor(Color::Reset))?;
        queue!(w, Print(format!("│{}│", " ".repeat(inner_w))))?;
        queue!(w, ResetColor)?;
    }

    // Divider
    {
        let row = start_row + 4;
        queue!(w, MoveTo(start_col as u16, row as u16))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, SetBackgroundColor(Color::Reset))?;
        queue!(w, Print(format!("├{}┤", "─".repeat(inner_w))))?;
        queue!(w, ResetColor)?;
    }

    // Actions row
    {
        let row = start_row + 5;
        let yes = "[Y]es";
        let no = "(N)o";
        let yes_w = UnicodeWidthStr::width(yes);
        let no_w = UnicodeWidthStr::width(no);
        let gap = inner_w.saturating_sub(yes_w + no_w + 2); // 1 space padding each side
        queue!(w, MoveTo(start_col as u16, row as u16))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, SetBackgroundColor(Color::Reset))?;
        queue!(w, Print("│"))?;
        queue!(w, Print(" "))?;
        queue!(w, SetForegroundColor(Theme::HELP_KEY_FG))?;
        queue!(w, Print(yes))?;
        queue!(w, Print(" ".repeat(gap)))?;
        queue!(w, SetForegroundColor(Theme::HELP_DIM_FG))?;
        queue!(w, Print(no))?;
        queue!(w, Print(" "))?;
        queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
        queue!(w, Print("│"))?;
        queue!(w, ResetColor)?;
    }

    // Bottom border
    {
        let row = start_row + 6;
        if row < rows as usize {
            queue!(w, MoveTo(start_col as u16, row as u16))?;
            queue!(w, SetForegroundColor(Theme::HELP_BORDER))?;
            queue!(w, SetBackgroundColor(Color::Reset))?;
            queue!(w, Print(format!("╰{}╯", "─".repeat(inner_w))))?;
            queue!(w, ResetColor)?;
        }
    }

    Ok(())
}

fn search_tokens(query: &str) -> Vec<String> {
    query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .collect()
}

fn highlight_spans<'a>(text: &'a str, tokens: &[String]) -> Vec<(bool, &'a str)> {
    if tokens.is_empty() || text.is_empty() {
        return vec![(false, text)];
    }

    let text_lower = text.to_lowercase();

    // Collect raw match intervals as byte ranges in text_lower
    let mut raw: Vec<(usize, usize)> = Vec::new();
    for token in tokens {
        if token.is_empty() {
            continue;
        }
        let mut pos = 0;
        while let Some(rel) = text_lower[pos..].find(token.as_str()) {
            let start = pos + rel;
            let end = start + token.len();
            raw.push((start, end));
            pos = end;
        }
    }

    if raw.is_empty() {
        return vec![(false, text)];
    }

    // Sort and merge overlapping intervals
    raw.sort_by_key(|(s, _)| *s);
    let mut merged: Vec<(usize, usize)> = Vec::new();
    for (s, e) in raw {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged.push((s, e));
    }

    // Map byte positions from text_lower to original text via char counts
    let orig: Vec<(usize, usize)> = merged
        .into_iter()
        .map(|(s, e)| {
            let sc = text_lower[..s].chars().count();
            let ec = text_lower[..e].chars().count();
            let bs = text
                .char_indices()
                .nth(sc)
                .map(|(p, _)| p)
                .unwrap_or(text.len());
            let be = text
                .char_indices()
                .nth(ec)
                .map(|(p, _)| p)
                .unwrap_or(text.len());
            (bs, be)
        })
        .collect();

    // Build spans by slicing the original text
    let mut spans: Vec<(bool, &'a str)> = Vec::new();
    let mut cursor = 0;
    for (s, e) in &orig {
        if *s > cursor {
            spans.push((false, &text[cursor..*s]));
        }
        spans.push((true, &text[*s..*e]));
        cursor = *e;
    }
    if cursor < text.len() {
        spans.push((false, &text[cursor..]));
    }

    spans
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

    #[test]
    fn highlight_spans_marks_matching_tokens() {
        let spans = highlight_spans(
            "cargo test --workspace",
            &["cargo".to_string(), "test".to_string()],
        );
        assert!(spans.iter().any(|(hi, s)| *hi && *s == "cargo"));
        assert!(spans.iter().any(|(hi, s)| *hi && *s == "test"));
        assert!(
            spans
                .iter()
                .any(|(hi, s)| !*hi && s.contains("--workspace"))
        );
    }

    #[test]
    fn highlight_spans_no_query_returns_single_span() {
        let spans = highlight_spans("cargo test", &[]);
        assert_eq!(spans.len(), 1);
        assert!(!spans[0].0);
    }

    #[test]
    fn highlight_spans_case_insensitive() {
        let spans = highlight_spans("Cargo Test", &["cargo".to_string()]);
        assert!(spans.iter().any(|(hi, _)| *hi));
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
