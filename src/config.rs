#![allow(dead_code)]

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::format::CopyFormat;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub shell: ShellConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub blocks: BlocksConfig,
    #[serde(default)]
    pub history: HistoryConfig,
    #[serde(default)]
    pub block_view: BlockViewConfig,
    #[serde(default)]
    pub block_layout: BlockLayoutConfig,
    #[serde(default)]
    pub raw_programs: Vec<String>,
    #[serde(default)]
    pub tui_apps: BTreeMap<String, TuiAppConfig>,
}

impl Config {
    pub fn load() -> Result<Self> {
        load_config()
    }
}

pub fn load_config() -> Result<Config> {
    let Some(path) = config_path() else {
        return Ok(Config::default());
    };

    let source = fs::read_to_string(&path)
        .with_context(|| format!("failed to read config file {}", path.display()))?;
    let config = toml::from_str(&source)
        .with_context(|| format!("failed to parse config file {}", path.display()))?;

    Ok(config)
}

fn config_path() -> Option<PathBuf> {
    let local = Path::new("config/tide.toml");
    if local.exists() {
        return Some(local.to_path_buf());
    }

    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    if let Some(path) = xdg_config_home
        .map(|dir| dir.join("tide/config.toml"))
        .filter(|path| path.exists())
    {
        return Some(path);
    }

    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|home| home.join(".config/tide/config.toml"))
        .filter(|path| path.exists())
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shell: ShellConfig::default(),
            ui: UiConfig::default(),
            blocks: BlocksConfig::default(),
            history: HistoryConfig::default(),
            block_view: BlockViewConfig::default(),
            block_layout: BlockLayoutConfig::default(),
            raw_programs: Vec::new(),
            tui_apps: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub block_layout: BlockLayoutConfig,
    pub block_view: BlockViewConfig,
    pub max_blocks: Option<usize>,
}

pub fn build_runtime_config(config: Config) -> RuntimeConfig {
    let max_blocks = config
        .history
        .max_blocks
        .or_else(|| Some(config.blocks.max_blocks));

    RuntimeConfig {
        block_layout: config.block_layout,
        block_view: config.block_view,
        max_blocks,
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShellConfig {
    #[serde(default = "default_shell_program")]
    pub program: String,
}

impl Default for ShellConfig {
    fn default() -> Self {
        Self {
            program: default_shell_program(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UiConfig {
    #[serde(default)]
    pub transitions: TransitionConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransitionConfig {
    #[serde(default = "default_transition_enabled")]
    pub enabled: bool,
    #[serde(default = "default_transition_duration_ms")]
    pub duration_ms: u64,
    #[serde(default = "default_transition_fps")]
    pub fps: u16,
    #[serde(default = "default_transition_skip_if_fast_under_ms")]
    pub skip_if_fast_under_ms: u64,
    #[serde(default)]
    pub reduced_motion: bool,
}

impl Default for TransitionConfig {
    fn default() -> Self {
        Self {
            enabled: default_transition_enabled(),
            duration_ms: default_transition_duration_ms(),
            fps: default_transition_fps(),
            skip_if_fast_under_ms: default_transition_skip_if_fast_under_ms(),
            reduced_motion: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlocksConfig {
    #[serde(default = "default_max_blocks")]
    pub max_blocks: usize,
    #[serde(default = "default_max_output_bytes_per_block")]
    pub max_output_bytes_per_block: usize,
    #[serde(default = "default_strip_ansi_for_text")]
    pub strip_ansi_for_text: bool,
    #[serde(default)]
    pub persist_session: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HistoryConfig {
    #[serde(default = "default_history_max_blocks")]
    pub max_blocks: Option<usize>,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_blocks: default_history_max_blocks(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockViewConfig {
    #[serde(default = "default_preview_lines")]
    pub preview_lines: usize,
    #[serde(default = "default_expanded_lines")]
    pub expanded_lines: usize,
    #[serde(default = "default_follow_tail")]
    pub follow_tail: bool,
    #[serde(default = "default_block_gap")]
    pub block_gap: usize,
    #[serde(default = "default_scroll_margin_blocks")]
    pub scroll_margin_blocks: usize,
    #[serde(default = "default_scroll_margin_lines")]
    pub scroll_margin_lines: usize,
    #[serde(default)]
    pub auto_follow_on_reach_bottom: bool,
    #[serde(default = "default_horizontal_margin")]
    pub horizontal_margin: usize,
    #[serde(default = "default_body_padding")]
    pub body_padding: usize,
    #[serde(default = "default_show_footer")]
    pub show_footer: bool,
    #[serde(default)]
    pub copy_format: CopyFormat,
}

impl Default for BlockViewConfig {
    fn default() -> Self {
        Self {
            preview_lines: default_preview_lines(),
            expanded_lines: default_expanded_lines(),
            follow_tail: default_follow_tail(),
            block_gap: default_block_gap(),
            scroll_margin_blocks: default_scroll_margin_blocks(),
            scroll_margin_lines: default_scroll_margin_lines(),
            auto_follow_on_reach_bottom: false,
            horizontal_margin: default_horizontal_margin(),
            body_padding: default_body_padding(),
            show_footer: default_show_footer(),
            copy_format: CopyFormat::Plaintext,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BlockLayoutConfig {
    #[serde(default = "default_horizontal_padding")]
    pub horizontal_padding: usize,
    #[serde(default = "default_show_padding_in_plain")]
    pub show_padding_in_plain: bool,
}

impl Default for BlockLayoutConfig {
    fn default() -> Self {
        Self {
            horizontal_padding: default_horizontal_padding(),
            show_padding_in_plain: default_show_padding_in_plain(),
        }
    }
}

impl Default for BlocksConfig {
    fn default() -> Self {
        Self {
            max_blocks: default_max_blocks(),
            max_output_bytes_per_block: default_max_output_bytes_per_block(),
            strip_ansi_for_text: default_strip_ansi_for_text(),
            persist_session: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct TuiAppConfig {
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub handoff: bool,
    #[serde(default)]
    pub snapshot: Vec<String>,
    #[serde(default)]
    pub after_exit: Vec<String>,
    #[serde(default = "default_return_panel")]
    pub return_panel: String,
}

fn default_shell_program() -> String {
    "zsh".to_string()
}

fn default_transition_enabled() -> bool {
    true
}

fn default_transition_duration_ms() -> u64 {
    220
}

fn default_transition_fps() -> u16 {
    30
}

fn default_transition_skip_if_fast_under_ms() -> u64 {
    80
}

fn default_max_blocks() -> usize {
    1000
}

fn default_history_max_blocks() -> Option<usize> {
    Some(1000)
}

fn default_preview_lines() -> usize {
    4
}

fn default_expanded_lines() -> usize {
    15
}

fn default_follow_tail() -> bool {
    true
}

fn default_block_gap() -> usize {
    0
}

fn default_scroll_margin_blocks() -> usize {
    2
}

fn default_scroll_margin_lines() -> usize {
    2
}

fn default_horizontal_margin() -> usize {
    1
}

fn default_body_padding() -> usize {
    1
}

fn default_show_footer() -> bool {
    true
}

fn default_max_output_bytes_per_block() -> usize {
    1_048_576
}

fn default_horizontal_padding() -> usize {
    1
}

fn default_show_padding_in_plain() -> bool {
    true
}

fn default_strip_ansi_for_text() -> bool {
    true
}

fn default_return_panel() -> String {
    "none".to_string()
}

#[cfg(test)]
mod tests {
    use super::{BlockViewConfig, Config, build_runtime_config};
    use crate::format::CopyFormat;

    #[test]
    fn runtime_config_uses_block_layout_defaults() {
        let runtime = build_runtime_config(Config::default());

        assert_eq!(runtime.block_layout.horizontal_padding, 1);
        assert!(runtime.block_layout.show_padding_in_plain);
        assert_eq!(runtime.block_view.preview_lines, 4);
        assert_eq!(runtime.block_view.expanded_lines, 15);
        assert!(runtime.block_view.follow_tail);
        assert_eq!(runtime.block_view.block_gap, 0);
        assert_eq!(runtime.block_view.scroll_margin_blocks, 2);
        assert_eq!(runtime.block_view.scroll_margin_lines, 2);
        assert!(!runtime.block_view.auto_follow_on_reach_bottom);
        assert_eq!(runtime.block_view.horizontal_margin, 1);
        assert_eq!(runtime.block_view.body_padding, 1);
        assert!(runtime.block_view.show_footer);
        assert_eq!(runtime.max_blocks, Some(1000));
    }

    #[test]
    fn copy_format_defaults_to_plaintext() {
        let cfg = BlockViewConfig::default();
        assert_eq!(cfg.copy_format, CopyFormat::Plaintext);
    }

    #[test]
    fn copy_format_deserializes_from_toml() {
        let toml = r#"copy_format = "markdown""#;
        let cfg: BlockViewConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.copy_format, CopyFormat::Markdown);
    }

    #[test]
    fn runtime_config_ignores_legacy_raw_programs_for_passthrough() {
        let config = Config {
            raw_programs: vec!["my-tui-app".to_string()],
            ..Config::default()
        };
        let runtime = build_runtime_config(config);

        assert_eq!(runtime.block_layout.horizontal_padding, 1);
    }
}
