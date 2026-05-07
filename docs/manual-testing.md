# Manual Testing

Manual testing is required for terminal behavior because automated tests cannot fully validate raw mode, PTY passthrough, resize behavior, and interactive TUI handoff.

Update this document whenever Tide gains a new user-visible terminal behavior, mode, command lifecycle feature, block action, or TUI handoff-return feature.

## Before Testing

Run static checks first:

```sh
cargo fmt --check
cargo check
cargo test
```

Run interactive tests from a real terminal emulator, not from an IDE output panel.

Use a terminal tab that you can close if raw mode or screen state breaks during development.

## Milestone 1: Transparent zsh Wrapper

Goal: `cargo run` starts Tide, Tide starts real `zsh`, and ordinary shell use still feels like normal zsh.

### 1. Start Tide

Command:

```sh
cargo run
```

Expected:

- The program starts without panic.
- A normal zsh prompt appears.
- The terminal does not show Tide-specific UI yet.
- Typing appears as expected for a raw-mode shell session.

### 2. Basic Command Passthrough

Inside Tide, run:

```sh
echo hello
pwd
printf 'a\nb\n'
```

Expected:

- Output appears exactly like normal zsh.
- Prompt returns after each command.
- No extra control sequences or Tide debug text are visible.

### 3. Interactive Input

Inside Tide, run:

```sh
read name
```

Type:

```text
tide
```

Then run:

```sh
echo $name
```

Expected:

- `read` accepts keyboard input.
- `echo $name` prints `tide`.
- Enter, Backspace, and normal text input work.

### 4. Ctrl-C Handling

Inside Tide, run:

```sh
sleep 10
```

Press `Ctrl-C`.

Expected:

- `sleep` is interrupted.
- zsh returns to the prompt.
- Tide does not exit.
- The terminal remains usable.

### 5. Ctrl-D and exit Handling

Start Tide again if needed:

```sh
cargo run
```

Inside Tide, test both:

```sh
exit
```

and in a separate run, press `Ctrl-D` at an empty prompt.

Expected:

- zsh exits.
- Tide exits.
- The outer terminal returns to normal input mode.
- Text typed after Tide exits is not stuck in raw mode.

### 6. Terminal Resize

Start Tide:

```sh
cargo run
```

Inside Tide, run:

```sh
stty size
```

Resize the terminal window, then run:

```sh
stty size
```

Expected:

- The reported rows and columns change after resizing.
- Prompt rendering remains coherent after resize.
- No panic occurs.

### 7. Full-Screen TUI Passthrough Smoke Test

Use whichever of these commands is installed:

```sh
nvim
vim
less Cargo.toml
```

Expected:

- The TUI app opens normally.
- Tide does not draw overlays.
- Keyboard input belongs to the TUI app.
- Exiting the TUI app returns to the zsh prompt.

Milestone 1 does not yet create `TuiSession` blocks or return panels. This test only verifies transparent passthrough.

### 8. Terminal Recovery After Failure

In an outer shell after Tide exits, run:

```sh
echo ok
stty -a
```

Expected:

- Text input echoes normally.
- Enter creates new lines normally.
- The terminal is not left in a broken raw-mode state.

If the terminal is broken during development, run:

```sh
reset
```

or:

```sh
stty sane
```

## Known Milestone 1 Limits

- No zsh lifecycle hook parsing yet.
- No command block capture yet.
- No BlockInteraction UI yet.
- No TUI handoff-return detection yet.
- No ReturnPanel yet.
- No AI features yet.

These are expected and should not be treated as failures for Milestone 1.

## Regression Checklist

Before committing changes that affect terminal behavior, verify:

- `cargo fmt --check` passes.
- `cargo check` passes.
- `cargo test` passes.
- `cargo run` starts zsh.
- Basic commands print output and return to prompt.
- `Ctrl-C` interrupts a foreground command without exiting Tide.
- `exit` or `Ctrl-D` exits Tide.
- Terminal input is normal after Tide exits.
- Resize updates the PTY size.
- A simple full-screen TUI still works as passthrough.
