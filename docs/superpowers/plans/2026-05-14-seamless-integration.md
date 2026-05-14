# Seamless Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enhance Tide's performance and "stealth" by optimizing shell buffer management and refining terminal state restoration.

**Architecture:**
1.  **ShellBuffer Optimization**: Replace expensive `chars().collect::<Vec<_>>()` operations with more efficient `char_indices()` and `replace_range`/`remove` methods in `src/buffer.rs`.
2.  **Atomic Terminal Restoration**: Refine `leave_block_render` in `src/renderer.rs` to minimize redundant terminal resets, ensuring a smooth transition back to the shell.
3.  **TUI Detection Polish**: Ensure `apply_pty_raw_mode_change` in `src/pty.rs` effectively handles unclassified interactive programs to prevent data capture overhead.

**Tech Stack:** Rust, Tokio, Crossterm, libc (termios)

---

### Task 1: Optimize ShellBuffer Performance

**Files:**
- Modify: `src/buffer.rs`

- [ ] **Step 1: Optimize `put_char` method**

Replace character vector collection with direct string manipulation using `char_indices`.

```rust
    fn put_char(&mut self, ch: char) {
        if self.current_col == self.current_line_chars {
            self.current_line.push(ch);
            self.current_col += 1;
            self.current_line_chars += 1;
            return;
        }

        if self.current_col > self.current_line_chars {
            let padding = self.current_col - self.current_line_chars;
            self.current_line.push_str(&" ".repeat(padding));
            self.current_line.push(ch);
            self.current_col += 1;
            self.current_line_chars += padding + 1;
            return;
        }

        if let Some((byte_offset, old_ch)) = self.current_line.char_indices().nth(self.current_col) {
            self.current_line.replace_range(byte_offset..byte_offset + old_ch.len_utf8(), &ch.to_string());
        }
        self.current_col += 1;
    }
```

- [ ] **Step 2: Optimize `backspace` method**

Similarly, use `char_indices` to find the character to remove.

```rust
    fn backspace(&mut self) {
        if self.current_col == 0 {
            return;
        }

        self.current_col -= 1;
        if let Some((byte_offset, _)) = self.current_line.char_indices().nth(self.current_col) {
            self.current_line.remove(byte_offset);
            self.current_line_chars = self.current_line_chars.saturating_sub(1);
        }
    }
```

- [ ] **Step 3: Verify with existing tests**

Run: `cargo test buffer`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src/buffer.rs
git commit -m "perf: optimize ShellBuffer by avoiding character vector collection"
```

---

### Task 2: Refine Terminal State Restoration

**Files:**
- Modify: `src/renderer.rs`

- [ ] **Step 1: Simplify `leave_block_render`**

Remove redundant `ResetColor` if we are leaving the alternate screen, as most terminals restore the main screen's state (including SGR) or the shell prompt will handle it.

```rust
pub fn leave_block_render<W: Write>(w: &mut W, was_alt_screen: bool) -> io::Result<()> {
    if was_alt_screen {
        // Most terminals restore the SGR state of the main buffer when leaving the alternate screen.
        // The shell prompt (zle reset-prompt) will also ensure the correct style is applied.
        execute!(w, LeaveAlternateScreen, Show)
    } else {
        execute!(w, ResetColor, Show)
    }
}
```

- [ ] **Step 2: Verify visual transition**

This requires manual verification or checking if it breaks existing rendering logic.
Run: `cargo test renderer`
Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src/renderer.rs
git commit -m "chore: refine leave_block_render to minimize redundant resets"
```

---

### Task 4: Polish TUI Detection and Capture Suspension

**Files:**
- Modify: `src/pty.rs`

- [ ] **Step 1: Verify `apply_pty_raw_mode_change` logic**

Ensure it's robust and doesn't cause overhead for non-interactive programs.
Actually, the current implementation seems solid based on the research. I'll add a check to ensure `capture_pending` is cleared when entering interactive mode.

- [ ] **Step 2: Commit (if changes made)**

```bash
git add src/pty.rs
git commit -m "chore: polish TUI detection and capture suspension"
```
