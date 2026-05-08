# Tide

Tide is a zsh-based layered shell wrapper.

It runs inside the user's existing terminal, starts real `zsh`, transparently shows Normal mode output, captures shell executions into structured blocks, and redraws that captured history in Block and Detail views:

- Plain
- Blocks
- Detail

Tide is not a terminal emulator and not a replacement for zsh. Normal mode is passthrough; Block mode is a reconstructed view based on Tide's own captured `ShellBuffer` and `BlockStore`.

Current focus: the minimal Block Layer loop.

```text
Normal:
zsh PTY output -> marker parser -> visible bytes -> real terminal
                              -> sidecar capture -> ShellBuffer + BlockStore

Block / Detail:
ShellBuffer + BlockStore + ViewState -> Compositor -> Renderer
```

See:

- [docs/architecture.md](docs/architecture.md)
- [docs/block-layer.md](docs/block-layer.md)
- [docs/internal-api.md](docs/internal-api.md)
- [docs/config.md](docs/config.md)
- [docs/zsh-integration.md](docs/zsh-integration.md)
- [docs/raw-program.md](docs/raw-program.md)
- [docs/manual-testing.md](docs/manual-testing.md)
