use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellHookEvent {
    Preexec { command: String },
    Precmd { exit_code: i32 },
    Cwd { cwd: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedPtyPart {
    Visible(Vec<u8>),
    Event(ShellHookEvent),
}

static HOOK_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct TempHookFiles {
    dir: PathBuf,
}

impl TempHookFiles {
    pub fn new() -> std::io::Result<Self> {
        let dir = {
            let pid = std::process::id();
            let counter = HOOK_DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
            std::env::temp_dir().join(format!("tide-{}-{}", pid, counter))
        };
        fs::create_dir_all(&dir)?;

        let hook_file = dir.join("tide-hooks.zsh");
        fs::write(&hook_file, install_script())?;

        let original_zdotdir = std::env::var("ZDOTDIR")
            .ok()
            .and_then(|s| if s.is_empty() { None } else { Some(s) })
            .unwrap_or_else(|| {
                std::env::var("HOME").unwrap_or_else(|_| String::new())
            });

        let zshenv_content = format!(
            "[[ -f '{}'/.zshenv ]] && source '{}'/.zshenv\n",
            escape_single_quotes(&original_zdotdir),
            escape_single_quotes(&original_zdotdir),
        );
        fs::write(dir.join(".zshenv"), zshenv_content)?;

        let zshrc_content = format!(
            "export ZDOTDIR='{}'\n[[ -f $ZDOTDIR/.zshrc ]] && source $ZDOTDIR/.zshrc\nsource '{}'\n",
            escape_single_quotes(&original_zdotdir),
            escape_single_quotes(hook_file.to_str().unwrap_or(""))
        );
        fs::write(dir.join(".zshrc"), zshrc_content)?;

        Ok(Self { dir })
    }

    pub fn zdotdir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for TempHookFiles {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn escape_single_quotes(s: &str) -> String {
    s.replace('\'', "'\\''")
}

#[derive(Debug, Default)]
pub struct Osc777Parser {
    pending: Vec<u8>,
}

impl Osc777Parser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<ParsedPtyPart> {
        self.pending.extend_from_slice(bytes);

        let mut parts = Vec::new();

        loop {
            let Some(start) = find_subsequence(&self.pending, marker()) else {
                let keep = marker_prefix_tail_len(&self.pending);
                let drain_until = self.pending.len().saturating_sub(keep);
                push_visible_part(&mut parts, self.pending.drain(..drain_until));
                break;
            };

            push_visible_part(&mut parts, self.pending.drain(..start));

            let Some(end) = self.pending.iter().position(|byte| *byte == b'\x07') else {
                break;
            };

            let raw_event = self.pending.drain(..=end).collect::<Vec<_>>();
            if let Some(event) = parse_osc777_event(&raw_event) {
                parts.push(ParsedPtyPart::Event(event));
            }
        }

        parts
    }

    pub fn flush_visible(&mut self) -> Vec<u8> {
        self.pending.drain(..).collect()
    }
}

pub fn install_script() -> &'static str {
    r#"autoload -Uz add-zsh-hook
_tide_hex_encode() {
  command od -An -tx1 -v | command tr -d ' \n'
}
_tide_preexec() {
  local payload
  payload=$(printf '%s' "$1" | _tide_hex_encode)
  print -rn -- $'\e]777;tide;preexec;hex:'"${payload}"$'\a'
}
_tide_precmd() {
  local exit_code=$?
  local payload
  payload=$(printf '%s' "${exit_code}" | _tide_hex_encode)
  print -rn -- $'\e]777;tide;precmd;hex:'"${payload}"$'\a'
}
_tide_chpwd() {
  local payload
  payload=$(printf '%s' "$PWD" | _tide_hex_encode)
  print -rn -- $'\e]777;tide;cwd;hex:'"${payload}"$'\a'
}
add-zsh-hook preexec _tide_preexec
add-zsh-hook precmd _tide_precmd
add-zsh-hook chpwd _tide_chpwd
"#
}

fn parse_osc777_event(bytes: &[u8]) -> Option<ShellHookEvent> {
    let text = std::str::from_utf8(bytes).ok()?;
    let text = text.strip_prefix("\x1b]777;tide;")?.strip_suffix('\x07')?;

    let (kind, payload) = text.split_once(';')?;

    let payload = decode_payload(payload)?;

    match kind {
        "preexec" => Some(ShellHookEvent::Preexec { command: payload }),
        "precmd" => Some(ShellHookEvent::Precmd {
            exit_code: payload.parse().unwrap_or(-1),
        }),
        "cwd" => Some(ShellHookEvent::Cwd { cwd: payload }),
        _ => None,
    }
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

fn marker() -> &'static [u8] {
    b"\x1b]777;tide;"
}

fn find_subsequence(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn marker_prefix_tail_len(bytes: &[u8]) -> usize {
    let marker = marker();
    let max_len = bytes.len().min(marker.len().saturating_sub(1));

    for len in (1..=max_len).rev() {
        if bytes[bytes.len() - len..] == marker[..len] {
            return len;
        }
    }

    0
}

fn push_visible_part(parts: &mut Vec<ParsedPtyPart>, bytes: impl Iterator<Item = u8>) {
    let bytes = bytes.collect::<Vec<_>>();
    if !bytes.is_empty() {
        parts.push(ParsedPtyPart::Visible(bytes));
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Osc777Parser, ParsedPtyPart, ShellHookEvent, TempHookFiles,
        escape_single_quotes,
    };
    use std::fs;

    #[test]
    fn temp_hook_files_create_dir_and_files() {
        let hooks = TempHookFiles::new().expect("create temp hook files");

        assert!(hooks.zdotdir().exists());
        assert!(hooks.zdotdir().join(".zshenv").exists());
        assert!(hooks.zdotdir().join(".zshrc").exists());
        assert!(hooks.zdotdir().join("tide-hooks.zsh").exists());
    }

    #[test]
    fn temp_hook_files_zshrc_sources_original_and_hooks() {
        let hooks = TempHookFiles::new().expect("create temp hook files");

        let zshrc = fs::read_to_string(hooks.zdotdir().join(".zshrc"))
            .expect("read .zshrc");

        assert!(zshrc.contains("export ZDOTDIR="));
        assert!(zshrc.contains("source $ZDOTDIR/.zshrc"));
        assert!(zshrc.contains("source '"));
        assert!(zshrc.contains("tide-hooks.zsh"));
    }

    #[test]
    fn temp_hook_files_zshenv_sources_original() {
        let hooks = TempHookFiles::new().expect("create temp hook files");

        let zshenv = fs::read_to_string(hooks.zdotdir().join(".zshenv"))
            .expect("read .zshenv");

        assert!(!zshenv.contains("export ZDOTDIR="), ".zshenv must not change ZDOTDIR");
        assert!(zshenv.contains(".zshenv"), "must source original .zshenv");
    }

    #[test]
    fn temp_hook_files_hook_file_contains_preexec() {
        let hooks = TempHookFiles::new().expect("create temp hook files");

        let hook_script = fs::read_to_string(hooks.zdotdir().join("tide-hooks.zsh"))
            .expect("read tide-hooks.zsh");

        assert!(hook_script.contains("_tide_preexec"));
        assert!(hook_script.contains("_tide_precmd"));
        assert!(hook_script.contains("_tide_chpwd"));
        assert!(hook_script.contains("add-zsh-hook"));
    }

    #[test]
    fn temp_hook_files_cleanup_on_drop() {
        let dir: std::path::PathBuf;
        {
            let hooks = TempHookFiles::new().expect("create temp hook files");
            dir = hooks.zdotdir().to_path_buf();
            assert!(dir.exists());
        }
        assert!(!dir.exists());
    }

    #[test]
    fn escape_single_quotes_handles_plain_string() {
        assert_eq!(escape_single_quotes("hello"), "hello");
    }

    #[test]
    fn escape_single_quotes_handles_embedded_quote() {
        assert_eq!(
            escape_single_quotes("it's working"),
            "it'\\''s working"
        );
    }

    #[test]
    fn strips_hook_event_from_visible_output() {
        let mut parser = Osc777Parser::default();
        let parsed = parser.push(b"hello\x1b]777;tide;precmd;hex:30\x07world");

        assert_eq!(
            parsed,
            vec![
                ParsedPtyPart::Visible(b"hello".to_vec()),
                ParsedPtyPart::Event(ShellHookEvent::Precmd { exit_code: 0 }),
                ParsedPtyPart::Visible(b"world".to_vec()),
            ]
        );
    }

    #[test]
    fn handles_split_hook_event() {
        let mut parser = Osc777Parser::default();

        let first = parser.push(b"abc\x1b]777;tide;pre");
        assert_eq!(first, vec![ParsedPtyPart::Visible(b"abc".to_vec())]);

        let second = parser.push(b"exec;hex:6563686f206869\x07def");
        assert_eq!(
            second,
            vec![
                ParsedPtyPart::Event(ShellHookEvent::Preexec {
                    command: "echo hi".to_string()
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
        let parsed = parser.push(b"\x1b]777;tide;preexec;hex:6563686f2068693b0a707764\x07");

        assert_eq!(
            parsed,
            vec![ParsedPtyPart::Event(ShellHookEvent::Preexec {
                command: "echo hi;\npwd".to_string()
            })]
        );
    }

    #[test]
    fn handles_multiple_events_in_one_chunk() {
        let mut parser = Osc777Parser::default();
        let parsed = parser
            .push(b"\x1b]777;tide;preexec;hex:66616c7365\x07out\x1b]777;tide;precmd;hex:31\x07");

        assert_eq!(
            parsed,
            vec![
                ParsedPtyPart::Event(ShellHookEvent::Preexec {
                    command: "false".to_string()
                }),
                ParsedPtyPart::Visible(b"out".to_vec()),
                ParsedPtyPart::Event(ShellHookEvent::Precmd { exit_code: 1 }),
            ]
        );
    }
}
