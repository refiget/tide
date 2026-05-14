# Fix Broken Build and Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix compilation errors caused by AgentLiveSnapshot change, consolidate locks in src/pty.rs, and clean up unused code/imports.

**Architecture:** Surgical updates to handle Option types, lock consolidation for better performance/safety, and general cleanup of unused symbols.

**Tech Stack:** Rust, Cargo

---

### Task 1: Fix src/compositor.rs

**Files:**
- Modify: `src/compositor.rs:961-970`

- [ ] **Step 1: Update get_agent_history_styled_lines to handle history as Option**

```rust
fn get_agent_history_styled_lines(block: &CommandBlock) -> Vec<crate::ansi::StyledText> {
    use crate::ansi::{StyledSpan, StyledText, TextStyle};
    use crate::theme::CatppuccinFrappe;

    let Some(snapshot) = block.live_snapshot.as_ref() else {
        return vec![StyledText::plain("no agent history available")];
    };
    
    let history = snapshot.history.as_deref().unwrap_or_default();
    if history.is_empty() {
        return vec![StyledText::plain("no conversation history")];
    }

    let mut lines = Vec::new();
    for record in history {
        if let Some(user_msg) = &record.user_message {
            // ... (rest of loop remains same)
```

- [ ] **Step 2: Commit**

```bash
git add src/compositor.rs
git commit -m "fix(compositor): handle snapshot history as Option"
```

### Task 2: Consolidate Locks and Cleanup src/pty.rs

**Files:**
- Modify: `src/pty.rs:2`, `src/pty.rs:552-570`

- [ ] **Step 1: Remove unused imports**

```rust
// Remove tdebug and ttrace from line 2
use crate::{tinfo};
```

- [ ] **Step 2: Consolidate double lock in watcher thread**

```rust
                if let Ok(mut state) = watcher_state.lock() {
                    if matches!(state.view.view, ViewKind::Blocks) {
                        sync_shared_agent_blocks(&mut state);
                        move_running_agents_to_bottom(&mut state);
                        state.render_state.dirty = true;
                        state.render_state.force_render = true;
                        let _ = render_runtime(&watcher_state, &watcher_stdout);
                    }
                }
```
*Note: render_runtime needs &Arc<Mutex<RuntimeState>>, which watcher_state is.*

- [ ] **Step 3: Commit**

```bash
git add src/pty.rs
git commit -m "fix(pty): consolidate double lock and remove unused imports"
```

### Task 3: Cleanup src/renderer.rs, src/agent_events.rs, and src/debug_log.rs

**Files:**
- Modify: `src/renderer.rs:9`, `src/agent_events.rs:54`, `src/debug_log.rs:114`

- [ ] **Step 1: Remove unused LeaveAlternateScreen in src/renderer.rs**

- [ ] **Step 2: Remove unused text field in src/agent_events.rs**

- [ ] **Step 3: Remove unused log method in src/debug_log.rs**

- [ ] **Step 4: Commit**

```bash
git add src/renderer.rs src/agent_events.rs src/debug_log.rs
git commit -m "chore: cleanup unused code and imports"
```

### Task 4: Verification

- [ ] **Step 1: Run cargo check**

Run: `cargo check`
Expected: 0 errors, 0 warnings

- [ ] **Step 2: Run cargo test**

Run: `cargo test`
Expected: ALL PASS
