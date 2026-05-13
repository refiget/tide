use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    Opencode,
}

impl AgentProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            AgentProvider::Opencode => "opencode",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "opencode" => Some(AgentProvider::Opencode),
            _ => None,
        }
    }
}

/// Lightweight reference to a running agent session (provider + registry alias).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRef {
    pub provider: AgentProvider,
    pub alias: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Running,
    Stale,
    Exited,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    pub provider: AgentProvider,
    pub alias: String,
    pub source_tide_id: String,
    pub command_block_id: u64,
    pub command: String,
    pub cwd: String,
    pub project_name: String,
    pub tmux_target: String,
    pub tmux_pane_id: String,
    pub tmux_window_id: String,
    pub status: AgentStatus,
    pub started_at_ms: u128,
    pub last_seen_at_ms: u128,
    pub exited_at_ms: Option<u128>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    pub records: Vec<AgentRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JumpRecord {
    pub from_tmux_target: String,
    pub to_tmux_target: String,
    pub at_ms: u128,
    #[serde(default)]
    pub from_zoomed: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct JumpStackFile {
    records: Vec<JumpRecord>,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

pub fn registry_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TIDE_REGISTRY_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".tide")
}

fn registry_path() -> PathBuf {
    registry_dir().join("agent_registry.json")
}

fn lock_path() -> PathBuf {
    registry_dir().join("agent_registry.lock")
}

fn jump_path() -> PathBuf {
    registry_dir().join("agent_jump_stack.json")
}

fn with_lock<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    fs::create_dir_all(registry_dir()).context("create ~/.tide")?;

    let mut lock_acquired = false;
    for _ in 0..40 {
        let open = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(lock_path());
        if open.is_ok() {
            lock_acquired = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }

    if !lock_acquired {
        anyhow::bail!("agent registry lock timeout");
    }

    let result = f();
    let _ = fs::remove_file(lock_path());
    result
}

fn read_registry_unlocked(path: &Path) -> RegistryFile {
    let mut data = String::new();
    if let Ok(mut f) = OpenOptions::new().read(true).open(path)
        && f.read_to_string(&mut data).is_ok()
        && !data.trim().is_empty()
        && let Ok(parsed) = serde_json::from_str::<RegistryFile>(&data)
    {
        return parsed;
    }
    RegistryFile::default()
}

fn write_registry_unlocked(path: &Path, reg: &RegistryFile) -> Result<()> {
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let payload = serde_json::to_string(reg).context("serialize agent registry")?;
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .context("open tmp registry")?;
    f.write_all(payload.as_bytes())
        .context("write tmp registry")?;
    f.sync_all().ok();
    fs::rename(&tmp, path).context("rename tmp registry")?;
    Ok(())
}

fn read_jump_stack_unlocked(path: &Path) -> JumpStackFile {
    let mut data = String::new();
    if let Ok(mut f) = OpenOptions::new().read(true).open(path)
        && f.read_to_string(&mut data).is_ok()
        && !data.trim().is_empty()
        && let Ok(parsed) = serde_json::from_str::<JumpStackFile>(&data)
    {
        return parsed;
    }
    JumpStackFile::default()
}

fn write_jump_stack_unlocked(path: &Path, stack: &JumpStackFile) -> Result<()> {
    let payload = serde_json::to_string(stack).context("serialize jump stack")?;
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .context("open tmp jump file")?;
    f.write_all(payload.as_bytes())
        .context("write tmp jump file")?;
    f.sync_all().ok();
    fs::rename(&tmp, path).context("rename tmp jump file")?;
    Ok(())
}

pub fn list_all(provider: AgentProvider) -> Result<Vec<AgentRecord>> {
    with_lock(|| {
        let path = registry_path();
        let reg = read_registry_unlocked(&path);
        Ok(reg
            .records
            .into_iter()
            .filter(|r| r.provider == provider)
            .collect())
    })
}

fn alias_to_index(alias: &str) -> Option<usize> {
    if alias.is_empty() {
        return None;
    }
    let mut value: usize = 0;
    for c in alias.chars() {
        if !c.is_ascii_lowercase() {
            return None;
        }
        value = value * 26 + ((c as u8 - b'a') as usize + 1);
    }
    Some(value - 1)
}

fn index_to_alias(mut idx: usize) -> String {
    idx += 1;
    let mut chars = Vec::new();
    while idx > 0 {
        let rem = (idx - 1) % 26;
        chars.push((b'a' + rem as u8) as char);
        idx = (idx - 1) / 26;
    }
    chars.into_iter().rev().collect()
}

fn allocate_alias(records: &[AgentRecord]) -> String {
    let mut used: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for r in records {
        if let Some(i) = alias_to_index(&r.alias) {
            used.insert(i);
        }
    }
    let mut idx = 0usize;
    loop {
        if !used.contains(&idx) {
            return index_to_alias(idx);
        }
        idx += 1;
    }
}

pub fn register_running(
    provider: AgentProvider,
    source_tide_id: &str,
    command_block_id: u64,
    command: &str,
    cwd: &str,
    project_name: &str,
    tmux_target: &str,
    tmux_pane_id: &str,
    tmux_window_id: &str,
) -> Result<String> {
    with_lock(|| {
        let path = registry_path();
        let mut reg = read_registry_unlocked(&path);

        if let Some(pos) = reg.records.iter().position(|r| {
            r.provider == provider
                && r.source_tide_id == source_tide_id
                && r.command_block_id == command_block_id
        }) {
            let alias = reg.records[pos].alias.clone();
            reg.records[pos].command = command.to_string();
            reg.records[pos].cwd = cwd.to_string();
            reg.records[pos].project_name = project_name.to_string();
            reg.records[pos].tmux_target = tmux_target.to_string();
            reg.records[pos].tmux_pane_id = tmux_pane_id.to_string();
            reg.records[pos].tmux_window_id = tmux_window_id.to_string();
            reg.records[pos].status = AgentStatus::Running;
            reg.records[pos].last_seen_at_ms = now_ms();
            reg.records[pos].exited_at_ms = None;
            write_registry_unlocked(&path, &reg)?;
            return Ok(alias);
        }

        let same_provider: Vec<AgentRecord> = reg
            .records
            .iter()
            .filter(|r| r.provider == provider)
            .cloned()
            .collect();
        let alias = allocate_alias(&same_provider);
        reg.records.push(AgentRecord {
            provider,
            alias: alias.clone(),
            source_tide_id: source_tide_id.to_string(),
            command_block_id,
            command: command.to_string(),
            cwd: cwd.to_string(),
            project_name: project_name.to_string(),
            tmux_target: tmux_target.to_string(),
            tmux_pane_id: tmux_pane_id.to_string(),
            tmux_window_id: tmux_window_id.to_string(),
            status: AgentStatus::Running,
            started_at_ms: now_ms(),
            last_seen_at_ms: now_ms(),
            exited_at_ms: None,
        });
        write_registry_unlocked(&path, &reg)?;
        Ok(alias)
    })
}

pub fn unregister_running(
    provider: AgentProvider,
    source_tide_id: &str,
    command_block_id: u64,
) -> Result<()> {
    with_lock(|| {
        let path = registry_path();
        let mut reg = read_registry_unlocked(&path);
        let now = now_ms();
        for rec in &mut reg.records {
            if rec.provider == provider
                && rec.source_tide_id == source_tide_id
                && rec.command_block_id == command_block_id
            {
                rec.status = AgentStatus::Exited;
                rec.last_seen_at_ms = now;
                rec.exited_at_ms = Some(now);
            }
        }
        write_registry_unlocked(&path, &reg)
    })
}

pub fn find_by_alias(provider: AgentProvider, alias: &str) -> Result<Option<AgentRecord>> {
    with_lock(|| {
        let path = registry_path();
        let reg = read_registry_unlocked(&path);
        Ok(reg
            .records
            .into_iter()
            .find(|r| r.provider == provider && r.alias == alias))
    })
}

pub fn mark_stale(provider: AgentProvider, alias: &str) -> Result<()> {
    with_lock(|| {
        let path = registry_path();
        let mut reg = read_registry_unlocked(&path);
        for rec in &mut reg.records {
            if rec.provider == provider && rec.alias == alias && rec.status == AgentStatus::Running
            {
                rec.status = AgentStatus::Stale;
                rec.last_seen_at_ms = now_ms();
            }
        }
        write_registry_unlocked(&path, &reg)
    })
}

pub fn write_last_jump(
    from_tmux_target: &str,
    to_tmux_target: &str,
    from_zoomed: bool,
) -> Result<()> {
    if from_tmux_target == to_tmux_target {
        return Ok(());
    }
    with_lock(|| {
        let path = jump_path();
        let mut stack = read_jump_stack_unlocked(&path);
        stack.records.push(JumpRecord {
            from_tmux_target: from_tmux_target.to_string(),
            to_tmux_target: to_tmux_target.to_string(),
            at_ms: now_ms(),
            from_zoomed,
        });
        if stack.records.len() > 32 {
            let drain = stack.records.len() - 32;
            stack.records.drain(0..drain);
        }
        write_jump_stack_unlocked(&path, &stack)
    })
}

pub fn pop_jump_for_target(current_target: &str) -> Result<Option<JumpRecord>> {
    with_lock(|| {
        let path = jump_path();
        let mut stack = read_jump_stack_unlocked(&path);
        let found = stack
            .records
            .iter()
            .rposition(|r| r.to_tmux_target == current_target);
        let out = found.map(|idx| stack.records.remove(idx));
        write_jump_stack_unlocked(&path, &stack)?;
        Ok(out)
    })
}
