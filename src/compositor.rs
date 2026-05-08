use crate::{
    app::{BlockId, BlockKind, CommandBlock, ViewAnchor, ViewKind, ViewState},
    block::{BlockStore, format_duration_ms},
    buffer::ShellBuffer,
    config::{BlockLayoutConfig, BlockViewConfig},
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
    Footer {
        text: String,
    },
}

pub struct Compositor;

#[derive(Debug, Clone, Copy)]
pub struct VisibleBlockRange {
    pub start: usize,
    pub end: usize,
    pub top_padding_lines: usize,
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
            ViewKind::Blocks | ViewKind::Detail => {
                Self::build_block_lines(shell, blocks, view, height, block_view)
            }
            ViewKind::Agent => Vec::new(),
        }
    }

    fn build_block_lines(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        height: u16,
        block_view: &BlockViewConfig,
    ) -> Vec<VisualLine> {
        let shell_lines = shell.snapshot();
        let mut visual_lines = Vec::new();
        let height = height as usize;
        let footer_height = usize::from(block_view.show_footer);
        let content_height = height.saturating_sub(footer_height);
        let range = Self::compute_visible_range_from_lines(
            &shell_lines,
            blocks,
            view,
            content_height,
            block_view,
        );

        for index in range.start..=range.end {
            let Some(block_id) = blocks.block_id_at(index) else {
                continue;
            };
            let Some(block) = blocks.block(block_id) else {
                continue;
            };

            let block_lines = Self::build_one_block_lines(
                &shell_lines,
                block,
                view,
                block_view,
                view.selected_block == Some(block_id),
            );
            visual_lines.extend(block_lines);
        }

        if visual_lines.len() > content_height {
            visual_lines.truncate(content_height);
        }

        if range.top_padding_lines > 0 {
            let mut bottom_aligned = Vec::with_capacity(content_height);
            bottom_aligned.extend(std::iter::repeat_n(
                VisualLine::Empty,
                range.top_padding_lines,
            ));
            bottom_aligned.extend(visual_lines);
            visual_lines = bottom_aligned;
        }

        if block_view.show_footer {
            visual_lines.push(VisualLine::Footer {
                text: footer_text(blocks, view),
            });
        }
        visual_lines
    }

    fn build_one_block_lines(
        shell_lines: &[crate::buffer::ShellLine],
        block: &CommandBlock,
        view: &ViewState,
        block_view: &BlockViewConfig,
        selected: bool,
    ) -> Vec<VisualLine> {
        let block_id = block.id;
        let mut lines = Vec::new();
        lines.push(VisualLine::BlockTopBorder {
            block_id,
            selected,
            label: top_label(block),
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
            let expanded =
                matches!(view.view, ViewKind::Detail) && view.expanded_block == Some(block_id);
            let limit = if expanded {
                block_view.expanded_lines
            } else {
                block_view.preview_lines
            };
            let shown = limit.min(all_body_lines.len());

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
                    format!("... {remaining} more lines")
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

        if matches!(view.view, ViewKind::Detail) && view.expanded_block == Some(block_id) {
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

    pub fn compute_visible_range(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        height: usize,
        block_view: &BlockViewConfig,
    ) -> VisibleBlockRange {
        let shell_lines = shell.snapshot();
        let height = height.saturating_sub(usize::from(block_view.show_footer));
        Self::compute_visible_range_from_lines(&shell_lines, blocks, view, height, block_view)
    }

    fn compute_visible_range_from_lines(
        shell_lines: &[crate::buffer::ShellLine],
        blocks: &BlockStore,
        view: &ViewState,
        height: usize,
        block_view: &BlockViewConfig,
    ) -> VisibleBlockRange {
        if blocks.is_empty() {
            return VisibleBlockRange {
                start: 0,
                end: 0,
                top_padding_lines: height,
            };
        }

        let start = view
            .block_viewport
            .scroll_offset
            .min(blocks.len().saturating_sub(1));
        let mut used_height = 0_usize;
        let mut end = start;

        for index in start..blocks.len() {
            let Some(block_id) = blocks.block_id_at(index) else {
                break;
            };
            let Some(block) = blocks.block(block_id) else {
                break;
            };
            let block_height = Self::visual_height_for_block_from_lines(
                shell_lines,
                block,
                view,
                block_view,
                view.selected_block == Some(block_id),
            );
            if used_height > 0 && used_height.saturating_add(block_height) > height {
                break;
            }
            used_height = used_height.saturating_add(block_height);
            end = index;
        }

        let top_padding_lines =
            if !matches!(view.block_viewport.anchor, ViewAnchor::Top) && used_height < height {
                height.saturating_sub(used_height)
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
        let shell_lines = shell.snapshot();
        let height = height.saturating_sub(usize::from(block_view.show_footer));
        let mut used_height = 0_usize;
        let mut offset = blocks.len();

        while offset > 0 {
            let index = offset - 1;
            let Some(block_id) = blocks.block_id_at(index) else {
                break;
            };
            let Some(block) = blocks.block(block_id) else {
                break;
            };
            let block_height = Self::visual_height_for_block_from_lines(
                &shell_lines,
                block,
                view,
                block_view,
                view.selected_block == Some(block_id),
            );
            if used_height.saturating_add(block_height) > height {
                break;
            }
            used_height = used_height.saturating_add(block_height);
            offset -= 1;
        }

        offset
    }

    pub fn compute_scroll_offset_ending_at(
        shell: &ShellBuffer,
        blocks: &BlockStore,
        view: &ViewState,
        selected_index: usize,
        height: usize,
        block_view: &BlockViewConfig,
    ) -> usize {
        let shell_lines = shell.snapshot();
        let height = height.saturating_sub(usize::from(block_view.show_footer));
        let mut used_height = 0_usize;
        let mut offset = selected_index.saturating_add(1).min(blocks.len());

        while offset > 0 {
            let index = offset - 1;
            let Some(block_id) = blocks.block_id_at(index) else {
                break;
            };
            let Some(block) = blocks.block(block_id) else {
                break;
            };
            let block_height = Self::visual_height_for_block_from_lines(
                &shell_lines,
                block,
                view,
                block_view,
                view.selected_block == Some(block_id),
            );
            if used_height.saturating_add(block_height) > height {
                break;
            }
            used_height = used_height.saturating_add(block_height);
            offset -= 1;
        }

        offset
    }

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
        Self::build_one_block_lines(shell_lines, block, view, block_view, selected).len()
    }
}

fn top_label(block: &CommandBlock) -> String {
    let marker = match block.status {
        crate::app::BlockStatus::Failed => "  ✗",
        crate::app::BlockStatus::Running => "  …",
        _ => "",
    };
    format!("#{}  {}{}", block.id, block.command, marker)
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

    if block.kind == BlockKind::RawProgram {
        lines.extend([
            "type: interactive program".to_string(),
            "capture: no linear text output was captured for this block.".to_string(),
            "actions: rerun | copy command".to_string(),
        ]);
    } else {
        lines.push("actions: explain | fix | rerun | copy".to_string());
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
    format!(
        "{status} · {exit} · {}",
        format_duration_ms(block.duration_ms)
    )
}

fn footer_text(blocks: &BlockStore, view: &ViewState) -> String {
    let total = blocks.len();
    let current = if total == 0 {
        0
    } else {
        view.block_viewport
            .selected_index
            .min(total.saturating_sub(1))
            + 1
    };
    format!("Block #{current}/{total}  j/k move  Enter detail  g/G top/bottom  q quit")
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
        );

        assert!(visual.len() <= height);
        if !store.is_empty() {
            assert!(range.start <= range.end);
        }
        assert_eq!(
            range.top_padding_lines
                + rendered_block_lines(shell, store, view, config, range.start, range.end),
            visual.len() - usize::from(config.show_footer)
        );
        assert_eq!(
            visual.len(),
            range.top_padding_lines
                + rendered_block_lines(shell, store, view, config, range.start, range.end)
                + usize::from(config.show_footer)
        );
    }

    fn rendered_block_lines(
        shell: &ShellBuffer,
        store: &BlockStore,
        view: &ViewState,
        config: &BlockViewConfig,
        start: usize,
        end: usize,
    ) -> usize {
        if store.is_empty() {
            return 0;
        }
        let shell_lines = shell.snapshot();
        (start..=end)
            .filter_map(|index| {
                let block_id = store.block_id_at(index)?;
                let block = store.block(block_id)?;
                Some(
                    Compositor::build_one_block_lines(
                        &shell_lines,
                        block,
                        view,
                        config,
                        view.selected_block == Some(block_id),
                    )
                    .len(),
                )
            })
            .sum()
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
        view.block_viewport.scroll_offset =
            Compositor::compute_tail_scroll_offset(&shell, &store, &view, 8, &config);

        let range = Compositor::compute_visible_range(&shell, &store, &view, 8, &config);

        assert!(range.start > 0);
        assert!(range.end < store.len());
        assert_eq!(range.end, store.len() - 1);
        assert!(range.top_padding_lines > 0);
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
        view.block_viewport.scroll_offset =
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
        view.view = ViewKind::Detail;
        view.expanded_block = Some(tail);
        view.selected_block = Some(tail);
        view.block_viewport.scroll_offset =
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
        view.block_viewport.scroll_offset = offset;
        let range = Compositor::compute_visible_range(&shell, &store, &view, 8, &config);
        assert_eq!(range.end, store.len() - 1);
    }
}
