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
}

pub struct Compositor;

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
        let viewport_start = view.block_viewport.scroll_offset.min(blocks.len());
        let mut used_height = 0_usize;
        let height = height as usize;

        for index in viewport_start..blocks.len() {
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
            let block_height = block_lines.len();
            if used_height > 0 && used_height.saturating_add(block_height) > height {
                break;
            }
            used_height = used_height.saturating_add(block_height);
            visual_lines.extend(block_lines);
        }

        if !matches!(view.block_viewport.anchor, ViewAnchor::Top) && visual_lines.len() < height {
            let mut bottom_aligned = Vec::with_capacity(height);
            bottom_aligned.extend(std::iter::repeat_n(
                VisualLine::Empty,
                height.saturating_sub(visual_lines.len()),
            ));
            bottom_aligned.extend(visual_lines);
            return bottom_aligned;
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
            label: format!("#{} · {}", block.id, block.command),
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
                    format!("... {remaining} more lines, press Enter to expand")
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

    format!(
        "#{} · {status} · exit {exit} · {}",
        block.id,
        format_duration_ms(block.duration_ms)
    )
}
