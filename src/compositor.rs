use std::path::Path;

use unicode_width::UnicodeWidthStr;

use crate::{
    app::{
        BlockId, BlockKind, CommandBlock, FooterSegment, ReturnPanelLineKind, ViewAnchor, ViewKind,
        ViewState,
    },
    block::{BlockStore, format_duration_ms},
    buffer::ShellBuffer,
    config::{BlockLayoutConfig, BlockViewConfig},
    format,
};

#[derive(Debug, Clone)]
pub enum VisualLine {
    Empty,
    ShellText {
        text: String,
        block_id: Option<BlockId>,
    },
    BlockBodyLine {
        text: String,
        block_id: BlockId,
        selected: bool,
        in_visual: bool,
    },
    BlockTopBorder {
        block_id: BlockId,
        selected: bool,
        in_visual: bool,
        label: crate::format::TopLabel,
        match_query: String,
        is_agent: bool,
    },
    BlockBottomBorder {
        block_id: BlockId,
        selected: bool,
        in_visual: bool,
        label: String,
        is_agent: bool,
    },
    /// Separator row injected once before the first shared-agent block.
    AgentSectionHeader,
    /// Closing border injected once after the last shared-agent block.
    AgentSectionFooter,
    BlockDetailLine {
        block_id: BlockId,
        text: String,
        selected: bool,
        in_visual: bool,
        in_detail_view: bool,
    },
    DetailTopBorder {
        #[allow(dead_code)]
        block_id: BlockId,
        label: String,
    },
    DetailBottomBorder {
        #[allow(dead_code)]
        block_id: BlockId,
        label: String,
    },
    StyledDetailBodyLine {
        #[allow(dead_code)]
        block_id: BlockId,
        styled: crate::ansi::StyledText,
        plain_text: String,
        is_cursor: bool,
        is_visual: bool,
    },
    StyledBlockBodyLine {
        block_id: BlockId,
        styled: crate::ansi::StyledText,
        plain_text: String,
        selected: bool,
        in_visual: bool,
    },
    ReturnPanelTopBorder {
        #[allow(dead_code)]
        block_id: BlockId,
        label: String,
    },
    ReturnPanelBodyLine {
        #[allow(dead_code)]
        block_id: BlockId,
        text: String,
        kind: crate::app::ReturnPanelLineKind,
    },
    ReturnPanelBottomBorder {
        #[allow(dead_code)]
        block_id: BlockId,
        label: String,
    },
    Footer {
        segments: Vec<FooterSegment>,
    },
}

pub struct Compositor;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct VisibleBlockRange {
    pub start: usize,
    pub end: usize,
    pub top_padding_lines: usize,
}

#[derive(Debug, Clone)]
pub struct VisualLayout {
    pub lines: Vec<VisualLine>,
    pub spans: Vec<BlockVisualSpan>,
    pub total_height: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockVisualSpan {
    pub block_id: BlockId,
    pub block_index: usize,
    pub start_line: usize,
    pub end_line: usize,
}

impl VisualLayout {
    pub fn span_for_block_index(&self, index: usize) -> Option<&BlockVisualSpan> {
        self.spans.iter().find(|span| span.block_index == index)
    }

    #[allow(dead_code)]
    pub fn block_index_at_line(&self, line: usize) -> Option<usize> {
        if let Some(span) = self
            .spans
            .iter()
            .find(|span| line >= span.start_line && line < span.end_line)
        {
            return Some(span.block_index);
        }

        self.spans
            .iter()
            .rev()
            .find(|span| span.start_line <= line)
            .or_else(|| self.spans.first())
            .map(|span| span.block_index)
    }
}

impl Compositor {
    pub fn build_visual_lines(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        _width: u16,
        height: u16,
        _layout: &BlockLayoutConfig,
        block_view: &BlockViewConfig,
        flash_message: Option<&str>,
        home: Option<&Path>,
    ) -> Vec<VisualLine> {
        match view.view {
            ViewKind::Plain | ViewKind::RawProgram => shell
                .snapshot()
                .into_iter()
                .map(|line| VisualLine::ShellText {
                    text: line.text,
                    block_id: line.block_id,
                })
                .collect(),
            ViewKind::Blocks => Self::build_block_lines(
                shell,
                blocks,
                view,
                height,
                _width,
                block_view,
                flash_message,
                home,
            ),
            ViewKind::Detail => Self::build_detail_lines(
                shell,
                blocks,
                view,
                _width,
                height,
                block_view,
                flash_message,
                home,
            ),
            ViewKind::Help => {
                let mut unsel = view.clone();
                unsel.selected_block = None;
                let return_view = view
                    .help
                    .as_ref()
                    .map(|h| &h.return_view)
                    .unwrap_or(&ViewKind::Blocks);
                match return_view {
                    ViewKind::Blocks => {
                        unsel.expanded_block = None;
                        Self::build_block_lines(
                            shell,
                            blocks,
                            &unsel,
                            height,
                            _width,
                            block_view,
                            flash_message,
                            home,
                        )
                    }
                    ViewKind::Detail => Self::build_detail_lines(
                        shell,
                        blocks,
                        &unsel,
                        _width,
                        height,
                        block_view,
                        flash_message,
                        home,
                    ),
                    _ => vec![],
                }
            }
            ViewKind::ReturnPanel => {
                Self::build_return_panel_lines(blocks, view, _width, height, block_view)
            }
            ViewKind::Agent => Vec::new(),
        }
    }

    fn build_block_lines(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        height: u16,
        width: u16,
        block_view: &BlockViewConfig,
        flash_message: Option<&str>,
        home: Option<&Path>,
    ) -> Vec<VisualLine> {
        let height = height as usize;
        let content_height = height.saturating_sub(usize::from(block_view.show_footer));
        let layout = Self::build_visual_layout(shell, blocks, view, width, block_view, home);
        let mut visual_lines = Self::slice_visible_lines(&layout, view, content_height);

        if block_view.show_footer {
            visual_lines.push(VisualLine::Footer {
                segments: footer_segments(blocks, view, flash_message),
            });
        }
        visual_lines
    }

    pub fn build_visual_layout(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        width: u16,
        block_view: &BlockViewConfig,
        home: Option<&Path>,
    ) -> VisualLayout {
        let shell_lines = shell.snapshot();
        let mut lines = Vec::new();
        let mut spans = Vec::new();

        // Compute visual selection range (block indices, inclusive) once before the loop.
        let visual_range: Option<(usize, usize)> = view.visual_anchor.and_then(|anchor| {
            let a_idx = view.visible.index_of(blocks, anchor)?;
            let b_idx = view.visible.index_of(blocks, view.selected_block?)?;
            Some((a_idx.min(b_idx), a_idx.max(b_idx)))
        });

        let mut agent_section_started = false;
        for (block_index, block_id) in view.visible.ids(blocks).iter().copied().enumerate() {
            let Some(block) = blocks.block(block_id) else {
                continue;
            };

            if block.agent_ref.is_some() && !agent_section_started {
                agent_section_started = true;
                lines.push(VisualLine::AgentSectionHeader);
                // No empty line — header flows directly into the first block.
            }

            let start_line = lines.len();
            let in_visual =
                visual_range.map_or(false, |(lo, hi)| block_index >= lo && block_index <= hi);
            let block_lines = Self::build_one_block_lines(
                &shell_lines,
                block,
                view,
                block_view,
                view.selected_block == Some(block_id),
                in_visual,
                width,
                home,
            );
            lines.extend(block_lines);
            spans.push(BlockVisualSpan {
                block_id,
                block_index,
                start_line,
                end_line: lines.len(),
            });
        }

        if agent_section_started {
            lines.push(VisualLine::AgentSectionFooter);
        }

        let total_height = lines.len();
        VisualLayout {
            lines,
            spans,
            total_height,
        }
    }

    pub fn slice_visible_lines(
        layout: &VisualLayout,
        view: &ViewState,
        content_height: usize,
    ) -> Vec<VisualLine> {
        if content_height == 0 {
            return Vec::new();
        }

        // When all content fits in one screen, no scrolling is needed.
        // Anchor controls vertical placement: Top → top-aligned, everything
        // else (Tail, Manual) → bottom-aligned.  j/k only change the
        // selected block, never the visual offset.
        if layout.total_height <= content_height {
            let top_padding = if matches!(view.block_viewport.anchor, ViewAnchor::Top) {
                0
            } else {
                content_height.saturating_sub(layout.total_height)
            };
            let mut out: Vec<VisualLine> =
                std::iter::repeat_n(VisualLine::Empty, top_padding).collect();
            out.extend(layout.lines.iter().cloned());
            while out.len() < content_height {
                out.push(VisualLine::Empty);
            }
            return out;
        }

        let max_offset = layout.total_height.saturating_sub(content_height);
        let start = view.block_viewport.line_offset.min(max_offset);
        let end = start
            .saturating_add(content_height)
            .min(layout.total_height);
        let mut lines = layout.lines[start..end].to_vec();
        if lines.len() < content_height {
            let top_padding = if matches!(view.block_viewport.anchor, ViewAnchor::Tail) {
                content_height.saturating_sub(lines.len())
            } else {
                0
            };
            if top_padding > 0 {
                let mut padded = Vec::with_capacity(content_height);
                padded.extend(std::iter::repeat_n(VisualLine::Empty, top_padding));
                padded.extend(lines);
                lines = padded;
            }
            lines.extend(std::iter::repeat_n(
                VisualLine::Empty,
                content_height.saturating_sub(lines.len()),
            ));
        }
        lines
    }

    fn build_one_block_lines(
        shell_lines: &[crate::buffer::ShellLine],
        block: &CommandBlock,
        view: &ViewState,
        block_view: &BlockViewConfig,
        selected: bool,
        in_visual: bool,
        width: u16,
        home: Option<&Path>,
    ) -> Vec<VisualLine> {
        let block_id = block.id;

        let block_frame_width =
            (width as usize).saturating_sub(block_view.horizontal_margin.saturating_mul(2));
        let available_label_width = block_frame_width.saturating_sub(5);

        let is_agent = block.agent_ref.is_some();
        let mut lines = Vec::new();
        lines.push(VisualLine::BlockTopBorder {
            block_id,
            selected,
            in_visual,
            label: format::build_top_label_parts(block, home, available_label_width),
            match_query: view
                .search_buffer
                .as_deref()
                .unwrap_or(&view.filter.command_query)
                .to_string(),
            is_agent,
        });

        if is_agent {
            // Agent block body: header line (name/cwd/status) + optional title line.
            let inner_w = block_frame_width.saturating_sub(2);
            let content_w = inner_w.saturating_sub(block_view.body_padding);
            let cwd_str = format::compact_cwd(&block.cwd, home, 32);
            let display_name = block.command.as_str();
            let left_part = format!("{display_name}  {cwd_str}");

            let status_str = block
                .live_snapshot
                .as_ref()
                .and_then(|s| s.status.display_label())
                .map(|label| format!("· {label}"))
                .unwrap_or_default();

            let left_w = UnicodeWidthStr::width(left_part.as_str());
            let right_w = UnicodeWidthStr::width(status_str.as_str());
            let fill = content_w.saturating_sub(left_w + right_w);
            let header_text = if status_str.is_empty() {
                left_part
            } else {
                format!("{left_part}{}{status_str}", " ".repeat(fill))
            };
            lines.push(VisualLine::BlockBodyLine {
                text: header_text,
                block_id,
                selected,
                in_visual,
            });

            if !block.output_text.is_empty() {
                lines.push(VisualLine::BlockBodyLine {
                    text: format!("    {}", block.output_text),
                    block_id,
                    selected,
                    in_visual,
                });
            }
        } else if matches!(block.kind, BlockKind::RawProgram | BlockKind::TuiSession) {
            lines.push(VisualLine::BlockBodyLine {
                text: "TUI session; screen output was not captured".to_string(),
                block_id,
                selected,
                in_visual,
            });
        } else if matches!(block.kind, BlockKind::Interactive) {
            lines.push(VisualLine::BlockBodyLine {
                text: "interactive REPL; session input/output was not captured".to_string(),
                block_id,
                selected,
                in_visual,
            });
        } else if block.output_raw.is_empty() {
            // No raw output yet — render shell_lines as plain text
            let body_start = block.start_line.min(shell_lines.len());
            let body_empty = body_start >= shell_lines.len() || block.start_line > block.end_line;
            if body_empty {
                lines.push(VisualLine::BlockBodyLine {
                    text: "no captured text output".to_string(),
                    block_id,
                    selected,
                    in_visual,
                });
            } else {
                let body_end = block.end_line.min(shell_lines.len().saturating_sub(1));
                let all_body_lines = &shell_lines[body_start..=body_end];
                for line in all_body_lines {
                    lines.push(VisualLine::BlockBodyLine {
                        text: line.text.clone(),
                        block_id,
                        selected,
                        in_visual,
                    });
                }
            }
        } else {
            // Parse output_raw for styled content
            let styled_lines = crate::ansi::parse_ansi_lines(&block.output_raw);
            let total = styled_lines.len();
            let expanded = view.expanded_block == Some(block_id);
            let shown = if expanded {
                block_view.expanded_lines.min(total)
            } else {
                block_view.preview_lines.min(total)
            };

            for styled in styled_lines.into_iter().take(shown) {
                let plain_text = crate::ansi::styled_to_plain(&styled);
                lines.push(VisualLine::StyledBlockBodyLine {
                    block_id,
                    styled,
                    plain_text,
                    selected,
                    in_visual,
                });
            }

            if total > shown {
                let remaining = total - shown;
                let text = if expanded {
                    format!("... {remaining} more lines · i to inspect in Detail")
                } else {
                    format!("... {remaining} more lines, Enter to expand")
                };
                lines.push(VisualLine::BlockBodyLine {
                    text,
                    block_id,
                    selected,
                    in_visual,
                });
            }
        }

        if view.expanded_block == Some(block_id) {
            lines.extend(detail_lines(block, selected, in_visual, false));
        }

        lines.push(VisualLine::BlockBottomBorder {
            block_id,
            selected,
            in_visual,
            label: bottom_label(block),
            is_agent,
        });

        for _ in 0..block_view.block_gap {
            lines.push(VisualLine::Empty);
        }

        lines
    }

    #[allow(dead_code)]
    pub fn compute_visible_range(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        height: usize,
        block_view: &BlockViewConfig,
    ) -> VisibleBlockRange {
        let layout = Self::build_visual_layout(shell, blocks, view, 0, block_view, None);
        let content_height = height.saturating_sub(usize::from(block_view.show_footer));
        if layout.spans.is_empty() {
            return VisibleBlockRange {
                start: 0,
                end: 0,
                top_padding_lines: content_height,
            };
        }
        let max_offset = layout.total_height.saturating_sub(content_height);
        let line_offset = view.block_viewport.line_offset.min(max_offset);
        let bottom = line_offset.saturating_add(content_height);
        let start = layout.block_index_at_line(line_offset).unwrap_or(0);
        let end = layout
            .block_index_at_line(bottom.saturating_sub(1))
            .unwrap_or_else(|| {
                layout
                    .spans
                    .last()
                    .map(|span| span.block_index)
                    .unwrap_or(start)
            });
        let visible_len = layout
            .total_height
            .saturating_sub(line_offset)
            .min(content_height);
        let top_padding_lines = if !matches!(view.block_viewport.anchor, ViewAnchor::Top)
            && visible_len < content_height
        {
            content_height.saturating_sub(visible_len)
        } else {
            0
        };
        VisibleBlockRange {
            start,
            end,
            top_padding_lines,
        }
    }

    pub fn compute_tail_scroll_offset(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        height: usize,
        block_view: &BlockViewConfig,
    ) -> usize {
        let layout = Self::build_visual_layout(shell, blocks, view, 0, block_view, None);
        let content_height = height.saturating_sub(usize::from(block_view.show_footer));
        layout.total_height.saturating_sub(content_height)
    }

    #[allow(dead_code)]
    pub fn compute_scroll_offset_ending_at(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        selected_index: usize,
        height: usize,
        block_view: &BlockViewConfig,
    ) -> usize {
        let layout = Self::build_visual_layout(shell, blocks, view, 0, block_view, None);
        let content_height = height.saturating_sub(usize::from(block_view.show_footer));
        let Some(span) = layout.span_for_block_index(selected_index) else {
            return view.block_viewport.line_offset;
        };
        let desired_bottom = span.end_line;
        let max_offset = layout.total_height.saturating_sub(content_height);
        desired_bottom
            .saturating_sub(content_height)
            .min(max_offset)
    }

    /// Full-screen Detail View: single block with line cursor.
    fn build_detail_lines(
        _shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        width: u16,
        height: u16,
        block_view: &BlockViewConfig,
        flash_message: Option<&str>,
        home: Option<&Path>,
    ) -> Vec<VisualLine> {
        let height = height as usize;

        let Some(block_id) = view.expanded_block else {
            return vec![VisualLine::Footer {
                segments: vec![FooterSegment::Plain("Detail: no block selected".into())],
            }];
        };

        let Some(block) = blocks.block(block_id) else {
            return vec![VisualLine::Footer {
                segments: vec![FooterSegment::Plain("Detail: block not found".into())],
            }];
        };

        let block_frame_width =
            (width as usize).saturating_sub(block_view.horizontal_margin.saturating_mul(2));
        let available_label_width = block_frame_width.saturating_sub(5);

        let styled_output_lines = get_block_styled_output_lines(block);
        let total = styled_output_lines.len();

        let meta_lines = detail_lines(block, false, false, true);
        let meta_count = meta_lines.len();

        // inner_height = rows - top_margin(1) - top_border(1) - meta_lines - bottom_border(1) - footer(1)
        let inner_height = height.saturating_sub(4).saturating_sub(meta_count);
        let short_mode = inner_height == 0 || total <= inner_height;

        let cursor = if matches!(view.view, ViewKind::Help) {
            usize::MAX // suppress cursor highlight when Help overlay is open
        } else {
            view.detail_line_cursor.min(total.saturating_sub(1))
        };
        let line_offset = view.block_viewport.line_offset;

        // Visual selection range for Detail View lines.
        let visual_range: Option<(usize, usize)> = if !matches!(view.view, ViewKind::Help) {
            view.detail_visual_anchor.map(|anchor| {
                let real_cursor = view.detail_line_cursor.min(total.saturating_sub(1));
                (anchor.min(real_cursor), anchor.max(real_cursor))
            })
        } else {
            None
        };

        let mut result: Vec<VisualLine> = Vec::with_capacity(height);

        if short_mode {
            let frame_height = 2 + styled_output_lines.len() + meta_count; // top_border + body + meta + bottom_border
            let available = height.saturating_sub(1); // minus footer
            let top_padding = available.saturating_sub(frame_height) / 2;
            let top_padding = top_padding.max(1); // always at least 1 row above
            for _ in 0..top_padding {
                result.push(VisualLine::Empty);
            }
        } else {
            result.push(VisualLine::Empty); // top margin row
        }

        result.push(VisualLine::DetailTopBorder {
            block_id,
            label: format::build_top_label(block, home, available_label_width),
        });

        if short_mode {
            for (i, styled) in styled_output_lines.iter().enumerate() {
                let plain_text = crate::ansi::styled_to_plain(styled);
                let is_visual = visual_range.map_or(false, |(lo, hi)| i >= lo && i <= hi);
                result.push(VisualLine::StyledDetailBodyLine {
                    styled: styled.clone(),
                    plain_text,
                    block_id,
                    is_cursor: i == cursor,
                    is_visual,
                });
            }
            result.extend(meta_lines);
            result.push(VisualLine::DetailBottomBorder {
                block_id,
                label: bottom_label(block),
            });
            while result.len() < height.saturating_sub(1) {
                result.push(VisualLine::Empty);
            }
        } else {
            let max_offset = total.saturating_sub(inner_height);
            let start = line_offset.min(max_offset);
            let end = (start + inner_height).min(total);
            for (i, styled) in styled_output_lines[start..end].iter().enumerate() {
                let abs = start + i;
                let plain_text = crate::ansi::styled_to_plain(styled);
                let is_visual = visual_range.map_or(false, |(lo, hi)| abs >= lo && abs <= hi);
                result.push(VisualLine::StyledDetailBodyLine {
                    styled: styled.clone(),
                    plain_text,
                    block_id,
                    is_cursor: abs == cursor,
                    is_visual,
                });
            }
            result.extend(meta_lines);
            result.push(VisualLine::DetailBottomBorder {
                block_id,
                label: bottom_label(block),
            });
        }

        result.push(VisualLine::Footer {
            segments: detail_footer_segments(block, view, total, inner_height, flash_message),
        });

        result
    }

    fn build_return_panel_lines(
        blocks: &BlockStore,
        view: &ViewState,
        width: u16,
        height: u16,
        block_view: &BlockViewConfig,
    ) -> Vec<VisualLine> {
        let Some(panel) = view.return_panel.as_ref() else {
            return build_return_panel_fallback(
                BlockId(0),
                width,
                height,
                block_view,
                "Return from TUI session",
                "Session data unavailable",
            );
        };
        let Some(block) = blocks.block(panel.block_id) else {
            return build_return_panel_fallback(
                panel.block_id,
                width,
                height,
                block_view,
                "Return from TUI session",
                "The session block is no longer available",
            );
        };
        if !matches!(block.kind, BlockKind::TuiSession) {
            return build_return_panel_fallback(
                panel.block_id,
                width,
                height,
                block_view,
                "Return from TUI session",
                "The selected block is not a TUI session",
            );
        }
        build_return_panel_content(block, panel.block_id, blocks, width, height, block_view)
    }

    #[allow(dead_code)]
    fn visual_height_for_block_from_lines(
        shell_lines: &[crate::buffer::ShellLine],
        block: &CommandBlock,
        view: &ViewState,
        block_view: &BlockViewConfig,
        selected: bool,
    ) -> usize {
        // Keep viewport math and rendered output on exactly the same code path.
        // If scrolling performance becomes an issue, cache visual heights keyed
        // by block_id, width, view mode, expanded state, and config.
        Self::build_one_block_lines(
            shell_lines,
            block,
            view,
            block_view,
            selected,
            false,
            0,
            None,
        )
        .len()
    }
}

// ─── Return Panel helpers ──────────────────────────────────────────────────

fn rp_line(block_id: BlockId, kind: ReturnPanelLineKind, text: impl Into<String>) -> VisualLine {
    VisualLine::ReturnPanelBodyLine {
        block_id,
        kind,
        text: text.into(),
    }
}

fn build_return_panel_content(
    block: &CommandBlock,
    block_id: BlockId,
    blocks: &BlockStore,
    _width: u16,
    height: u16,
    _block_view: &BlockViewConfig,
) -> Vec<VisualLine> {
    let app_name = block.app_name.as_deref().unwrap_or("TUI session");
    let title = format!("Return from {app_name}");
    let status = match block.exit_code {
        Some(0) => "ok".to_string(),
        Some(code) => format!("exit {code}"),
        None => "done".to_string(),
    };
    let dur = format_duration_ms(block.duration_ms);
    let bottom_label = format!("{status} · {dur}");

    let lines = vec![
        VisualLine::ReturnPanelTopBorder {
            block_id,
            label: title.clone(),
        },
        rp_line(block_id, ReturnPanelLineKind::Empty, ""),
        rp_line(block_id, ReturnPanelLineKind::Title, title),
        rp_line(block_id, ReturnPanelLineKind::Empty, ""),
        rp_line(
            block_id,
            ReturnPanelLineKind::Field,
            format!("command:   {}", block.command),
        ),
        rp_line(
            block_id,
            ReturnPanelLineKind::Field,
            format!(
                "exit code: {}",
                block.exit_code.map_or("unknown".into(), |c| c.to_string())
            ),
        ),
        rp_line(
            block_id,
            ReturnPanelLineKind::Field,
            format!("duration:  {}", dur),
        ),
        rp_line(block_id, ReturnPanelLineKind::Empty, ""),
        rp_line(block_id, ReturnPanelLineKind::Separator, ""),
        rp_line(
            block_id,
            ReturnPanelLineKind::Hint,
            "Press Enter to continue",
        ),
        rp_line(block_id, ReturnPanelLineKind::Empty, ""),
        VisualLine::ReturnPanelBottomBorder {
            block_id,
            label: bottom_label,
        },
        build_return_panel_footer(blocks, block_id),
    ];
    center_return_panel_lines(lines, height as usize)
}

fn build_return_panel_fallback(
    block_id: BlockId,
    _width: u16,
    height: u16,
    _block_view: &BlockViewConfig,
    title: &str,
    message: &str,
) -> Vec<VisualLine> {
    let lines = vec![
        VisualLine::ReturnPanelTopBorder {
            block_id,
            label: title.to_string(),
        },
        rp_line(block_id, ReturnPanelLineKind::Empty, ""),
        rp_line(block_id, ReturnPanelLineKind::Title, title.to_string()),
        rp_line(block_id, ReturnPanelLineKind::Empty, ""),
        rp_line(block_id, ReturnPanelLineKind::Field, message.to_string()),
        rp_line(block_id, ReturnPanelLineKind::Empty, ""),
        rp_line(
            block_id,
            ReturnPanelLineKind::Hint,
            "Press Enter to continue",
        ),
        rp_line(block_id, ReturnPanelLineKind::Empty, ""),
        VisualLine::ReturnPanelBottomBorder {
            block_id,
            label: "unavailable".to_string(),
        },
        VisualLine::Footer {
            segments: vec![FooterSegment::Label(
                "Block unavailable  Enter: continue".into(),
            )],
        },
    ];
    center_return_panel_lines(lines, height as usize)
}

fn build_return_panel_footer(blocks: &BlockStore, block_id: BlockId) -> VisualLine {
    let label = match blocks.position_of(block_id) {
        Some(index) => {
            format!("Block #{}/{}  Enter: continue", index + 1, blocks.len())
        }
        None => "Block unavailable  Enter: continue".to_string(),
    };
    VisualLine::Footer {
        segments: vec![FooterSegment::Label(label)],
    }
}

fn center_return_panel_lines(mut lines: Vec<VisualLine>, height: usize) -> Vec<VisualLine> {
    let content_height = height.saturating_sub(1);
    let footer = if lines
        .last()
        .is_some_and(|l| matches!(l, VisualLine::Footer { .. }))
    {
        lines.pop()
    } else {
        None
    };
    if lines.len() >= content_height {
        if let Some(f) = footer {
            lines.push(f);
        }
        return lines;
    }
    let top_padding = (content_height - lines.len()) / 2;
    let mut padded: Vec<VisualLine> = std::iter::repeat_n(VisualLine::Empty, top_padding).collect();
    padded.extend(lines);
    while padded.len() < content_height {
        padded.push(VisualLine::Empty);
    }
    if let Some(f) = footer {
        padded.push(f);
    }
    padded
}

fn get_block_styled_output_lines(block: &CommandBlock) -> Vec<crate::ansi::StyledText> {
    use crate::ansi::StyledText;
    if matches!(block.kind, BlockKind::RawProgram | BlockKind::TuiSession) {
        return vec![StyledText::plain(
            "TUI session; screen output was not captured",
        )];
    }
    if matches!(block.kind, BlockKind::Interactive) {
        return vec![StyledText::plain(
            "interactive REPL; session input/output was not captured",
        )];
    }
    if block.output_raw.is_empty() {
        return vec![StyledText::plain("no captured text output")];
    }
    let lines = crate::ansi::parse_ansi_lines(&block.output_raw);
    if lines.is_empty() {
        vec![StyledText::plain("no captured text output")]
    } else {
        lines
    }
}

fn detail_footer_segments(
    block: &CommandBlock,
    view: &ViewState,
    total_lines: usize,
    inner_height: usize,
    flash_message: Option<&str>,
) -> Vec<FooterSegment> {
    use FooterSegment::*;

    if let Some(msg) = flash_message {
        return vec![Plain(msg.to_string())];
    }

    let _id = block.id;
    let left = if total_lines > inner_height {
        let cursor_display = view
            .detail_line_cursor
            .saturating_add(1)
            .min(total_lines.max(1));
        Plain(format!("{cursor_display}/{total_lines}"))
    } else {
        Plain(String::new())
    };

    vec![left, Spacer, Label("Keybindings: ".into()), Key("?".into())]
}

fn detail_lines(
    block: &CommandBlock,
    selected: bool,
    in_visual: bool,
    in_detail_view: bool,
) -> Vec<VisualLine> {
    let block_id = block.id;
    let exit = block
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "-".to_string());
    let status_value = match block.status {
        crate::app::BlockStatus::Running => "running".to_string(),
        crate::app::BlockStatus::Success => "ok".to_string(),
        crate::app::BlockStatus::Failed => format!("fail · exit {exit}"),
        crate::app::BlockStatus::Interrupted => "cancelled".to_string(),
        crate::app::BlockStatus::Unknown => "unknown".to_string(),
    };

    let mut lines = vec![
        String::new(),
        "Detail".to_string(),
        format!("command: {}", block.command),
        format!("cwd: {}", block.cwd.display()),
        format!("status: {status_value}"),
        format!("duration: {}", format_duration_ms(block.duration_ms)),
        String::new(),
    ];

    if block.output_truncated {
        lines.push("capture: output was truncated".to_string());
        lines.push(String::new());
    }

    if matches!(block.kind, BlockKind::RawProgram | BlockKind::TuiSession) {
        lines.extend([
            "type: TUI session".to_string(),
            "capture: screen output was not captured for this block.".to_string(),
        ]);
    }
    if matches!(block.kind, BlockKind::Interactive) {
        lines.extend([
            "type: interactive REPL".to_string(),
            "capture: session input/output was not captured for this block.".to_string(),
        ]);
    }

    lines
        .into_iter()
        .map(|text| VisualLine::BlockDetailLine {
            block_id,
            text,
            selected,
            in_visual,
            in_detail_view,
        })
        .collect()
}

fn format_ago(t: std::time::SystemTime) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(t)
        .unwrap_or_default()
        .as_secs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

fn bottom_label(block: &CommandBlock) -> String {
    if matches!(block.kind, BlockKind::RawProgram | BlockKind::TuiSession) {
        return format!("tui · {}", format_duration_ms(block.duration_ms));
    }
    if matches!(block.kind, BlockKind::Interactive) {
        return format!("repl · {}", format_duration_ms(block.duration_ms));
    }
    let truncated = if block.output_truncated {
        " · truncated"
    } else {
        ""
    };
    let ago = block
        .finished_at
        .map(format_ago)
        .map(|s| format!(" · {s}"))
        .unwrap_or_default();
    let dur = format_duration_ms(block.duration_ms);
    match block.status {
        crate::app::BlockStatus::Running => format!("󰔟 running · {dur}{truncated}"),
        crate::app::BlockStatus::Success => format!("󰄬 ok · {dur}{truncated}{ago}"),
        crate::app::BlockStatus::Failed => {
            let exit = block
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "-".to_string());
            format!("󰅙 fail · exit {exit} · {dur}{truncated}{ago}")
        }
        crate::app::BlockStatus::Interrupted => {
            format!("󰅙 cancelled · {dur}{truncated}{ago}")
        }
        crate::app::BlockStatus::Unknown => format!("? unknown · {dur}{truncated}{ago}"),
    }
}

fn footer_segments(
    blocks: &BlockStore,
    view: &ViewState,
    flash_message: Option<&str>,
) -> Vec<FooterSegment> {
    let _ = blocks;
    use FooterSegment::*;

    if let Some(buf) = &view.search_buffer {
        return vec![
            Plain(format!("/{buf}\u{258c}")),
            Sep,
            Label("Apply: ".into()),
            Key("Enter".into()),
            Sep,
            Label("Cancel: ".into()),
            Key("Esc".into()),
        ];
    }

    if let Some(msg) = flash_message {
        return vec![Plain(msg.to_string())];
    }

    if view.filter.is_active() {
        let mut tags: Vec<String> = Vec::new();
        if !view.filter.command_query.is_empty() {
            tags.push(format!("\"{}\"", view.filter.command_query));
        }
        if view.filter.failed_only {
            tags.push("failed".to_string());
        }
        let tag_str = tags.join(" \u{b7} ");
        return vec![
            Plain(tag_str),
            Spacer,
            Label("Keybindings: ".into()),
            Key("?".into()),
        ];
    }

    vec![Spacer, Label("Keybindings: ".into()), Key("?".into())]
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::{
        app::{BlockKind, BlockStatus, ViewAnchor},
        block::BlockStore,
        buffer::ShellBuffer,
        config::BlockViewConfig,
    };

    use super::*;

    fn fixture() -> (ShellBuffer, BlockStore, ViewState, BlockViewConfig) {
        (
            ShellBuffer::new(),
            BlockStore::new(PathBuf::from("/tmp"), None, 1024 * 1024),
            ViewState::default(),
            BlockViewConfig::default(),
        )
    }

    fn add_block(
        shell: &mut ShellBuffer,
        store: &mut BlockStore,
        command: &str,
        lines: &[&str],
    ) -> BlockId {
        let start = shell.line_count();
        let id = store.start_command(command.to_string(), start, BlockKind::NormalCommand);
        for line in lines {
            shell.append(format!("{line}\n").as_bytes(), Some(id));
            store.append_output(format!("{line}\n").as_bytes());
        }
        let end = if lines.is_empty() {
            start.saturating_sub(1)
        } else {
            shell.line_count().saturating_sub(1)
        };
        store.finish_command(0, end);
        id
    }

    fn add_raw_block(shell: &ShellBuffer, store: &mut BlockStore, command: &str) -> BlockId {
        let id = store.start_command(
            command.to_string(),
            shell.line_count(),
            BlockKind::RawProgram,
        );
        store.finish_command(0, shell.line_count().saturating_sub(1));
        if let Some(block) = store.block_mut(id) {
            block.kind = BlockKind::RawProgram;
            block.status = BlockStatus::Success;
        }
        id
    }

    fn tail_view(view: &mut ViewState, store: &BlockStore) {
        view.view = ViewKind::Blocks;
        view.block_viewport.anchor = ViewAnchor::Tail;
        let selected = store.len().saturating_sub(1);
        view.block_viewport.selected_index = selected;
        view.selected_block = store.block_id_at(selected);
    }

    fn assert_lines_match_range(
        shell: &ShellBuffer,
        store: &BlockStore,
        view: &ViewState,
        config: &BlockViewConfig,
        height: usize,
    ) {
        let range = Compositor::compute_visible_range(shell, store, view, height, config);
        let visual = Compositor::build_visual_lines(
            shell,
            store,
            view,
            80,
            height as u16,
            &Default::default(),
            config,
            None,
            None,
        );

        assert!(visual.len() <= height);
        if !store.is_empty() {
            assert!(range.start <= range.end);
        }
        let content_len = visual.len() - usize::from(config.show_footer);
        assert!(content_len <= height.saturating_sub(usize::from(config.show_footer)));
    }

    #[test]
    fn visible_range_for_empty_store_is_safe() {
        let (shell, store, mut view, config) = fixture();
        view.view = ViewKind::Blocks;
        view.block_viewport.anchor = ViewAnchor::Tail;
        let range = Compositor::compute_visible_range(&shell, &store, &view, 10, &config);
        let visual = Compositor::build_visual_lines(
            &shell,
            &store,
            &view,
            80,
            10,
            &Default::default(),
            &config,
            None,
            None,
        );

        assert_eq!(range.start, 0);
        assert_eq!(range.end, 0);
        assert_eq!(range.top_padding_lines, 9);
        assert_eq!(visual.len(), 10);
    }

    #[test]
    fn single_tail_block_bottom_aligns_when_shorter_than_screen() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "echo one", &["one"]);
        tail_view(&mut view, &store);

        let range = Compositor::compute_visible_range(&shell, &store, &view, 10, &config);

        assert_eq!(range.start, 0);
        assert_eq!(range.end, 0);
        assert!(range.top_padding_lines > 0);
        assert_lines_match_range(&shell, &store, &view, &config, 10);
    }

    #[test]
    fn range_limits_visible_blocks_when_history_exceeds_screen() {
        let (mut shell, mut store, mut view, config) = fixture();
        for index in 0..6 {
            add_block(
                &mut shell,
                &mut store,
                &format!("echo {index}"),
                &[&format!("{index}")],
            );
        }
        tail_view(&mut view, &store);
        view.block_viewport.line_offset =
            Compositor::compute_tail_scroll_offset(&shell, &store, &view, 8, &config);

        let range = Compositor::compute_visible_range(&shell, &store, &view, 8, &config);

        assert!(range.start > 0);
        assert!(range.end < store.len());
        assert_eq!(range.end, store.len() - 1);
        assert_eq!(range.top_padding_lines, 0);
        assert_lines_match_range(&shell, &store, &view, &config, 8);
    }

    #[test]
    fn mixed_height_blocks_do_not_overrun_visible_range() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "short", &["one"]);
        add_block(
            &mut shell,
            &mut store,
            "long",
            &["1", "2", "3", "4", "5", "6", "7", "8"],
        );
        add_block(&mut shell, &mut store, "tail", &["tail"]);
        tail_view(&mut view, &store);
        view.block_viewport.line_offset =
            Compositor::compute_tail_scroll_offset(&shell, &store, &view, 12, &config);

        assert_lines_match_range(&shell, &store, &view, &config, 12);
    }

    #[test]
    fn raw_and_empty_blocks_have_consistent_placeholder_height() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "true", &[]);
        add_raw_block(&shell, &mut store, "vim");
        tail_view(&mut view, &store);

        let range = Compositor::compute_visible_range(&shell, &store, &view, 20, &config);
        let visual = Compositor::build_visual_lines(
            &shell,
            &store,
            &view,
            80,
            20,
            &Default::default(),
            &config,
            None,
            None,
        );

        assert_eq!(range.start, 0);
        assert!(visual.iter().any(|line| matches!(line, VisualLine::BlockBodyLine { text, .. } if text == "no captured text output")));
        assert!(visual.iter().any(|line| matches!(line, VisualLine::BlockBodyLine { text, .. } if text == "TUI session; screen output was not captured")));
        assert_lines_match_range(&shell, &store, &view, &config, 20);
    }

    #[test]
    fn detail_expansion_participates_in_visible_range_and_tail_offset() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "one", &["one"]);
        let tail = add_block(
            &mut shell,
            &mut store,
            "two",
            &["1", "2", "3", "4", "5", "6"],
        );
        tail_view(&mut view, &store);
        view.view = ViewKind::Blocks;
        view.expanded_block = Some(tail);
        view.selected_block = Some(tail);
        view.block_viewport.line_offset =
            Compositor::compute_tail_scroll_offset(&shell, &store, &view, 14, &config);

        let range = Compositor::compute_visible_range(&shell, &store, &view, 14, &config);
        let visual = Compositor::build_visual_lines(
            &shell,
            &store,
            &view,
            80,
            14,
            &Default::default(),
            &config,
            None,
            None,
        );

        assert_eq!(range.end, store.len() - 1);
        assert!(visual.iter().any(
            |line| matches!(line, VisualLine::BlockDetailLine { text, .. } if text == "Detail")
        ));
        assert!(visual.len() <= 14);
    }

    #[test]
    fn tail_offset_is_zero_when_history_is_shorter_than_screen() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "one", &["one"]);
        add_block(&mut shell, &mut store, "two", &["two"]);
        tail_view(&mut view, &store);

        let offset = Compositor::compute_tail_scroll_offset(&shell, &store, &view, 20, &config);

        assert_eq!(offset, 0);
    }

    #[test]
    fn tail_offset_points_to_recent_blocks_when_history_exceeds_screen() {
        let (mut shell, mut store, mut view, config) = fixture();
        for index in 0..8 {
            add_block(
                &mut shell,
                &mut store,
                &format!("cmd {index}"),
                &[&format!("{index}")],
            );
        }
        tail_view(&mut view, &store);

        let offset = Compositor::compute_tail_scroll_offset(&shell, &store, &view, 8, &config);

        assert!(offset > 0);
        view.block_viewport.line_offset = offset;
        let range = Compositor::compute_visible_range(&shell, &store, &view, 8, &config);
        assert_eq!(range.end, store.len() - 1);
    }

    #[test]
    fn visual_layout_spans_are_ordered_and_non_overlapping() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "a", &["a"]);
        add_block(&mut shell, &mut store, "b", &["1", "2", "3", "4"]);
        add_block(&mut shell, &mut store, "c", &["c"]);
        tail_view(&mut view, &store);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);

        assert_eq!(layout.total_height, layout.lines.len());
        assert_eq!(layout.spans.len(), 3);
        assert_eq!(layout.spans[0].start_line, 0);
        for pair in layout.spans.windows(2) {
            assert!(pair[0].end_line <= pair[1].start_line);
            assert!(pair[0].start_line < pair[0].end_line);
        }
    }

    #[test]
    fn partial_block_viewport_slices_from_middle_without_panic() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "a", &["1", "2", "3", "4"]);
        add_block(&mut shell, &mut store, "b", &["b"]);
        view.view = ViewKind::Blocks;
        view.block_viewport.anchor = ViewAnchor::Manual;
        view.block_viewport.line_offset = 2;
        view.block_viewport.selected_index = 1;
        view.selected_block = store.block_id_at(1);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        let visible = Compositor::slice_visible_lines(&layout, &view, 5);

        assert_eq!(visible.len(), 5);
        assert!(matches!(
            visible.first(),
            Some(
                VisualLine::BlockBodyLine { .. }
                    | VisualLine::StyledBlockBodyLine { .. }
                    | VisualLine::BlockBottomBorder { .. }
            )
        ));
    }

    #[test]
    fn block_index_at_line_handles_gaps_and_edges() {
        let (mut shell, mut store, mut view, mut config) = fixture();
        config.block_gap = 1;
        add_block(&mut shell, &mut store, "a", &["a"]);
        add_block(&mut shell, &mut store, "b", &["b"]);
        tail_view(&mut view, &store);
        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);

        assert_eq!(layout.block_index_at_line(0), Some(0));
        assert_eq!(
            layout.block_index_at_line(layout.total_height + 10),
            Some(1)
        );
    }

    // ─── Block gap tests ──────────────────────────────────────────────────

    #[test]
    fn block_gap_zero_emits_no_empty_lines() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "echo", &["hello"]);
        tail_view(&mut view, &store);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);

        let empty_count = layout
            .lines
            .iter()
            .filter(|l| matches!(l, VisualLine::Empty))
            .count();
        assert_eq!(empty_count, 0);
    }

    #[test]
    fn block_gap_one_emits_one_empty_line_per_block() {
        let (mut shell, mut store, mut view, mut config) = fixture();
        config.block_gap = 1;
        add_block(&mut shell, &mut store, "a", &["a1"]);
        add_block(&mut shell, &mut store, "b", &["b1"]);
        tail_view(&mut view, &store);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);

        // Count Empty lines
        let empty_indices: Vec<usize> = layout
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| matches!(l, VisualLine::Empty))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(empty_indices.len(), 2, "one gap line per block");

        // Each Empty line must be immediately after a BlockBottomBorder
        for &idx in &empty_indices {
            assert!(idx > 0, "Empty line at {idx} should not be at position 0");
            assert!(
                matches!(layout.lines[idx - 1], VisualLine::BlockBottomBorder { .. }),
                "Empty line at {idx} should follow BlockBottomBorder"
            );
        }
    }

    #[test]
    fn block_gap_two_emits_two_empty_lines_per_block() {
        let (mut shell, mut store, mut view, mut config) = fixture();
        config.block_gap = 2;
        add_block(&mut shell, &mut store, "echo", &["hello"]);
        tail_view(&mut view, &store);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);

        // Collect consecutive Empty lines at the end of the block
        let empty_runs: Vec<&VisualLine> = layout
            .lines
            .iter()
            .skip_while(|l| !matches!(l, VisualLine::BlockBottomBorder { .. }))
            .skip(1)
            .collect();
        assert_eq!(empty_runs.len(), 2, "gap=2 produces 2 Empty lines");
        assert!(
            empty_runs.iter().all(|l| matches!(l, VisualLine::Empty)),
            "both gap lines must be VisualLine::Empty"
        );
    }

    #[test]
    fn block_gap_increases_total_height() {
        let (mut shell, mut store, mut view, mut config) = fixture();
        add_block(&mut shell, &mut store, "a", &["a1"]);
        add_block(&mut shell, &mut store, "b", &["b1"]);
        tail_view(&mut view, &store);

        // Baseline: height with gap=0
        config.block_gap = 0;
        let layout0 = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        let base_height = layout0.total_height;

        // With gap=1: adds 1 per block
        config.block_gap = 1;
        let layout1 = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        assert_eq!(layout1.total_height, base_height + 2);
    }

    #[test]
    fn block_gap_empty_lines_belong_to_preceding_block_span() {
        let (mut shell, mut store, mut view, mut config) = fixture();
        config.block_gap = 1;
        add_block(&mut shell, &mut store, "a", &["a1"]);
        add_block(&mut shell, &mut store, "b", &["b1"]);
        tail_view(&mut view, &store);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);

        // Find the first BlockBottomBorder (end of block 0)
        let bottom_border_idx = layout
            .lines
            .iter()
            .position(|l| matches!(l, VisualLine::BlockBottomBorder { .. }))
            .expect("should have at least one bottom border");

        // The gap Empty line immediately follows this bottom border
        let gap_idx = bottom_border_idx + 1;
        assert!(
            matches!(layout.lines[gap_idx], VisualLine::Empty),
            "gap line at index {gap_idx} should be VisualLine::Empty"
        );

        // block_index_at_line on the gap should return block 0, not block 1
        assert_eq!(
            layout.block_index_at_line(gap_idx),
            Some(0),
            "gap after block 0 should belong to block 0's span"
        );
    }

    // --- Short-content alignment tests ---

    fn short_fixture() -> (ShellBuffer, BlockStore, ViewState, BlockViewConfig) {
        let (mut shell, mut store, mut view, config) = fixture();
        // Two single-line blocks — total visual height is well under 20.
        add_block(&mut shell, &mut store, "echo one", &["one"]);
        add_block(&mut shell, &mut store, "echo two", &["two"]);
        view.view = ViewKind::Blocks;
        (shell, store, view, config)
    }

    /// Count leading Empty lines in the visual output (top padding).
    fn count_top_padding(visible: &[VisualLine]) -> usize {
        visible
            .iter()
            .take_while(|l| matches!(l, VisualLine::Empty))
            .count()
    }

    #[test]
    fn short_content_tail_bottom_aligned() {
        let (shell, store, mut view, config) = short_fixture();
        tail_view(&mut view, &store);
        view.block_viewport.line_offset = 0;

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        let visible = Compositor::slice_visible_lines(&layout, &view, 20);

        assert_eq!(visible.len(), 20);
        let padding = count_top_padding(&visible);
        assert!(padding > 0, "Tail should bottom-align short content");
        assert_eq!(padding, 20 - layout.total_height);
    }

    #[test]
    fn short_content_manual_stays_bottom_aligned() {
        let (shell, store, mut view, config) = short_fixture();
        // Simulate pressing k: selected_index moves to 0, anchor becomes Manual.
        view.block_viewport.selected_index = 0;
        view.block_viewport.anchor = ViewAnchor::Manual;
        view.block_viewport.line_offset = 0;
        view.selected_block = store.block_id_at(0);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        let visible = Compositor::slice_visible_lines(&layout, &view, 20);

        assert_eq!(visible.len(), 20);
        let padding = count_top_padding(&visible);
        assert!(
            padding > 0,
            "Manual anchor should still bottom-align when content fits"
        );
        assert_eq!(padding, 20 - layout.total_height);
    }

    #[test]
    fn short_content_manual_after_k_stays_bottom_aligned() {
        let (shell, store, mut view, config) = short_fixture();
        // Start from Tail, move up to first block, simulate k behavior.
        tail_view(&mut view, &store);
        view.block_viewport.selected_index = 0;
        view.block_viewport.anchor = ViewAnchor::Manual;
        view.block_viewport.line_offset = 0;
        view.selected_block = store.block_id_at(0);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        let visible = Compositor::slice_visible_lines(&layout, &view, 20);

        assert_eq!(visible.len(), 20);
        let padding = count_top_padding(&visible);
        assert!(
            padding > 0,
            "k should not jump content to top when content fits"
        );
        assert_eq!(padding, 20 - layout.total_height);
    }

    #[test]
    fn short_content_top_aligned() {
        let (shell, store, mut view, config) = short_fixture();
        view.block_viewport.anchor = ViewAnchor::Top;
        view.block_viewport.line_offset = 0;

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        let visible = Compositor::slice_visible_lines(&layout, &view, 20);

        assert_eq!(visible.len(), 20);
        let padding = count_top_padding(&visible);
        assert_eq!(padding, 0, "Top anchor should not pad short content");
        // Content starts at line 0.
        assert!(matches!(visible[0], VisualLine::BlockTopBorder { .. }));
    }

    #[test]
    fn short_content_g_goes_top_g_returns_tail_bottom() {
        let (shell, store, mut view, config) = short_fixture();

        // Simulate g: Top anchor, selected_index = 0
        view.block_viewport.anchor = ViewAnchor::Top;
        view.block_viewport.selected_index = 0;
        view.block_viewport.line_offset = 0;

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        let top_visible = Compositor::slice_visible_lines(&layout, &view, 20);
        assert_eq!(count_top_padding(&top_visible), 0);

        // Simulate G: Tail anchor, selected_index = last
        view.block_viewport.anchor = ViewAnchor::Tail;
        view.block_viewport.selected_index = store.len() - 1;

        let tail_visible = Compositor::slice_visible_lines(&layout, &view, 20);
        let padding = count_top_padding(&tail_visible);
        assert!(padding > 0);
        assert_eq!(padding, 20 - layout.total_height);
    }

    #[test]
    fn truncated_hint_appears_in_bottom_label_and_detail() {
        let (mut shell, mut store, mut view, config) = fixture();
        let id = add_block(&mut shell, &mut store, "long", &["1", "2", "3"]);
        if let Some(block) = store.block_mut(id) {
            block.output_truncated = true;
        }
        tail_view(&mut view, &store);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        assert!(layout.lines.iter().any(|line| matches!(
            line,
            VisualLine::BlockBottomBorder { label, .. } if label.contains("truncated")
        )));

        view.view = ViewKind::Detail;
        view.expanded_block = Some(id);
        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        assert!(layout.lines.iter().any(|line| matches!(
            line,
            VisualLine::BlockDetailLine { text, .. } if text.contains("truncated")
        )));
    }

    #[test]
    fn detail_shows_command_and_status() {
        let (mut shell, mut store, mut view, config) = fixture();
        let id = add_block(&mut shell, &mut store, "echo hello", &["hello"]);
        view.view = ViewKind::Detail;
        view.expanded_block = Some(id);
        view.selected_block = Some(id);
        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        let detail_texts: Vec<&str> = layout
            .lines
            .iter()
            .filter_map(|line| match line {
                VisualLine::BlockDetailLine { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            detail_texts.iter().any(|t| t.contains("command:")),
            "detail should contain command line"
        );
        assert!(
            detail_texts.iter().any(|t| t.contains("status:")),
            "detail should contain status line"
        );
    }

    #[test]
    fn footer_shows_flash_message() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "echo one", &["one"]);
        view.view = ViewKind::Blocks;
        view.block_viewport.selected_index = 0;
        view.selected_block = store.block_id_at(0);

        let visible = Compositor::build_visual_lines(
            &shell,
            &store,
            &view,
            80,
            10,
            &Default::default(),
            &config,
            Some("copied output"),
            None,
        );
        let footer = visible.last().unwrap();
        match footer {
            VisualLine::Footer { segments } => {
                let flat = FooterSegment::flatten(segments);
                assert_eq!(flat, "copied output");
            }
            other => panic!("expected Footer, got {other:?}"),
        }
    }

    #[test]
    fn footer_reverts_when_no_flash() {
        let (mut shell, mut store, mut view, config) = fixture();
        add_block(&mut shell, &mut store, "echo one", &["one"]);
        view.view = ViewKind::Blocks;
        view.block_viewport.selected_index = 0;
        view.selected_block = store.block_id_at(0);

        let visible = Compositor::build_visual_lines(
            &shell,
            &store,
            &view,
            80,
            10,
            &Default::default(),
            &config,
            None,
            None,
        );
        let footer = visible.last().unwrap();
        match footer {
            VisualLine::Footer { segments } => {
                let flat = FooterSegment::flatten(segments);
                assert!(
                    flat.contains("Keybindings: ?"),
                    "normal footer should show keybindings hint, got: {flat}"
                );
                assert!(
                    !flat.contains("copied"),
                    "normal footer should not contain flash text"
                );
            }
            other => panic!("expected Footer, got {other:?}"),
        }
    }

    // --- build_detail_layout tests ---

    #[test]
    fn expanded_block_shows_all_output_lines() {
        let (mut shell, mut store, mut view, config) = fixture();
        let lines: Vec<String> = (0..50).map(|i| format!("line{i}")).collect();
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let id = add_block(&mut shell, &mut store, "long", &refs);
        view.view = ViewKind::Blocks;
        view.expanded_block = Some(id);
        view.selected_block = Some(id);

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        let body_lines: Vec<&VisualLine> = layout
            .lines
            .iter()
            .filter(|l| {
                matches!(
                    l,
                    VisualLine::BlockBodyLine { .. } | VisualLine::StyledBlockBodyLine { .. }
                )
            })
            .collect();
        // expanded_lines = 15, + 1 truncation hint = 16 body-type lines
        assert_eq!(
            body_lines.len(),
            16,
            "expanded block should show expanded_lines + 1 truncation hint"
        );
        // Truncation hint is always emitted as plain BlockBodyLine
        if let VisualLine::BlockBodyLine { text, .. } = body_lines.last().unwrap() {
            assert!(
                text.contains("more lines"),
                "last line should be truncation hint"
            );
            assert!(
                text.contains("Detail"),
                "expanded truncation hint should mention Detail"
            );
        }
    }

    #[test]
    fn collapsed_block_respects_preview_lines() {
        let (mut shell, mut store, mut view, config) = fixture();
        let lines: Vec<String> = (0..50).map(|i| format!("line{i}")).collect();
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        add_block(&mut shell, &mut store, "long", &refs);
        view.view = ViewKind::Blocks;

        let layout = Compositor::build_visual_layout(&shell, &store, &view, 80, &config, None);
        // Body lines = preview_lines actual lines + 1 truncation hint line
        let body_count = layout
            .lines
            .iter()
            .filter(|l| {
                matches!(
                    l,
                    VisualLine::BlockBodyLine { .. } | VisualLine::StyledBlockBodyLine { .. }
                )
            })
            .count();
        assert_eq!(
            body_count,
            config.preview_lines + 1,
            "collapsed block should show preview_lines plus one truncation hint"
        );
    }
}
