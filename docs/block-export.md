# Block Export v1

`block_export.v1` is Tide's stable machine-oriented JSON projection for a `CommandBlock`.

## Source

- Generated from `CommandBlock` fact data plus deterministic derived views.
- Emitted through copy pipeline when `block_view.copy_format = "json"`.

## Shape

Single block:

```json
{
  "schema_version": "block_export.v1",
  "id": 42,
  "kind": "normal_command",
  "status": "failed",
  "output_semantics": "line_oriented",
  "output_truncated": true,
  "cwd": "/Users/bob/Projects/Tide",
  "started_at_ms": 1715491200000,
  "finished_at_ms": 1715491201400,
  "duration_ms": 1400,
  "exit_code": 1,
  "output_stored_bytes": 1024,
  "command": "cargo test",
  "output_text": "...",
  "views": {
    "summary": {
      "headline": "cargo test",
      "status": "failed",
      "duration_ms": 1400,
      "exit_code": 1,
      "truncated": true
    },
    "error": {
      "status": "failed",
      "exit_code": 1,
      "tail": "..."
    },
    "audit": ["output_truncated", "command_failed"],
    "context": {
      "command": "cargo test",
      "cwd": "/Users/bob/Projects/Tide",
      "status": "failed",
      "output_excerpt": "..."
    }
  }
}
```

Multiple blocks are returned as a JSON array of block objects.

## CopyPart Behavior

- `Command`: includes `command`; excludes `output_text` and `views`.
- `Output`: includes `output_text`; excludes `command` and `views`.
- `Both`: includes `command`, `output_text`, and `views`.

## Semantics Notes

- `kind` and `status` are intentionally separated:
  - `kind` describes execution type (`normal_command`, `tui_session`, `raw_program`, ...)
  - `status` describes result (`running`, `success`, `failed`, ...)
- `output_truncated` is a first-class semantic field and should be honored by consumers.
- `output_semantics` is `non_linear_tui` for `raw_program`/`tui_session`; otherwise `line_oriented`.
- For `tui_session` and `raw_program`, `views.context.output_excerpt` is intentionally empty.
