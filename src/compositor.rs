use std::path::Path;

use crate::{
    app::{BlockId, BlockKind, CommandBlock, ViewAnchor, ViewKind, ViewState},
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
    },
    BlockTopBorder {
        block_id: BlockId,
        selected: bool,
        label: String,
    },
    BlockBottomBorder {
        block_id: BlockId,
        selected: bool,
        label: String,
    },
    BlockDetailLine {
        block_id: BlockId,
        text: String,
        selected: bool,
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
    DetailBodyLine {
        #[allow(dead_code)]
        block_id: BlockId,
        text: String,
        is_cursor: bool,
    },
    Footer {
        text: String,
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
                text: footer_text(blocks, view, flash_message),
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

        for (block_index, block_id) in view.visible.ids(blocks).iter().copied().enumerate() {
            let Some(block) = blocks.block(block_id) else {
                continue;
            };
            let start_line = lines.len();
            let block_lines = Self::build_one_block_lines(
                &shell_lines,
                block,
                view,
                block_view,
                view.selected_block == Some(block_id),
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
        width: u16,
        home: Option<&Path>,
    ) -> Vec<VisualLine> {
        let block_id = block.id;

        let block_frame_width =
            (width as usize).saturating_sub(block_view.horizontal_margin.saturating_mul(2));
        let available_label_width = block_frame_width.saturating_sub(5);

        let mut lines = Vec::new();
        lines.push(VisualLine::BlockTopBorder {
            block_id,
            selected,
            label: format::build_top_label(block, home, available_label_width),
        });

        let body_start = block.start_line.min(shell_lines.len());
        let body_end = block.end_line.min(shell_lines.len().saturating_sub(1));
        let body_empty = body_start >= shell_lines.len() || block.start_line > block.end_line;

        if block.kind == BlockKind::RawProgram {
            lines.push(VisualLine::BlockBodyLine {
                text: "interactive program; screen output was not captured".to_string(),
                block_id,
                selected,
            });
        } else if body_empty {
            lines.push(VisualLine::BlockBodyLine {
                text: "no captured text output".to_string(),
                block_id,
                selected,
            });
        } else {
            let all_body_lines = &shell_lines[body_start..=body_end];
            let expanded = view.expanded_block == Some(block_id);
            let shown = if expanded {
                block_view.expanded_lines.min(all_body_lines.len())
            } else {
                block_view.preview_lines.min(all_body_lines.len())
            };

            for line in all_body_lines.iter().take(shown) {
                lines.push(VisualLine::BlockBodyLine {
                    text: line.text.clone(),
                    block_id,
                    selected,
                });
            }

            if all_body_lines.len() > shown {
                let remaining = all_body_lines.len() - shown;
                let text = if expanded {
                    format!("... {remaining} more lines · i to inspect in Detail")
                } else {
                    format!("... {remaining} more lines, Enter to expand")
                };
                lines.push(VisualLine::BlockBodyLine {
                    text,
                    block_id,
                    selected,
                });
            }
        }

        if view.expanded_block == Some(block_id) {
            lines.extend(detail_lines(block, selected));
        }

        lines.push(VisualLine::BlockBottomBorder {
            block_id,
            selected,
            label: bottom_label(block),
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
        shell: &ShellBuffer,
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
                text: "Detail: no block selected   q back".into(),
            }];
        };

        let Some(block) = blocks.block(block_id) else {
            return vec![VisualLine::Footer {
                text: "Detail: block not found   q back".into(),
            }];
        };

        let block_frame_width =
            (width as usize).saturating_sub(block_view.horizontal_margin.saturating_mul(2));
        let available_label_width = block_frame_width.saturating_sub(5);

        let shell_lines = shell.snapshot();
        let output_lines = get_block_output_lines(block, &shell_lines);
        let total = output_lines.len();

        // inner_height = rows - top_margin(1) - top_border(1) - bottom_border(1) - footer(1)
        let inner_height = height.saturating_sub(4);
        let short_mode = inner_height == 0 || total <= inner_height;

        let cursor = view.detail_line_cursor.min(total.saturating_sub(1));
        let line_offset = view.block_viewport.line_offset;

        let mut result: Vec<VisualLine> = Vec::with_capacity(height);

        if short_mode {
            let frame_height = 2 + output_lines.len(); // top_border + body + bottom_border
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
            for (i, text) in output_lines.iter().enumerate() {
                result.push(VisualLine::DetailBodyLine {
                    text: text.clone(),
                    block_id,
                    is_cursor: i == cursor,
                });
            }
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
            for (i, text) in output_lines[start..end].iter().enumerate() {
                let abs = start + i;
                result.push(VisualLine::DetailBodyLine {
                    text: text.clone(),
                    block_id,
                    is_cursor: abs == cursor,
                });
            }
            result.push(VisualLine::DetailBottomBorder {
                block_id,
                label: bottom_label(block),
            });
        }

        result.push(VisualLine::Footer {
            text: detail_footer_text(block, view, total, inner_height, flash_message),
        });

        result
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
        Self::build_one_block_lines(shell_lines, block, view, block_view, selected, 0, None).len()
    }
}

fn get_block_output_lines(
    block: &CommandBlock,
    shell_lines: &[crate::buffer::ShellLine],
) -> Vec<String> {
    if block.kind == BlockKind::RawProgram {
        return vec!["interactive program; screen output was not captured".into()];
    }
    let start = block.start_line.min(shell_lines.len());
    let end = block.end_line.min(shell_lines.len().saturating_sub(1));
    if start >= shell_lines.len() || block.start_line > block.end_line {
        return vec!["no captured text output".into()];
    }
    shell_lines[start..=end]
        .iter()
        .map(|l| l.text.clone())
        .collect()
}

fn detail_footer_text(
    block: &CommandBlock,
    view: &ViewState,
    total_lines: usize,
    inner_height: usize,
    flash_message: Option<&str>,
) -> String {
    if let Some(msg) = flash_message {
        return msg.to_string();
    }
    let id = block.id;
    if total_lines <= inner_height {
        format!("Detail #{id}   q back   yc cmd   yo output   yb block")
    } else {
        let cursor_display = view
            .detail_line_cursor
            .saturating_add(1)
            .min(total_lines.max(1));
        format!(
            "Detail #{id}   ↑↓ scroll   g/G top/bottom   q back   line {cursor_display}/{total_lines}"
        )
    }
}

fn detail_lines(block: &CommandBlock, selected: bool) -> Vec<VisualLine> {
    let block_id = block.id;
    let exit = block
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "-".to_string());
    let status = match block.status {
        crate::app::BlockStatus::Running => "running",
        crate::app::BlockStatus::Success => "ok",
        crate::app::BlockStatus::Failed => "failed",
        crate::app::BlockStatus::Interrupted => "cancelled",
        crate::app::BlockStatus::Unknown => "unknown",
    };

    let mut lines = vec![
        String::new(),
        "Detail".to_string(),
        format!("command: {}", block.command),
        format!("cwd: {}", block.cwd.display()),
        format!("exit code: {exit}"),
        format!("duration: {}", format_duration_ms(block.duration_ms)),
        format!("status: {status}"),
        String::new(),
    ];

    if block.output_truncated {
        lines.push("capture: output was truncated".to_string());
        lines.push(String::new());
    }

    if block.kind == BlockKind::RawProgram {
        lines.extend([
            "type: interactive program".to_string(),
            "capture: no linear text output was captured for this block.".to_string(),
            "actions:  y copy output   Y copy command   r rerun".to_string(),
        ]);
    } else {
        lines.push("actions:  y copy output   Y copy command   r rerun".to_string());
    }

    lines
        .into_iter()
        .map(|text| VisualLine::BlockDetailLine {
            block_id,
            text,
            selected,
        })
        .collect()
}

fn bottom_label(block: &CommandBlock) -> String {
    let status = match block.status {
        crate::app::BlockStatus::Running => "running",
        crate::app::BlockStatus::Success => "ok",
        crate::app::BlockStatus::Failed => "failed",
        crate::app::BlockStatus::Interrupted => "cancelled",
        crate::app::BlockStatus::Unknown => "unknown",
    };
    let exit = block
        .exit_code
        .map(|code| code.to_string())
        .unwrap_or_else(|| "-".to_string());

    let status = if block.kind == BlockKind::RawProgram {
        "raw"
    } else {
        status
    };
    let truncated = if block.output_truncated {
        " · truncated"
    } else {
        ""
    };
    format!(
        "{status} · {exit} · {}{}",
        format_duration_ms(block.duration_ms),
        truncated
    )
}

fn footer_text(blocks: &BlockStore, view: &ViewState, flash_message: Option<&str>) -> String {
    if let Some(buf) = &view.search_buffer {
        return format!("/{buf}\u{258c}  Enter apply \u{b7} Esc cancel");
    }

    if let Some(msg) = flash_message {
        return msg.to_string();
    }

    let visible_count = view.visible.len(blocks);
    let total_count = blocks.len();
    let current = if visible_count == 0 {
        0
    } else {
        view.block_viewport
            .selected_index
            .min(visible_count.saturating_sub(1))
            + 1
    };

    let mut tags = Vec::new();
    if !view.filter.command_query.is_empty() {
        tags.push(format!("\"{}\"", view.filter.command_query));
    }
    if view.filter.failed_only {
        tags.push("failed".to_string());
    }

    let count = if view.filter.is_active() {
        let tag_str = tags.join(" \u{b7} ");
        format!("#{current}/{visible_count} of {total_count} \u{b7} {tag_str}")
    } else {
        format!("#{current}/{total_count}")
    };

    let search_hint = if view.filter.is_active() {
        " / new search"
    } else {
        " / search"
    };

    format!("Block {count}{search_hint} f filter  j/k move  Enter expand  i detail  g/G  q quit")
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
        assert!(visual.iter().any(|line| matches!(line, VisualLine::BlockBodyLine { text, .. } if text == "interactive program; screen output was not captured")));
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
            Some(VisualLine::BlockBodyLine { .. } | VisualLine::BlockBottomBorder { .. })
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
    fn detail_actions_shows_copy_keys() {
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
        let actions_line = detail_texts.iter().find(|t| t.contains("actions:"));
        assert!(actions_line.is_some(), "detail should contain actions line");
        let actions = actions_line.unwrap();
        assert!(
            actions.contains("y copy output"),
            "actions should show y copy output, got: {actions}"
        );
        assert!(
            actions.contains("Y copy command"),
            "actions should show Y copy command, got: {actions}"
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
            VisualLine::Footer { text } => {
                assert_eq!(text, "copied output");
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
            VisualLine::Footer { text } => {
                assert!(
                    text.starts_with("Block #"),
                    "normal footer should show block info, got: {text}"
                );
                assert!(
                    !text.contains("copied"),
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
            .filter(|l| matches!(l, VisualLine::BlockBodyLine { .. }))
            .collect();
        // expanded_lines = 15, + 1 truncation hint = 16 body-type lines
        assert_eq!(
            body_lines.len(),
            16,
            "expanded block should show expanded_lines + 1 truncation hint"
        );
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
            .filter(|l| matches!(l, VisualLine::BlockBodyLine { .. }))
            .count();
        assert_eq!(
            body_count,
            config.preview_lines + 1,
            "collapsed block should show preview_lines plus one truncation hint"
        );
    }
}
