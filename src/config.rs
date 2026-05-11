#![allow(dead_code)]

use std::{
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::app::{BlockViewAction, DetailViewAction, ReturnPanelTarget};
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
    #[serde(default)]
    pub tui: TuiConfig,
    #[serde(default)]
    pub keymap: KeymapConfig,
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
            tui: TuiConfig::default(),
            keymap: KeymapConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct KeymapConfig {
    #[serde(default)]
    pub blocks: HashMap<String, String>,
    #[serde(default)]
    pub detail: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub block_layout: BlockLayoutConfig,
    pub block_view: BlockViewConfig,
    pub max_blocks: Option<usize>,
    pub resolved_block_keymap: HashMap<u8, BlockViewAction>,
    pub resolved_detail_keymap: HashMap<u8, DetailViewAction>,
    /// Extra TUI commands from user config (merged with builtins).
    pub tui_extra_commands: Vec<String>,
    /// Per-app TUI configuration from `[tui.apps]` or `[tui_apps]`.
    pub tui_apps: BTreeMap<String, TuiAppConfig>,
}

fn deserialize_block_action(s: &str) -> Option<BlockViewAction> {
    match s {
        "nav_down" => Some(BlockViewAction::NavDown),
        "nav_up" => Some(BlockViewAction::NavUp),
        "nav_top" => Some(BlockViewAction::NavTop),
        "nav_bottom" => Some(BlockViewAction::NavBottom),
        "scroll_half_down" => Some(BlockViewAction::ScrollHalfDown),
        "scroll_half_up" => Some(BlockViewAction::ScrollHalfUp),
        "scroll_full_down" => Some(BlockViewAction::ScrollFullDown),
        "scroll_full_up" => Some(BlockViewAction::ScrollFullUp),
        "expand" => Some(BlockViewAction::Expand),
        "detail_view" => Some(BlockViewAction::DetailView),
        "toggle_failed_filter" => Some(BlockViewAction::ToggleFailedFilter),
        "open_search" => Some(BlockViewAction::OpenSearch),
        "copy_command" => Some(BlockViewAction::CopyCommand),
        "copy_output" => Some(BlockViewAction::CopyOutput),
        "copy_both" => Some(BlockViewAction::CopyBoth),
        "rerun" => Some(BlockViewAction::Rerun),
        "delete" => Some(BlockViewAction::Delete),
        "visual_mode" => Some(BlockViewAction::VisualMode),
        "search_next" => Some(BlockViewAction::SearchNext),
        "search_prev" => Some(BlockViewAction::SearchPrev),
        "help" => Some(BlockViewAction::Help),
        "quit" => Some(BlockViewAction::Quit),
        _ => None,
    }
}

fn deserialize_detail_action(s: &str) -> Option<DetailViewAction> {
    match s {
        "nav_down" => Some(DetailViewAction::NavDown),
        "nav_up" => Some(DetailViewAction::NavUp),
        "nav_top" => Some(DetailViewAction::NavTop),
        "nav_bottom" => Some(DetailViewAction::NavBottom),
        "scroll_half_down" => Some(DetailViewAction::ScrollHalfDown),
        "scroll_half_up" => Some(DetailViewAction::ScrollHalfUp),
        "scroll_full_down" => Some(DetailViewAction::ScrollFullDown),
        "scroll_full_up" => Some(DetailViewAction::ScrollFullUp),
        "copy_command" => Some(DetailViewAction::CopyCommand),
        "copy_output" => Some(DetailViewAction::CopyOutput),
        "copy_both" => Some(DetailViewAction::CopyBoth),
        "rerun" => Some(DetailViewAction::Rerun),
        "visual_mode" => Some(DetailViewAction::VisualMode),
        "help" => Some(DetailViewAction::Help),
        "quit" => Some(DetailViewAction::Quit),
        _ => None,
    }
}

pub fn default_block_keymap() -> HashMap<u8, BlockViewAction> {
    let mut m = HashMap::new();
    m.insert(b'j', BlockViewAction::NavDown);
    m.insert(b'k', BlockViewAction::NavUp);
    m.insert(b'G', BlockViewAction::NavBottom);
    m.insert(b'g', BlockViewAction::NavTop);
    m.insert(0x15, BlockViewAction::ScrollHalfUp);
    m.insert(0x04, BlockViewAction::ScrollHalfDown);
    m.insert(0x02, BlockViewAction::ScrollFullUp);
    m.insert(0x06, BlockViewAction::ScrollFullDown);
    m.insert(b'\r', BlockViewAction::Expand);
    m.insert(b'\n', BlockViewAction::Expand);
    m.insert(b'i', BlockViewAction::DetailView);
    m.insert(b'f', BlockViewAction::ToggleFailedFilter);
    m.insert(b'/', BlockViewAction::OpenSearch);
    m.insert(b'n', BlockViewAction::SearchNext);
    m.insert(b'N', BlockViewAction::SearchPrev);
    m.insert(b'c', BlockViewAction::CopyCommand);
    m.insert(b'o', BlockViewAction::CopyOutput);
    m.insert(b'y', BlockViewAction::CopyBoth);
    m.insert(b'r', BlockViewAction::Rerun);
    m.insert(b'd', BlockViewAction::Delete);
    m.insert(b'v', BlockViewAction::VisualMode);
    m.insert(b'?', BlockViewAction::Help);
    m.insert(b'q', BlockViewAction::Quit);
    m.insert(0x1b, BlockViewAction::Quit);
    m
}

pub fn default_detail_keymap() -> HashMap<u8, DetailViewAction> {
    let mut m = HashMap::new();
    m.insert(b'j', DetailViewAction::NavDown);
    m.insert(b'k', DetailViewAction::NavUp);
    m.insert(b'G', DetailViewAction::NavBottom);
    m.insert(b'g', DetailViewAction::NavTop);
    m.insert(b'c', DetailViewAction::CopyCommand);
    m.insert(b'o', DetailViewAction::CopyOutput);
    m.insert(b'y', DetailViewAction::CopyBoth);
    m.insert(b'r', DetailViewAction::Rerun);
    m.insert(b'v', DetailViewAction::VisualMode);
    m.insert(b'V', DetailViewAction::VisualMode);
    m.insert(b'?', DetailViewAction::Help);
    m.insert(b'q', DetailViewAction::Quit);
    m.insert(0x1b, DetailViewAction::Quit);
    m
}

pub fn build_resolved_block_keymap(
    overrides: &HashMap<String, String>,
) -> HashMap<u8, BlockViewAction> {
    let mut map = default_block_keymap();
    for (action_name, key_str) in overrides {
        let Some(action) = deserialize_block_action(action_name) else {
            continue;
        };
        if let Some(&byte) = key_str.as_bytes().first() {
            map.retain(|_, v| *v != action);
            map.remove(&byte);
            map.insert(byte, action);
        }
    }
    map
}

pub fn build_resolved_detail_keymap(
    overrides: &HashMap<String, String>,
) -> HashMap<u8, DetailViewAction> {
    let mut map = default_detail_keymap();
    for (action_name, key_str) in overrides {
        let Some(action) = deserialize_detail_action(action_name) else {
            continue;
        };
        if let Some(&byte) = key_str.as_bytes().first() {
            map.retain(|_, v| *v != action);
            map.remove(&byte);
            map.insert(byte, action);
        }
    }
    map
}

pub fn build_runtime_config(config: Config) -> RuntimeConfig {
    let max_blocks = config
        .history
        .max_blocks
        .or_else(|| Some(config.blocks.max_blocks));

    // Merge legacy `tui_apps` top-level section with `[tui.apps]` (latter wins).
    let mut merged_apps = config.tui_apps;
    for (name, app) in config.tui.apps {
        merged_apps.insert(name, app);
    }

    RuntimeConfig {
        block_layout: config.block_layout,
        block_view: config.block_view,
        max_blocks,
        resolved_block_keymap: build_resolved_block_keymap(&config.keymap.blocks),
        resolved_detail_keymap: build_resolved_detail_keymap(&config.keymap.detail),
        tui_extra_commands: config.tui.extra_commands,
        tui_apps: merged_apps,
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
    #[serde(default)]
    pub return_panel: ReturnPanelTarget,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct TuiConfig {
    #[serde(default)]
    pub extra_commands: Vec<String>,
    #[serde(default)]
    pub apps: BTreeMap<String, TuiAppConfig>,
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

#[cfg(test)]
mod tests {
    use super::{
        BlockViewConfig, Config, build_resolved_block_keymap, build_runtime_config,
        default_block_keymap,
    };
    use crate::app::BlockViewAction;
    use crate::format::CopyFormat;
    use std::collections::HashMap;

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

    // ─── Keymap tests ────────────────────────────────────────────────────

    #[test]
    fn keymap_default_j_maps_to_nav_down() {
        let map = default_block_keymap();
        assert_eq!(map.get(&b'j'), Some(&BlockViewAction::NavDown));
    }

    #[test]
    fn keymap_user_override_remaps_key() {
        let mut overrides = HashMap::new();
        overrides.insert("nav_down".to_string(), "n".to_string());

        let map = build_resolved_block_keymap(&overrides);

        // NavDown is now on 'n', not 'j'
        assert_eq!(map.get(&b'n'), Some(&BlockViewAction::NavDown));
        assert_eq!(map.get(&b'j'), None);
    }

    #[test]
    fn keymap_unknown_action_in_toml_is_ignored() {
        let mut overrides = HashMap::new();
        // "non_existent_action" is not a valid BlockViewAction name
        overrides.insert("non_existent_action".to_string(), "x".to_string());

        // Should not panic — unknown action is silently ignored
        let map = build_resolved_block_keymap(&overrides);

        // Default keymap should be unchanged
        assert_eq!(map.get(&b'j'), Some(&BlockViewAction::NavDown));
    }
}
