# Tide Structured Logging Design

## Goal
Implement a categorized, structured, and AI-readable logging system to replace the current primitive debug logging. This will enable deep analysis of PTY byte flows, state transitions, and shell hook processing.

## Architecture

### 1. Log Severity & Categorization
Introduce levels and categories to allow precise filtering.

- **Levels**: `INFO`, `DEBUG`, `TRACE`
- **Categories**:
    - `PTY`: Raw byte stream and I/O.
    - `HOOK`: OSC markers and shell state changes.
    - `APP`: Input handling and state transitions.
    - `RENDER`: Compositor and renderer logic.
    - `AGENT`: Cross-session sync and tmux logic.

### 2. Format Specification
Each log line will follow a consistent pattern for easy parsing:
`[+ms][LEVEL][CATEGORY] Message | Context: { ... }`

- **Byte Stream Representation**: For `TRACE` logs containing PTY bytes, non-printable characters will be escaped (e.g., `\x1b`) to ensure AI readability while preserving semantic meaning.

### 3. Core Components

#### `src/debug_log.rs` (Refactor)
- Update `DebugLog` to store log level and category.
- Implement a `log_structured` method that handles categorization and context serialization.
- Add a cleanup mechanism to retain only the last 10 log files in the `debug/` directory.

#### `dlog!` Macro Upgrades
Introduce a new set of macros for type-safe, categorized logging:
- `tinfo!(cat, msg)`
- `tdebug!(cat, msg, context)`
- `ttrace!(cat, bytes)`

#### Context Serialization
- Leverage `serde_json` to dump internal state (e.g., `CommandBlock`, `ViewState`) into logs when using `tdebug!`.

## Testing & Validation
- **Unit Tests**: Verify that logs are correctly categorized and formatted.
- **Verification**: Manually confirm that `debug/` cleanup works and that the log output is readable and accurate.

---
**Reviewer Note**: This design replaces the existing `dlog!` macro with more specific variants to prevent "logging clutter" and ensure that high-volume data (like PTY bytes) is only recorded when appropriate.
