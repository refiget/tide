#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellHookEvent {
    Preexec { command: String },
    Precmd { exit_code: i32, cwd: Option<String> },
    CwdChanged { cwd: String },
    AltScreenEnter,
    AltScreenExit,
    ZleReady,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedPtyPart {
    Visible(Vec<u8>),
    Event(ShellHookEvent),
}

#[derive(Debug, Default)]
pub struct Osc777Parser {
    pending: Vec<u8>,
}

const MAX_PENDING_ESC: usize = 512;

impl Osc777Parser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<ParsedPtyPart> {
        self.pending.extend_from_slice(bytes);

        let mut parts = Vec::new();

        loop {
            // Find the first escape character.
            let Some(esc_pos) = self.pending.iter().position(|&b| b == b'\x1b') else {
                // No ESC. Drain everything as visible.
                if !self.pending.is_empty() {
                    parts.push(ParsedPtyPart::Visible(self.pending.drain(..).collect()));
                }
                break;
            };

            // Drain anything before the ESC as visible.
            if esc_pos > 0 {
                parts.push(ParsedPtyPart::Visible(
                    self.pending.drain(..esc_pos).collect(),
                ));
            }

            // We are at the start of an ESC sequence.
            if self.pending.len() < 2 {
                break; // Need more bytes.
            }

            match self.pending[1] {
                b'[' => {
                    // CSI sequence: ESC [ ...
                    match self.try_parse_csi() {
                        Some(res) => {
                            parts.extend(res);
                            continue;
                        }
                        None => {
                            if self.pending.len() > MAX_PENDING_ESC {
                                // Buffer overflow or malformed. Flush the ESC and continue.
                                parts.push(ParsedPtyPart::Visible(
                                    self.pending.drain(..1).collect(),
                                ));
                                continue;
                            }
                            break; // Incomplete sequence.
                        }
                    }
                }
                b']' => {
                    // OSC sequence: ESC ] ...
                    match self.try_parse_osc() {
                        Some(res) => {
                            parts.extend(res);
                            continue;
                        }
                        None => {
                            if self.pending.len() > MAX_PENDING_ESC {
                                // Buffer overflow or malformed. Flush the ESC and continue.
                                parts.push(ParsedPtyPart::Visible(
                                    self.pending.drain(..1).collect(),
                                ));
                                continue;
                            }
                            break; // Incomplete sequence.
                        }
                    }
                }
                _ => {
                    // Some other ESC sequence. Treat as visible and skip the ESC.
                    parts.push(ParsedPtyPart::Visible(self.pending.drain(..1).collect()));
                    continue;
                }
            }
        }

        parts
    }

    /// Try to parse a CSI sequence at the start of pending.
    /// Returns Some(parts) if complete (even if ignored), None if incomplete.
    fn try_parse_csi(&mut self) -> Option<Vec<ParsedPtyPart>> {
        // Find the terminator (usually 'h', 'l', 'm', etc.)
        // CSI sequences end with a byte in the range 0x40-0x7E.
        let end_pos = self
            .pending
            .iter()
            .skip(2)
            .position(|&b| (0x40..=0x7E).contains(&b))?;
        let end_pos = end_pos + 2;
        let terminator = self.pending[end_pos];
        let raw_seq = self.pending.drain(..=end_pos).collect::<Vec<u8>>();

        let mut parts = vec![ParsedPtyPart::Visible(raw_seq.clone())];

        // Only parse CSI private mode sequences starting with '?'.
        if raw_seq.starts_with(b"\x1b[?") && (terminator == b'h' || terminator == b'l') {
            let is_enter = terminator == b'h';
            let params_str = std::str::from_utf8(&raw_seq[3..raw_seq.len() - 1]).ok()?;

            let mut affects_alt = false;
            for param in params_str.split(';') {
                if param == "47" || param == "1047" || param == "1049" {
                    affects_alt = true;
                    break;
                }
            }

            if affects_alt {
                parts.push(ParsedPtyPart::Event(if is_enter {
                    ShellHookEvent::AltScreenEnter
                } else {
                    ShellHookEvent::AltScreenExit
                }));
            }
        }

        Some(parts)
    }

    /// Try to parse an OSC sequence at the start of pending.
    fn try_parse_osc(&mut self) -> Option<Vec<ParsedPtyPart>> {
        // OSC ends with BEL (\x07) or ST (\x1b\).
        let bel_pos = self.pending.iter().position(|&b| b == b'\x07');
        let st_pos = self.pending.windows(2).position(|w| w == b"\x1b\\");

        let end_pos = match (bel_pos, st_pos) {
            (Some(b), Some(s)) => Some(b.min(s)),
            (Some(b), None) => Some(b),
            (None, Some(s)) => Some(s),
            (None, None) => None,
        }?;

        let is_st = st_pos.is_some() && (bel_pos.is_none() || st_pos.unwrap() < bel_pos.unwrap());
        let end_len = if is_st { 2 } else { 1 };
        let raw_seq = self.pending.drain(..end_pos + end_len).collect::<Vec<u8>>();

        // Check if it's a Tide OSC 777 marker.
        if let Some(event) = parse_osc777_event(&raw_seq) {
            // Tide markers are stripped from the visible stream.
            return Some(vec![ParsedPtyPart::Event(event)]);
        }

        // Other OSC sequences are passed through.
        Some(vec![ParsedPtyPart::Visible(raw_seq)])
    }

    pub fn flush_visible(&mut self) -> Vec<u8> {
        self.pending.drain(..).collect()
    }
}

pub fn install_script() -> &'static str {
    r#"autoload -Uz add-zsh-hook

_tide_escape_osc() {
  printf '%s' "$1" | command od -An -tx1 -v | command tr -d ' \n'
}

_tide_preexec() {
  local cmd="$1"
  cmd="$(_tide_escape_osc "$cmd")"
  printf '\033]777;block_start;cmd=hex:%s\a' "$cmd"
}

_tide_emit_cwd() {
  local cwd="$PWD"
  cwd="$(_tide_escape_osc "$cwd")"
  printf '\033]777;cwd;cwd=hex:%s\a' "$cwd"
  printf '\033]7;file://%s%s\a' "${HOST:-localhost}" "$PWD"
}

_tide_precmd() {
  local ec=$?
  local cwd="$PWD"
  cwd="$(_tide_escape_osc "$cwd")"
  _tide_emit_cwd
  printf '\033]777;block_end;exit=%d;cwd=hex:%s\a' "$ec" "$cwd"
}

# Tide internal: force zle to redraw the prompt after leaving Block/Detail view.
# Bound to Ctrl-X Ctrl-R (unusual sequence, unlikely to conflict).
_tide_redraw_prompt() {
  zle reset-prompt
  zle -R
}
zle -N _tide_redraw_prompt 2>/dev/null
bindkey '^X^R' _tide_redraw_prompt 2>/dev/null

_tide_zle_line_init() {
  printf '\033]777;zle_ready\a'
}
zle -N zle-line-init _tide_zle_line_init 2>/dev/null

add-zsh-hook preexec _tide_preexec
add-zsh-hook precmd _tide_precmd
add-zsh-hook chpwd _tide_emit_cwd
"#
}

fn parse_osc777_event(bytes: &[u8]) -> Option<ShellHookEvent> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = if text.ends_with('\x07') {
        text.strip_prefix("\x1b]777;")?.strip_suffix('\x07')?
    } else {
        text.strip_prefix("\x1b]777;")?.strip_suffix("\x1b\\")?
    };

    parse_block_marker(text)
}

fn parse_block_marker(text: &str) -> Option<ShellHookEvent> {
    if text == "zle_ready" {
        return Some(ShellHookEvent::ZleReady);
    }

    if let Some(payload) = text.strip_prefix("block_start;cmd=") {
        return Some(ShellHookEvent::Preexec {
            command: decode_payload(payload)?,
        });
    }

    if let Some(payload) = text.strip_prefix("cwd;cwd=") {
        return Some(ShellHookEvent::CwdChanged {
            cwd: decode_payload(payload)?,
        });
    }

    let payload = text.strip_prefix("block_end;")?;
    let mut exit_code = None;
    let mut cwd = None;
    for part in payload.split(';') {
        if let Some(value) = part.strip_prefix("exit=") {
            exit_code = Some(value.parse().unwrap_or(-1));
        } else if let Some(value) = part.strip_prefix("cwd=") {
            cwd = Some(decode_payload(value)?);
        }
    }

    Some(ShellHookEvent::Precmd {
        exit_code: exit_code.unwrap_or(-1),
        cwd,
    })
}

fn decode_payload(payload: &str) -> Option<String> {
    let Some(hex) = payload.strip_prefix("hex:") else {
        return Some(payload.to_string());
    };

    if hex.len() % 2 != 0 {
        return None;
    }

    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for index in (0..hex.len()).step_by(2) {
        let byte = u8::from_str_radix(&hex[index..index + 2], 16).ok()?;
        bytes.push(byte);
    }

    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::{Osc777Parser, ParsedPtyPart, ShellHookEvent, install_script};

    #[test]
    fn install_script_contains_additive_hooks_without_visible_ui() {
        let script = install_script();

        assert!(script.contains("add-zsh-hook preexec _tide_preexec"));
        assert!(script.contains("add-zsh-hook precmd _tide_precmd"));
        assert!(script.contains("add-zsh-hook chpwd _tide_emit_cwd"));
        assert!(script.contains("file://"));
        assert!(script.contains("block_start"));
        assert!(script.contains("block_end"));
        assert!(script.contains("zle-line-init"));
        assert!(script.contains("zle_ready"));
        assert!(!script.contains("PROMPT="));
        assert!(!script.contains("RPROMPT="));
    }

    #[test]
    fn strips_zle_ready_marker() {
        let mut parser = Osc777Parser::default();
        let parsed = parser.push(b"\x1b]777;zle_ready\x07");

        assert_eq!(parsed, vec![ParsedPtyPart::Event(ShellHookEvent::ZleReady)]);
    }

    #[test]
    fn strips_block_start_marker_from_visible_output() {
        let mut parser = Osc777Parser::default();
        let parsed = parser.push(b"hello\x1b]777;block_start;cmd=hex:6563686f206869\x07world");

        assert_eq!(
            parsed,
            vec![
                ParsedPtyPart::Visible(b"hello".to_vec()),
                ParsedPtyPart::Event(ShellHookEvent::Preexec {
                    command: "echo hi".to_string(),
                }),
                ParsedPtyPart::Visible(b"world".to_vec()),
            ]
        );
    }

    #[test]
    fn strips_block_end_marker_from_visible_output() {
        let mut parser = Osc777Parser::default();
        let parsed = parser.push(b"\x1b]777;block_end;exit=1;cwd=hex:2f746d70\x07");

        assert_eq!(
            parsed,
            vec![ParsedPtyPart::Event(ShellHookEvent::Precmd {
                exit_code: 1,
                cwd: Some("/tmp".to_string()),
            })]
        );
    }

    #[test]
    fn strips_cwd_marker_from_visible_output() {
        let mut parser = Osc777Parser::default();
        let parsed =
            parser.push(b"\x1b]777;cwd;cwd=hex:2f55736572732f626f622f50726f6a65637473\x07");

        assert_eq!(
            parsed,
            vec![ParsedPtyPart::Event(ShellHookEvent::CwdChanged {
                cwd: "/Users/bob/Projects".to_string(),
            })]
        );
    }

    #[test]
    fn passes_osc7_cwd_sequence_through() {
        let mut parser = Osc777Parser::default();
        let raw = b"\x1b]7;file://host/Users/bob/Projects\x07";
        let parsed = parser.push(raw);

        assert_eq!(parsed, vec![ParsedPtyPart::Visible(raw.to_vec())]);
    }

    #[test]
    fn handles_split_hook_event() {
        let mut parser = Osc777Parser::default();

        let first = parser.push(b"abc\x1b]777;block_st");
        assert_eq!(first, vec![ParsedPtyPart::Visible(b"abc".to_vec())]);

        let second = parser.push(b"art;cmd=hex:6563686f206869\x07def");
        assert_eq!(
            second,
            vec![
                ParsedPtyPart::Event(ShellHookEvent::Preexec {
                    command: "echo hi".to_string(),
                }),
                ParsedPtyPart::Visible(b"def".to_vec()),
            ]
        );
    }

    #[test]
    fn does_not_delay_normal_output() {
        let mut parser = Osc777Parser::default();

        let parsed = parser.push(b"prompt> ");
        assert_eq!(parsed, vec![ParsedPtyPart::Visible(b"prompt> ".to_vec())]);
    }

    #[test]
    fn decodes_command_with_semicolon_and_newline() {
        let mut parser = Osc777Parser::default();
        let parsed = parser.push(b"\x1b]777;block_start;cmd=hex:6563686f2068693b0a707764\x07");

        assert_eq!(
            parsed,
            vec![ParsedPtyPart::Event(ShellHookEvent::Preexec {
                command: "echo hi;\npwd".to_string(),
            })]
        );
    }

    #[test]
    fn handles_multiple_events_in_one_chunk() {
        let mut parser = Osc777Parser::default();
        let parsed = parser.push(
            b"\x1b]777;block_start;cmd=hex:66616c7365\x07out\x1b]777;block_end;exit=1;cwd=hex:2f\x07",
        );

        assert_eq!(
            parsed,
            vec![
                ParsedPtyPart::Event(ShellHookEvent::Preexec {
                    command: "false".to_string(),
                }),
                ParsedPtyPart::Visible(b"out".to_vec()),
                ParsedPtyPart::Event(ShellHookEvent::Precmd {
                    exit_code: 1,
                    cwd: Some("/".to_string()),
                }),
            ]
        );
    }

    #[test]
    fn leaves_non_tide_osc_777_visible() {
        let mut parser = Osc777Parser::default();
        let parsed = parser.push(b"\x1b]777;not-tide\x07");

        assert_eq!(
            parsed,
            vec![ParsedPtyPart::Visible(b"\x1b]777;not-tide\x07".to_vec())]
        );
    }

    #[test]
    fn handles_fragmented_alt_screen_sequence() {
        let mut parser = Osc777Parser::default();

        // Split ESC [ ?
        let first = parser.push(b"out\x1b[");
        assert_eq!(first, vec![ParsedPtyPart::Visible(b"out".to_vec())]);

        // Split 1049h
        let second = parser.push(b"?10");
        assert_eq!(second, vec![]);

        let third = parser.push(b"49hmore");
        assert_eq!(
            third,
            vec![
                ParsedPtyPart::Visible(b"\x1b[?1049h".to_vec()),
                ParsedPtyPart::Event(ShellHookEvent::AltScreenEnter),
                ParsedPtyPart::Visible(b"more".to_vec())
            ]
        );
    }

    #[test]
    fn handles_batched_csi_private_parameters() {
        let mut parser = Osc777Parser::default();

        // Batched Enter: 1047;1048h -> AltScreenEnter (1047 wins)
        let parsed = parser.push(b"\x1b[?1047;1048h");
        assert_eq!(
            parsed,
            vec![
                ParsedPtyPart::Visible(b"\x1b[?1047;1048h".to_vec()),
                ParsedPtyPart::Event(ShellHookEvent::AltScreenEnter)
            ]
        );

        // Batched Exit: 47;1048;1049l -> AltScreenExit (47 and 1049 win)
        let parsed = parser.push(b"\x1b[?47;1048;1049l");
        assert_eq!(
            parsed,
            vec![
                ParsedPtyPart::Visible(b"\x1b[?47;1048;1049l".to_vec()),
                ParsedPtyPart::Event(ShellHookEvent::AltScreenExit)
            ]
        );
    }

    #[test]
    fn non_alt_screen_csi_preserved_as_raw_bytes() {
        let mut parser = Osc777Parser::default();
        let parsed = parser.push(b"\x1b[?1048h");
        assert_eq!(
            parsed,
            vec![ParsedPtyPart::Visible(b"\x1b[?1048h".to_vec())]
        );
    }

    #[test]
    fn interleaved_text_and_csi() {
        let mut parser = Osc777Parser::default();
        let parsed = parser.push(b"text\x1b[?1049hmore");
        assert_eq!(
            parsed,
            vec![
                ParsedPtyPart::Visible(b"text".to_vec()),
                ParsedPtyPart::Visible(b"\x1b[?1049h".to_vec()),
                ParsedPtyPart::Event(ShellHookEvent::AltScreenEnter),
                ParsedPtyPart::Visible(b"more".to_vec()),
            ]
        );
    }

    #[test]
    fn malformed_long_csi_does_not_grow_unbounded() {
        let mut parser = Osc777Parser::default();
        let mut long = b"\x1b[?".to_vec();
        for _ in 0..600 {
            long.push(b'1');
        }
        let parsed = parser.push(&long);
        // Should have flushed at least the ESC at some point
        assert!(parsed.len() > 0);
        assert!(parser.pending.len() < long.len());
    }
}
