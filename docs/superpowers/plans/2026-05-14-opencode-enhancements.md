# OpenCode Enhancements Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform the OpenCode integration from a passive observer into a high-performance, interactive experience by adding real-time updates, conversational history rendering, and bidirectional control.

**Architecture:**
1.  **Event-Driven Sync**: Add the `notify` crate to replace 500ms polling with filesystem events.
2.  **Conversational Detail View**: Extend `AgentLiveSnapshot` to include full history and update the compositor/renderer to draw "bubbles" or structured blocks for user/assistant messages.
3.  **Bidirectional Control**: Add keybindings in Block View (`s` for stop, `r` for retry) that send signals or keys to the agent's tmux pane.

**Tech Stack:** Rust, Notify (new), Ratatui, Serde, Tmux CLI

---

### Task 1: Event-Driven Real-time Updates

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/pty.rs`

- [ ] **Step 1: Add `notify` dependency**

Add `notify = "8.0.0"` to `Cargo.toml`.

- [ ] **Step 2: Replace polling loop with `notify` watcher**

In `src/pty.rs`, replace the `watcher_thread` loop with a `notify::RecommendedWatcher`.

```rust
    // In src/pty.rs near line 518
    let watcher_state = Arc::clone(&state);
    let watcher_stdout = Arc::clone(&stdout);
    let (tx, rx) = std::sync::mpsc::channel();
    let mut watcher = notify::recommended_watcher(tx).context("failed to create watcher")?;

    // Watch the registry directory for agent file changes
    let agents_dir = crate::agent_registry::registry_dir().join("agents");
    let _ = fs::create_dir_all(&agents_dir);
    watcher.watch(&agents_dir, notify::RecursiveMode::Recursive).ok();

    thread::spawn(move || {
        for res in rx {
            if let Ok(event) = res {
                // Only trigger if a relevant file (events.jsonl/session.json) changed
                let interesting = event.paths.iter().any(|p| {
                    p.extension().map_or(false, |ext| ext == "json" || ext == "jsonl")
                });
                if !interesting { continue; }

                if let Ok(mut st) = watcher_state.lock() {
                    if matches!(st.view.view, ViewKind::Blocks) {
                        sync_shared_agent_blocks(&mut st);
                        move_running_agents_to_bottom(&mut st);
                        st.render_state.dirty = true;
                        st.render_state.force_render = true;
                        let _ = render_runtime(&watcher_state, &watcher_stdout);
                    }
                }
            }
        }
    });
```

- [ ] **Step 3: Verify build and basic sync**

Run: `cargo build`
Expected: PASS

---

### Task 2: Conversational Detail View

**Files:**
- Modify: `src/app.rs`: `AgentLiveSnapshot` and `HistoryRecord` types
- Modify: `src/agent_events.rs`: Implement full history parsing
- Modify: `src/compositor.rs`: Logic for generating history lines
- Modify: `src/renderer.rs`: Styled rendering for bubbles/blocks

- [ ] **Step 1: Update data models**

Add `history` to `AgentLiveSnapshot` in `src/app.rs`.

```rust
pub struct AgentLiveSnapshot {
    pub status: AgentLiveStatus,
    pub at_ms: Option<u64>,
    pub current_tool: Option<String>,
    pub current_command: Option<String>,
    pub title: Option<String>,
    pub history: Vec<AgentHistoryRecord>, // New
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHistoryRecord {
    pub role: String, // "user" or "assistant"
    pub content: String,
    pub tool_calls: Vec<String>,
}
```

- [ ] **Step 2: Implement history parsing in `src/agent_events.rs`**

Update `read_agent_live_snapshot` to also read and parse `history.json`.

- [ ] **Step 3: Update Compositor to generate styled lines**

In `src/compositor.rs`, handle `ViewKind::Detail` for Agent blocks by iterating over `history` and creating `VisualLine`s with distinct colors for User/Assistant.

- [ ] **Step 4: Verify with a mock history file**

Create a dummy `history.json` and verify it renders in Detail View.

---

### Task 3: Bidirectional Control (Stop/Retry)

**Files:**
- Modify: `src/app.rs`: Add `AgentAction` enum
- Modify: `src/pty.rs`: Implement key handling for agent blocks

- [ ] **Step 1: Add key bindings in `src/pty.rs`**

In `handle_block_view_key`, detect when an agent block is selected.
Add `s` -> Stop Agent, `r` -> Retry (send `Enter` or a specific command).

```rust
// Logic for stopping an agent via tmux
fn stop_agent(pane_id: &str) {
    let _ = Command::new("tmux")
        .args(["send-keys", "-t", pane_id, "C-c"])
        .status();
}
```

- [ ] **Step 2: Implement "Jump and Retry"**

Pressing `r` should jump to the pane and optionally send a key.

- [ ] **Step 3: Verify interaction**

Run Tide, select a running OpenCode block, press `s`, and verify the agent in the other pane stops.

---

### Task 4: Polish & Performance (Incremental Buffer)

- [ ] **Step 1: Implement the "Seamless Integration" buffer optimizations**

Follow the steps in `docs/superpowers/plans/2026-05-14-seamless-integration.md` to ensure high performance even with large agent outputs.

- [ ] **Step 2: Final Integration Test**

Run full suite and manual test with a live OpenCode session.
