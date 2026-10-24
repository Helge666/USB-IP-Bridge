//! Minimal logger: timestamped lines to stderr and, optionally, a file.

use log::{LevelFilter, Log, Metadata, Record};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct Logger {
    file: Option<Mutex<File>>,
    level: LevelFilter,
}

fn timestamp() -> String {
    let d = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = d.as_secs();
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    format!("{h:02}:{m:02}:{s:02}.{:03}Z", d.subsec_millis())
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("{} {:5} {}\n", timestamp(), record.level(), record.args());
        let _ = std::io::stderr().write_all(line.as_bytes());
        if let Some(f) = &self.file {
            if let Ok(mut f) = f.lock() {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }

    fn flush(&self) {}
}

/// Installs the logger. `USBIPB_LOG=debug` raises the level.
pub fn init(file: Option<&Path>) {
    let level = match std::env::var("USBIPB_LOG").ok().as_deref() {
        Some("trace") => LevelFilter::Trace,
        Some("debug") => LevelFilter::Debug,
        Some("warn") => LevelFilter::Warn,
        _ => LevelFilter::Info,
    };
    let file = file.and_then(|p| {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        // Keep the log small: start a new one at 5 MB, keeping the previous one as ".1".
        if std::fs::metadata(p).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
            let _ = std::fs::rename(p, p.with_extension("log.1"));
        }
        OpenOptions::new().create(true).append(true).open(p).ok().map(Mutex::new)
    });
    if log::set_boxed_logger(Box::new(Logger { file, level })).is_ok() {
        log::set_max_level(level);
    }
}
