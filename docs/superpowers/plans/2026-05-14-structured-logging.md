# Structured Logging Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement a categorized, structured, and AI-readable logging system to replace the current primitive debug logging.

**Architecture:**
- Refactor `DebugLog` in `src/debug_log.rs` to support `LogLevel` and `LogCategory`.
- Implement specialized macros (`tinfo!`, `tdebug!`, `ttrace!`) for structured logging.
- Add a cleanup mechanism to `DebugLog::open_if_enabled` to maintain only the last 10 log files.
- Migrate all existing `dlog!` calls in `src/pty.rs` and other files to the new structured macros.

**Tech Stack:** Rust, Serde, Serde JSON

---

### Task 1: Refactor DebugLog and Categories

**Files:**
- Modify: `src/debug_log.rs`

- [ ] **Step 1: Define LogLevel and LogCategory**

Add these enums to `src/debug_log.rs`.

```rust
#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    INFO,
    DEBUG,
    TRACE,
}

impl std::fmt::Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::INFO => write!(f, "INFO"),
            LogLevel::DEBUG => write!(f, "DEBUG"),
            LogLevel::TRACE => write!(f, "TRACE"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LogCategory {
    PTY,
    HOOK,
    APP,
    RENDER,
    AGENT,
}

impl std::fmt::Display for LogCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogCategory::PTY => write!(f, "PTY"),
            LogCategory::HOOK => write!(f, "HOOK"),
            LogCategory::APP => write!(f, "APP"),
            LogCategory::RENDER => write!(f, "RENDER"),
            LogCategory::AGENT => write!(f, "AGENT"),
        }
    }
}
```

- [ ] **Step 2: Update DebugLog struct and methods**

Add `log_structured` method and a helper for byte escaping.

```rust
impl DebugLog {
    // ... existing open_if_enabled ...

    pub fn log_structured(&mut self, level: LogLevel, cat: LogCategory, msg: &str, context: Option<&str>) {
        let ms = self.start.elapsed().as_millis();
        let ctx_str = context.map(|c| format!(" | Context: {}", c)).unwrap_or_default();
        let _ = writeln!(self.writer, "[+{}ms][{}][{}] {}{}", ms, level, cat, msg, ctx_str);
        let _ = self.writer.flush();
    }

    pub fn escape_bytes(bytes: &[u8]) -> String {
        let mut s = String::new();
        for &b in bytes {
            if b.is_ascii_graphic() || b == b' ' {
                s.push(b as char);
            } else {
                s.push_str(&format!("\\x{:02x}", b));
            }
        }
        s
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add src/debug_log.rs
git commit -m "feat: refactor DebugLog with levels and categories"
```

---

### Task 2: Implement New Macros

**Files:**
- Modify: `src/debug_log.rs`

- [ ] **Step 1: Implement tinfo!, tdebug!, ttrace! macros**

Add these macros to `src/debug_log.rs` and update/remove `dlog!`.

```rust
#[macro_export]
macro_rules! tinfo {
    ($log:expr, $cat:expr, $($arg:tt)*) => {
        if let Some(ref mut __log) = $log {
            __log.log_structured($crate::debug_log::LogLevel::INFO, $crate::debug_log::LogCategory::$cat, &format!($($arg)*), None);
        }
    };
}

#[macro_export]
macro_rules! tdebug {
    ($log:expr, $cat:expr, $msg:expr, $context:expr) => {
        if let Some(ref mut __log) = $log {
            let ctx_json = serde_json::to_string($context).unwrap_or_else(|_| "null".to_string());
            __log.log_structured($crate::debug_log::LogLevel::DEBUG, $crate::debug_log::LogCategory::$cat, $msg, Some(&ctx_json));
        }
    };
}

#[macro_export]
macro_rules! ttrace {
    ($log:expr, $cat:expr, $msg:expr, $bytes:expr) => {
        if let Some(ref mut __log) = $log {
            let escaped = $crate::debug_log::DebugLog::escape_bytes($bytes);
            __log.log_structured($crate::debug_log::LogLevel::TRACE, $crate::debug_log::LogCategory::$cat, &format!("{}: {}", $msg, escaped), None);
        }
    };
}
```

- [ ] **Step 2: Commit**

```bash
git add src/debug_log.rs
git commit -m "feat: implement structured logging macros"
```

---

### Task 3: Implement Log Cleanup

**Files:**
- Modify: `src/debug_log.rs`

- [ ] **Step 1: Add cleanup logic to open_if_enabled**

Before creating a new log file, list files in `debug/` and remove old ones.

```rust
    pub fn open_if_enabled() -> Option<Self> {
        if std::env::var_os("TIDE_DEBUG").is_none() {
            return None;
        }
        let dir = std::env::current_dir().ok()?.join("debug");
        std::fs::create_dir_all(&dir).ok()?;

        // Cleanup: keep last 10 logs
        if let Ok(entries) = std::fs::read_dir(&dir) {
            let mut paths: Vec<_> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
            paths.sort_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
            if paths.len() >= 10 {
                for i in 0..paths.len().saturating_sub(9) {
                    let _ = std::fs::remove_file(&paths[i]);
                }
            }
        }

        // ... existing logic to create new file ...
    }
```

- [ ] **Step 2: Commit**

```bash
git add src/debug_log.rs
git commit -m "feat: add log file cleanup to retain only last 10 logs"
```

---

### Task 4: Migrate Existing Logs

**Files:**
- Modify: `src/pty.rs`
- Modify: `src/app.rs` (ensure derive Serialize)
- Modify: `src/block.rs` (ensure derive Serialize)

- [ ] **Step 1: Update core structs to derive Serialize**

In `src/app.rs`, `src/block.rs`, and other relevant files, ensure `serde::Serialize` is derived where context dumping is needed.

- [ ] **Step 2: Replace dlog! with structured macros in src/pty.rs**

Search and replace `dlog!` calls.
Examples:
- `dlog!(state.debug_log, "hook zle_ready ...")` -> `tinfo!(state.debug_log, HOOK, "zle_ready")`
- `dlog!(state.debug_log, "view Plain -> Blocks")` -> `tinfo!(state.debug_log, APP, "View: Plain -> Blocks")`

- [ ] **Step 3: Verify and Commit**

Run `cargo check` and then commit.
```bash
git commit -am "chore: migrate existing logs to structured macros"
```
