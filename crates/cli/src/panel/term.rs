//! Minimal terminal handling for the panel: raw keyboard input, screen size and drawing
//! with plain VT escape sequences (no extra libraries; Windows 10+ consoles understand
//! them, too). Everything drawn is ASCII.

use anyhow::{bail, Result};
use std::io::{Read, Write};
use std::sync::mpsc::{self, Receiver};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Enter,
    Esc,
    Tab,
    Backspace,
    CtrlC,
    Char(char),
}

/// The terminal in raw mode on the alternate screen; restored when dropped.
pub struct Terminal {
    #[cfg(unix)]
    saved: String,
    #[cfg(windows)]
    modes: (u32, u32),
    /// Bytes typed, read by a background thread.
    pub input: Receiver<Vec<u8>>,
    /// The last frame drawn, line by line (only changed lines are sent again).
    shown: Vec<String>,
}

#[cfg(unix)]
fn stty(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("stty").args(args).stdin(std::process::Stdio::inherit()).stderr(std::process::Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

impl Terminal {
    pub fn enter() -> Result<Self> {
        #[cfg(unix)]
        let saved = {
            let Some(saved) = stty(&["-g"]) else { bail!("the panel needs an interactive terminal") };
            if stty(&["raw", "-echo"]).is_none() {
                bail!("cannot switch the terminal to raw mode");
            }
            saved
        };
        #[cfg(windows)]
        let modes = {
            use windows::Win32::System::Console::*;
            unsafe {
                let (Ok(hin), Ok(hout)) = (GetStdHandle(STD_INPUT_HANDLE), GetStdHandle(STD_OUTPUT_HANDLE)) else { bail!("no console") };
                let (mut min, mut mout) = (CONSOLE_MODE(0), CONSOLE_MODE(0));
                if GetConsoleMode(hin, &mut min).is_err() || GetConsoleMode(hout, &mut mout).is_err() {
                    bail!("the panel needs an interactive console");
                }
                SetConsoleMode(hin, ENABLE_VIRTUAL_TERMINAL_INPUT)?;
                SetConsoleMode(hout, mout | ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING)?;
                (min.0, mout.0)
            }
        };
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            let mut stdin = std::io::stdin();
            while let Ok(n) = stdin.read(&mut buf) {
                if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                    break;
                }
            }
        });
        // alternate screen, hidden cursor
        print_flush("\x1b[?1049h\x1b[?25l\x1b[2J");
        Ok(Self {
            #[cfg(unix)]
            saved,
            #[cfg(windows)]
            modes,
            input: rx,
            shown: Vec::new(),
        })
    }

    /// Columns and rows of the window.
    pub fn size(&self) -> (usize, usize) {
        #[cfg(unix)]
        {
            #[repr(C)]
            struct WinSize {
                rows: u16,
                cols: u16,
                _x: u16,
                _y: u16,
            }
            extern "C" {
                fn ioctl(fd: i32, request: std::ffi::c_ulong, ...) -> i32;
            }
            #[cfg(target_os = "linux")]
            const TIOCGWINSZ: std::ffi::c_ulong = 0x5413;
            #[cfg(not(target_os = "linux"))]
            const TIOCGWINSZ: std::ffi::c_ulong = 0x4008_7468;
            let mut ws = WinSize { rows: 0, cols: 0, _x: 0, _y: 0 };
            if unsafe { ioctl(1, TIOCGWINSZ, &mut ws as *mut WinSize) } == 0 && ws.cols > 0 && ws.rows > 0 {
                return (ws.cols as usize, ws.rows as usize);
            }
        }
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Console::*;
            let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
            if let Ok(h) = GetStdHandle(STD_OUTPUT_HANDLE) {
                if GetConsoleScreenBufferInfo(h, &mut info).is_ok() {
                    let w = info.srWindow;
                    return ((w.Right - w.Left + 1).max(1) as usize, (w.Bottom - w.Top + 1).max(1) as usize);
                }
            }
        }
        (80, 24)
    }

    /// Draws a frame: one string per line, already styled (see `Canvas::lines`).
    pub fn draw(&mut self, lines: Vec<String>) {
        let mut out = String::new();
        if lines.len() != self.shown.len() {
            out.push_str("\x1b[0m\x1b[2J");
            self.shown.clear();
        }
        for (i, line) in lines.iter().enumerate() {
            if self.shown.get(i) != Some(line) {
                out.push_str(&format!("\x1b[{};1H{line}\x1b[0m", i + 1));
            }
        }
        self.shown = lines;
        if !out.is_empty() {
            print_flush(&out);
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        print_flush("\x1b[0m\x1b[2J\x1b[?25h\x1b[?1049l");
        #[cfg(unix)]
        stty(&[&self.saved]);
        #[cfg(windows)]
        unsafe {
            use windows::Win32::System::Console::*;
            if let (Ok(hin), Ok(hout)) = (GetStdHandle(STD_INPUT_HANDLE), GetStdHandle(STD_OUTPUT_HANDLE)) {
                let _ = SetConsoleMode(hin, CONSOLE_MODE(self.modes.0));
                let _ = SetConsoleMode(hout, CONSOLE_MODE(self.modes.1));
            }
        }
    }
}

fn print_flush(s: &str) {
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(s.as_bytes());
    let _ = out.flush();
}

/// Turns typed bytes into keys (VT sequences for arrows and the like).
pub fn parse(bytes: &[u8]) -> Vec<Key> {
    let c: Vec<char> = String::from_utf8_lossy(bytes).chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '\x1b' if i + 1 < c.len() && (c[i + 1] == '[' || c[i + 1] == 'O') => {
                let mut j = i + 2;
                while j < c.len() && (c[j].is_ascii_digit() || c[j] == ';') {
                    j += 1;
                }
                if j >= c.len() {
                    break;
                }
                let params: String = c[i + 2..j].iter().collect();
                let key = match (c[j], params.as_str()) {
                    ('A', _) => Some(Key::Up),
                    ('B', _) => Some(Key::Down),
                    ('C', _) => Some(Key::Right),
                    ('D', _) => Some(Key::Left),
                    ('H', _) => Some(Key::Home),
                    ('F', _) => Some(Key::End),
                    ('~', "1" | "7") => Some(Key::Home),
                    ('~', "4" | "8") => Some(Key::End),
                    ('~', "5") => Some(Key::PageUp),
                    ('~', "6") => Some(Key::PageDown),
                    ('P', _) => Some(Key::Char('?')), // F1
                    _ => None,
                };
                out.extend(key);
                i = j + 1;
            }
            '\x1b' => {
                out.push(Key::Esc);
                i += 1;
            }
            ch => {
                match ch {
                    '\r' => out.push(Key::Enter),
                    '\n' if i > 0 && c[i - 1] == '\r' => {}
                    '\n' => out.push(Key::Enter),
                    '\t' => out.push(Key::Tab),
                    '\x7f' | '\x08' => out.push(Key::Backspace),
                    '\x03' => out.push(Key::CtrlC),
                    ch if !ch.is_control() => out.push(Key::Char(ch)),
                    _ => {}
                }
                i += 1;
            }
        }
    }
    out
}

/// Text attributes (no colors: they read differently on every terminal).
pub const BOLD: u8 = 1;
pub const REVERSE: u8 = 2;
pub const DIM: u8 = 4;

/// Replaces everything that is not printable ASCII (umlauts are spelled out).
pub fn ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            ' '..='~' => out.push(ch),
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'Ä' => out.push_str("Ae"),
            'Ö' => out.push_str("Oe"),
            'Ü' => out.push_str("Ue"),
            'ß' => out.push_str("ss"),
            'é' | 'è' | 'ê' => out.push('e'),
            'á' | 'à' | 'â' => out.push('a'),
            '–' | '—' | '·' => out.push('-'),
            '…' => out.push_str("..."),
            '→' => out.push_str("->"),
            '×' => out.push('x'),
            '„' | '“' | '”' | '«' | '»' => out.push('"'),
            '‘' | '’' | '‚' => out.push('\''),
            '\t' | '\n' | '\r' => out.push(' '),
            '\u{a0}' => out.push(' '),
            _ => out.push('?'),
        }
    }
    out
}

/// A screen being composed: characters with attributes.
pub struct Canvas {
    pub w: usize,
    pub h: usize,
    cells: Vec<(u8, u8)>,
}

impl Canvas {
    pub fn new(w: usize, h: usize) -> Self {
        Self { w, h, cells: vec![(b' ', 0); w * h] }
    }

    /// Writes text at (x, y), cut at the right edge; returns the column after it.
    pub fn put(&mut self, x: usize, y: usize, text: &str, attr: u8) -> usize {
        self.put_max(x, y, text, attr, usize::MAX)
    }

    /// Like `put`, at most `max` columns.
    pub fn put_max(&mut self, x: usize, y: usize, text: &str, attr: u8, max: usize) -> usize {
        if y >= self.h {
            return x;
        }
        let mut col = x;
        for b in ascii(text).bytes().take(max) {
            if col >= self.w {
                break;
            }
            self.cells[y * self.w + col] = (b, attr);
            col += 1;
        }
        col
    }

    /// Fills a rectangle with spaces of the given attribute.
    pub fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, attr: u8) {
        for row in y..(y + h).min(self.h) {
            for col in x..(x + w).min(self.w) {
                self.cells[row * self.w + col] = (b' ', attr);
            }
        }
    }

    /// A frame drawn with + - |, interior cleared; `title` goes into the top line.
    pub fn frame(&mut self, x: usize, y: usize, w: usize, h: usize, title: &str, title_attr: u8) {
        if w < 2 || h < 2 {
            return;
        }
        self.fill(x, y, w, h, 0);
        let line = format!("+{}+", "-".repeat(w - 2));
        self.put(x, y, &line, 0);
        self.put(x, y + h - 1, &line, 0);
        for row in y + 1..y + h - 1 {
            self.put(x, row, "|", 0);
            self.put(x + w - 1, row, "|", 0);
        }
        if !title.is_empty() && w > 6 {
            let end = self.put_max(x + 2, y, &format!(" {title} "), title_attr, w - 4);
            let _ = end;
        }
    }

    /// The lines with escape sequences for the attributes.
    pub fn lines(&self) -> Vec<String> {
        (0..self.h)
            .map(|y| {
                let mut s = String::with_capacity(self.w + 16);
                let mut cur = 0u8;
                for &(b, attr) in &self.cells[y * self.w..(y + 1) * self.w] {
                    if attr != cur {
                        s.push_str("\x1b[0m");
                        if attr & BOLD != 0 {
                            s.push_str("\x1b[1m");
                        }
                        if attr & DIM != 0 {
                            s.push_str("\x1b[2m");
                        }
                        if attr & REVERSE != 0 {
                            s.push_str("\x1b[7m");
                        }
                        cur = attr;
                    }
                    s.push(b as char);
                }
                s
            })
            .collect()
    }
}

/// Splits text into lines of at most `width` columns (at spaces where possible).
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    for para in ascii(text).split('\n') {
        let mut line = String::new();
        for word in para.split(' ') {
            let mut word = word.to_string();
            while word.len() > width {
                if !line.is_empty() {
                    out.push(std::mem::take(&mut line));
                }
                let rest = word.split_off(width);
                out.push(word);
                word = rest;
            }
            if !line.is_empty() && line.len() + 1 + word.len() > width {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(&word);
        }
        out.push(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(parse(b"\x1b[A\x1b[B\x1bOC\x1b[5~q\r\n"), vec![Key::Up, Key::Down, Key::Right, Key::PageUp, Key::Char('q'), Key::Enter]);
        assert_eq!(parse(b"\x1b"), vec![Key::Esc]);
    }

    #[test]
    fn ascii_only() {
        assert_eq!(ascii("Gerät – läuft…"), "Geraet - laeuft...");
        assert!(ascii("Ω日本").bytes().all(|b| b.is_ascii()));
    }

    #[test]
    fn wrapping() {
        assert_eq!(wrap("eins zwei drei vier", 10), vec!["eins zwei", "drei vier"]);
        assert_eq!(wrap("abcdefghijkl", 8), vec!["abcdefgh", "ijkl"]);
    }
}
