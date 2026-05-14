use std::{
    fmt::Display,
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    time::Instant,
};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy)]
pub enum LogLevel {
    INFO,
    DEBUG,
    TRACE,
}

impl Display for LogLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogLevel::INFO => write!(f, "INFO"),
            LogLevel::DEBUG => write!(f, "DEBUG"),
            LogLevel::TRACE => write!(f, "TRACE"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LogCategory {
    PTY,
    HOOK,
    APP,
    RENDER,
    AGENT,
}

impl Display for LogCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LogCategory::PTY => write!(f, "PTY"),
            LogCategory::HOOK => write!(f, "HOOK"),
            LogCategory::APP => write!(f, "APP"),
            LogCategory::RENDER => write!(f, "RENDER"),
            LogCategory::AGENT => write!(f, "AGENT"),
        }
    }
}

/// Per-session debug log.  Created when `TIDE_DEBUG=1` is set.
///
/// Each Tide session writes to a separate file named
/// `$TMPDIR/tide-debug-<pid>-<unix-ts>.log`.
///
/// Log lines have the format:
///   `[+<ms>ms] <message>`
/// where `ms` is milliseconds since session start.
pub struct DebugLog {
    writer: BufWriter<File>,
    start: Instant,
    pub path: PathBuf,
}

impl DebugLog {
    /// Open a new log file if `TIDE_DEBUG=1`.  Returns `None` otherwise.
    ///
    /// Files are written to `<project-root>/debug/tide-<pid>-<unix-ts>.log`.
    /// The directory is created if it does not exist.
    pub fn open_if_enabled() -> Option<Self> {
        if std::env::var_os("TIDE_DEBUG").is_none() {
            return None;
        }
        // Resolve project root relative to the binary's location.
        // In development `cargo run` sets the cwd to the workspace root.
        let dir = std::env::current_dir().ok()?.join("debug");
        std::fs::create_dir_all(&dir).ok()?;

        Self::cleanup_old_logs(&dir);

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = dir.join(format!("tide-{}-{}.log", std::process::id(), ts));
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .ok()?;
        Some(Self {
            writer: BufWriter::new(file),
            start: Instant::now(),
            path,
        })
    }

    fn cleanup_old_logs(dir: &std::path::Path) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        let mut logs = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                    if name.starts_with("tide-") && name.ends_with(".log") {
                        if let Ok(metadata) = entry.metadata() {
                            if let Ok(mtime) = metadata.modified() {
                                logs.push((path, mtime));
                            }
                        }
                    }
                }
            }
        }

        if logs.len() < 10 {
            return;
        }

        // Sort by modification time, oldest first
        logs.sort_by_key(|&(_, mtime)| mtime);

        let to_delete = logs.len().saturating_sub(9);
        for (path, _) in logs.iter().take(to_delete) {
            let _ = std::fs::remove_file(path);
        }
    }

    pub fn log(&mut self, msg: &str) {
        let ms = self.start.elapsed().as_millis();
        let _ = writeln!(self.writer, "[+{}ms] {}", ms, msg);
        // Flush so the file is readable mid-session.
        let _ = self.writer.flush();
    }

    pub fn log_structured(
        &mut self,
        level: LogLevel,
        cat: LogCategory,
        msg: &str,
        context: Option<&str>,
    ) {
        let ms = self.start.elapsed().as_millis();
        let _ = write!(self.writer, "[+{}ms][{}][{}] {}", ms, level, cat, msg);
        if let Some(c) = context {
            let _ = write!(self.writer, " | Context: {{{}}}", c);
        }
        let _ = writeln!(self.writer);
        let _ = self.writer.flush();
    }

    /// Escapes non-printable ASCII characters for trace logging.
    pub fn escape_bytes(bytes: &[u8]) -> String {
        let mut escaped = String::with_capacity(bytes.len() * 4);
        for &b in bytes {
            if (0x20..=0x7E).contains(&b) {
                escaped.push(b as char);
            } else {
                match b {
                    b'\n' => escaped.push_str("\\n"),
                    b'\r' => escaped.push_str("\\r"),
                    b'\t' => escaped.push_str("\\t"),
                    _ => {
                        let _ = write!(escaped, "\\x{:02x}", b);
                    }
                }
            }
        }
        escaped
    }
}

/// Write an INFO level structured log entry.
#[macro_export]
macro_rules! tinfo {
    ($log:expr, $cat:expr, $($arg:tt)*) => {
        if let Some(ref mut __log) = $log {
            __log.log_structured(
                $crate::debug_log::LogLevel::INFO,
                $cat,
                &format!($($arg)*),
                None,
            );
        }
    };
}

/// Write a DEBUG level structured log entry with JSON context.
#[macro_export]
macro_rules! tdebug {
    ($log:expr, $cat:expr, $msg:expr, $context:expr) => {
        if let Some(ref mut __log) = $log {
            let __ctx_json = serde_json::to_string($context).unwrap_or_else(|_| "{}".to_string());
            __log.log_structured(
                $crate::debug_log::LogLevel::DEBUG,
                $cat,
                $msg,
                Some(&__ctx_json),
            );
        }
    };
}

/// Write a TRACE level structured log entry for byte sequences.
#[macro_export]
macro_rules! ttrace {
    ($log:expr, $cat:expr, $msg:expr, $bytes:expr) => {
        if let Some(ref mut __log) = $log {
            let __escaped = $crate::debug_log::DebugLog::escape_bytes($bytes);
            __log.log_structured(
                $crate::debug_log::LogLevel::TRACE,
                $cat,
                $msg,
                Some(&__escaped),
            );
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_cleanup() {
        let temp_dir = std::env::temp_dir().join("tide_test_logs");
        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Create 12 dummy log files
        for i in 0..12 {
            let path = temp_dir.join(format!("tide-test-{}.log", i));
            std::fs::write(&path, "test").unwrap();
            // Sleep briefly to ensure different modification times
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        // Call the cleanup method
        DebugLog::cleanup_old_logs(&temp_dir);

        // Verify that exactly 9 files remain (since it's intended to leave room for the 10th)
        let remaining = std::fs::read_dir(&temp_dir).unwrap().count();
        assert_eq!(remaining, 9, "Should have 9 logs remaining");

        // Cleanup
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_escape_bytes() {
        let input = b"Hello\nWorld\r\t\x01\x7F";
        let escaped = DebugLog::escape_bytes(input);
        assert_eq!(escaped, "Hello\\nWorld\\r\\t\\x01\\x7f");
    }

    #[test]
    fn test_macros_compilation() {
        // This test primarily ensures the macros compile and can be called.
        // We use a mock-like approach or just check if it compiles.
        let mut log = DebugLog::open_if_enabled(); // Likely None in tests unless env set
        
        tinfo!(log, LogCategory::APP, "Test info message");
        tinfo!(log, LogCategory::APP, "Test info message with arg: {}", 42);
        
        let context = serde_json::json!({"key": "value"});
        tdebug!(log, LogCategory::APP, "Test debug message", &context);
        
        ttrace!(log, LogCategory::APP, "Test trace message", b"data\x00");
    }
}
