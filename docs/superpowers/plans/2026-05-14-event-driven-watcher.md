# Event-driven Agent Watcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the 500ms polling loop in Tide with an event-driven filesystem watcher using the `notify` crate.

**Architecture:** Use `notify::RecommendedWatcher` to monitor the agent events directory. When a `.json` or `.jsonl` file is modified, send an event to a dedicated thread that triggers a UI sync and re-render.

**Tech Stack:** Rust, `notify` crate.

---

### Task 1: Add Dependencies

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add notify dependency**

```toml
notify = "8.0.0"
```

- [ ] **Step 2: Verify compilation**

Run: `cargo build`
Expected: PASS

### Task 2: Implement Event-driven Watcher in `src/pty.rs`

**Files:**
- Modify: `src/pty.rs`

- [ ] **Step 1: Update imports**

Add `notify` related imports.

```rust
use notify::{Watcher, RecursiveMode, RecommendedWatcher, Config as NotifyConfig};
```

- [ ] **Step 2: Replace `watcher_thread` implementation**

Find the `watcher_thread` around line 518 and replace the polling loop with `notify` logic.

```rust
    // Watcher thread: uses notify to watch agent event files.
    // Triggers a sync + re-render when opencode writes new events.
    let watcher_running = Arc::clone(&running);
    let watcher_state = Arc::clone(&state);
    let watcher_stdout = Arc::clone(&stdout);
    let watcher_thread = thread::spawn(move || {
        let (tx, rx) = mpsc::channel();

        let mut watcher = match RecommendedWatcher::new(tx, NotifyConfig::default()) {
            Ok(w) => w,
            Err(e) => {
                dlog!("Failed to create watcher: {:?}", e);
                return;
            }
        };

        let agents_dir = crate::agent_registry::registry_dir().join("agents");
        if let Err(e) = fs::create_dir_all(&agents_dir) {
             dlog!("Failed to create agents dir: {:?}", e);
             return;
        }

        if let Err(e) = watcher.watch(&agents_dir, RecursiveMode::Recursive) {
            dlog!("Failed to watch agents dir: {:?}", e);
            return;
        }

        while watcher_running.load(Ordering::SeqCst) {
            // Use a timeout to periodically check if we should stop
            if let Ok(Ok(event)) = rx.recv_timeout(Duration::from_millis(500)) {
                let is_agent_event = event.paths.iter().any(|p| {
                    let ext = p.extension().and_then(|s| s.to_str());
                    matches!(ext, Some("json") | Some("jsonl"))
                });

                if !is_agent_event {
                    continue;
                }

                let should_sync = {
                    let Ok(state) = watcher_state.lock() else {
                        continue;
                    };
                    matches!(state.view.view, ViewKind::Blocks)
                };

                if should_sync {
                    let should_render = if let Ok(mut state) = watcher_state.lock() {
                        sync_shared_agent_blocks(&mut state);
                        move_running_agents_to_bottom(&mut state);
                        state.render_state.dirty = true;
                        state.render_state.force_render = true;
                        true
                    } else {
                        false
                    };
                    if should_render {
                        let _ = render_runtime(&watcher_state, &watcher_stdout);
                    }
                }
            }
        }
    });
```

- [ ] **Step 3: Handle `fs` import if needed**

Ensure `std::fs` is available in `src/pty.rs`.

- [ ] **Step 4: Verify compilation**

Run: `cargo build`
Expected: PASS

### Task 3: Cleanup and Final Verification

**Files:**
- Modify: `src/pty.rs`

- [ ] **Step 1: Remove `agent_event_mtimes` from `RuntimeState`**

Since we no longer poll mtimes, this field is redundant.

- [ ] **Step 2: Verify compilation and tests**

Run: `cargo build && cargo test`
Expected: PASS
