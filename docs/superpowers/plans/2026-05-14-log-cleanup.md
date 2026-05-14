# Log Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement automatic cleanup of old log files in the `debug/` directory, keeping only the 10 most recent ones.

**Architecture:** Add a private helper `cleanup_old_logs` to `DebugLog` and call it during initialization.

**Tech Stack:** Rust (std::fs, std::time).

---

### Task 1: Implement `cleanup_old_logs` helper

**Files:**
- Modify: `src/debug_log.rs`

- [ ] **Step 1: Add `cleanup_old_logs` method to `DebugLog`**

```rust
    fn cleanup_old_logs(dir: &std::path::Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        let mut logs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with("tide-") && name.ends_with(".log") {
                        if let Ok(metadata) = entry.metadata() {
                            if let Ok(mtime) = metadata.modified() {
                                logs.push((path, mtime));
                            }
                        }
                    }
                }
            }
        }

        if logs.len() < 10 {
            return;
        }

        // Sort by modification time, oldest first
        logs.sort_by_key(|&(_, mtime)| mtime);

        let to_delete = logs.len().saturating_sub(9);
        for (path, _) in logs.iter().take(to_delete) {
            let _ = std::fs::remove_file(path);
        }
    }
```

- [ ] **Step 2: Integrate into `open_if_enabled`**

```rust
        // ... inside open_if_enabled ...
        let dir = std::env::current_dir().ok()?.join("debug");
        std::fs::create_dir_all(&dir).ok()?;
        
        Self::cleanup_old_logs(&dir); // Add this call

        let ts = std::time::SystemTime::now()
        // ...
```

- [ ] **Step 3: Run `cargo check`**

Run: `cargo check`
Expected: Success

- [ ] **Step 4: Commit**

```bash
git add src/debug_log.rs
git commit -m "feat: add log file cleanup to retain only last 10 logs"
```

### Task 2: Add unit tests

**Files:**
- Modify: `src/debug_log.rs`

- [ ] **Step 1: Add `test_log_cleanup` to the tests module**

```rust
    #[test]
    fn test_log_cleanup() {
        let temp_dir = std::env::temp_dir().join("tide_test_logs");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create 12 "log" files
        for i in 0..12 {
            let path = temp_dir.join(format!("tide-test-{}.log", i));
            std::fs::write(&path, "test").unwrap();
            // Sleep briefly to ensure different modification times if the FS resolution is low
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        DebugLog::cleanup_old_logs(&temp_dir);

        let remaining = std::fs::read_dir(&temp_dir).unwrap().count();
        assert_eq!(remaining, 9);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
```

- [ ] **Step 2: Run tests**

Run: `cargo test debug_log`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/debug_log.rs
git commit -m "test: add unit test for log cleanup"
```
