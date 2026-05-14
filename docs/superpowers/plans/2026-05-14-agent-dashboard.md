# Agent Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform Tide into an information panel (dashboard) for agents, allowing users to see summaries/status and seamlessly jump to/from agent panes, rather than controlling them directly.

**Architecture:** 
1. Revert previous bidirectional control actions (`AgentStop`, `AgentRetry`).
2. Map `Enter` (`Expand`) and `i` (`DetailView`) to unconditionally jump to the Agent's tmux pane if an Agent block is selected.
3. Map `Ctrl-b` (`0x02`) to jump back to Tide from a previous jump.
4. Clean up the help overlay and configurations to reflect this dashboard-centric workflow.

**Tech Stack:** Rust, Tmux CLI

---

### Task 1: Clean Up Control Actions

**Files:**
- Modify: `src/app.rs`
- Modify: `src/config.rs`
- Modify: `src/pty.rs`

- [ ] **Step 1: Remove AgentStop and AgentRetry**
In `src/app.rs`, remove `AgentStop` and `AgentRetry` from `BlockViewAction`.

- [ ] **Step 2: Update Configuration Deserializer**
In `src/config.rs`, remove `"agent_stop"` and `"agent_retry"` from `deserialize_block_action`. Keep `"jump_back"`.

- [ ] **Step 3: Update Execution Logic**
In `src/pty.rs` (`execute_block_view_action`), remove the `BlockViewAction::AgentStop` and `BlockViewAction::AgentRetry` match arms. Revert the `Rerun` action so it no longer checks for Agent blocks (it should behave normally or show "shared block: jump only").

- [ ] **Step 4: Verify Compilation**
Run `cargo check` to ensure no dangling references to the removed actions.

---

### Task 2: Configure Dashboard Navigation (Jump & Jump Back)

**Files:**
- Modify: `src/config.rs`
- Modify: `src/pty.rs`
- Modify: `src/renderer.rs`

- [ ] **Step 1: Map Ctrl-b to Jump Back**
In `src/config.rs` (`default_block_keymap`), remove the `b` mapping for `JumpBack`. Instead, map `0x02` (Ctrl-b) to `BlockViewAction::JumpBack`. Also, remove the previous mapping of `0x02` (which was `ScrollFullUp`) or let `JumpBack` replace it.
```rust
    // Remove: m.insert(0x02, BlockViewAction::ScrollFullUp);
    // Remove: m.insert(b'b', BlockViewAction::JumpBack);
    // Remove: m.insert(b's', BlockViewAction::AgentStop);
    m.insert(0x02, BlockViewAction::JumpBack);
```

- [ ] **Step 2: Ensure `Enter` and `i` jump to the agent pane**
In `src/pty.rs`, in `execute_block_view_action`, both `Expand` and `DetailView` should jump to the agent pane if an agent block is selected. (This might already be partially implemented via `jump_to_agent_pane(state, id, false)`, just verify `DetailView` does it unconditionally instead of showing Detail View for agents).

- [ ] **Step 3: Update Help Overlay**
In `src/renderer.rs` (`BLOCK_HELP_ENTRIES`), remove the `s` and `r` agent entries. Add or update the entry for jumping back.
```rust
    HelpEntry {
        key: "Enter / i",
        desc: "jump to agent pane",
    },
    HelpEntry {
        key: "Ctrl-b",
        desc: "jump back to tide",
    },
```
*(Make sure to remove any conflicting or old help entries).*

- [ ] **Step 4: Verify Compilation and Tests**
Run `cargo test`.
