#![allow(dead_code)]

use std::{collections::BTreeMap, fs, path::Path};

use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub shell: ShellConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub blocks: BlocksConfig,
    #[serde(default)]
    pub tui_apps: BTreeMap<String, TuiAppConfig>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let path = Path::new("config/tide.toml");

        if !path.exists() {
            return Ok(Self::default());
        }

        let source = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file {}", path.display()))?;
        let config = toml::from_str(&source)
            .with_context(|| format!("failed to parse config file {}", path.display()))?;

        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shell: ShellConfig::default(),
            ui: UiConfig::default(),
            blocks: BlocksConfig::default(),
            tui_apps: BTreeMap::new(),
        }
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
    10
}

fn default_max_output_bytes_per_block() -> usize {
    1_048_576
}

fn default_strip_ansi_for_text() -> bool {
    true
}

fn default_return_panel() -> String {
    "none".to_string()
}
