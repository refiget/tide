use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpencodeRecord {
    pub alias: String,
    pub source_tide_id: String,
    pub command_block_id: u64,
    pub command: String,
    pub cwd: String,
    pub project_name: String,
    pub tmux_target: String,
    pub started_at_ms: u128,
    pub last_seen_at_ms: u128,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    pub records: Vec<OpencodeRecord>,
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn registry_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TIDE_REGISTRY_DIR") {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".tide")
}

fn registry_path() -> PathBuf {
    registry_dir().join("opencode_registry.json")
}

fn lock_path() -> PathBuf {
    registry_dir().join("opencode_registry.lock")
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
        anyhow::bail!("opencode registry lock timeout");
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
    let payload = serde_json::to_string(reg).context("serialize opencode registry")?;
    let mut f = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&tmp)
        .context("open tmp registry")?;
    f.write_all(payload.as_bytes()).context("write tmp registry")?;
    f.sync_all().ok();
    fs::rename(&tmp, path).context("rename tmp registry")?;
    Ok(())
}

pub fn list_running() -> Result<Vec<OpencodeRecord>> {
    with_lock(|| {
        let path = registry_path();
        let reg = read_registry_unlocked(&path);
        Ok(reg.records)
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

fn allocate_alias(records: &[OpencodeRecord]) -> String {
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
    source_tide_id: &str,
    command_block_id: u64,
    command: &str,
    cwd: &str,
    project_name: &str,
    tmux_target: &str,
) -> Result<String> {
    with_lock(|| {
        let path = registry_path();
        let mut reg = read_registry_unlocked(&path);

        if let Some(pos) = reg
            .records
            .iter()
            .position(|r| r.source_tide_id == source_tide_id && r.command_block_id == command_block_id)
        {
            let alias = reg.records[pos].alias.clone();
            reg.records[pos].command = command.to_string();
            reg.records[pos].cwd = cwd.to_string();
            reg.records[pos].project_name = project_name.to_string();
            reg.records[pos].tmux_target = tmux_target.to_string();
            reg.records[pos].last_seen_at_ms = now_ms();
            write_registry_unlocked(&path, &reg)?;
            return Ok(alias);
        }

        let alias = allocate_alias(&reg.records);
        reg.records.push(OpencodeRecord {
            alias: alias.clone(),
            source_tide_id: source_tide_id.to_string(),
            command_block_id,
            command: command.to_string(),
            cwd: cwd.to_string(),
            project_name: project_name.to_string(),
            tmux_target: tmux_target.to_string(),
            started_at_ms: now_ms(),
            last_seen_at_ms: now_ms(),
        });
        write_registry_unlocked(&path, &reg)?;
        Ok(alias)
    })
}

pub fn unregister_running(source_tide_id: &str, command_block_id: u64) -> Result<()> {
    with_lock(|| {
        let path = registry_path();
        let mut reg = read_registry_unlocked(&path);
        reg.records
            .retain(|r| !(r.source_tide_id == source_tide_id && r.command_block_id == command_block_id));
        write_registry_unlocked(&path, &reg)
    })
}

pub fn find_by_alias(alias: &str) -> Result<Option<OpencodeRecord>> {
    with_lock(|| {
        let path = registry_path();
        let reg = read_registry_unlocked(&path);
        Ok(reg.records.into_iter().find(|r| r.alias == alias))
    })
}

#[cfg(test)]
mod tests {
    use super::{alias_to_index, index_to_alias};

    #[test]
    fn alias_roundtrip() {
        for idx in [0usize, 1, 25, 26, 27, 51, 52, 701] {
            let a = index_to_alias(idx);
            assert_eq!(alias_to_index(&a), Some(idx));
        }
    }
}
