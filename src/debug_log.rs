use std::{
    fs::{File, OpenOptions},
    io::{BufWriter, Write},
    path::PathBuf,
    time::Instant,
};

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

    pub fn log(&mut self, msg: &str) {
        let ms = self.start.elapsed().as_millis();
        let _ = writeln!(self.writer, "[+{}ms] {}", ms, msg);
        // Flush so the file is readable mid-session.
        let _ = self.writer.flush();
    }
}

/// Write a debug log entry if the log is present.
#[macro_export]
macro_rules! dlog {
    ($log:expr, $($arg:tt)*) => {
        if let Some(ref mut __log) = $log {
            __log.log(&format!($($arg)*));
        }
    };
}
