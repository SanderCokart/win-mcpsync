//! Status lines on stderr, coloured when stderr is a console.

use std::io::IsTerminal;
use std::sync::OnceLock;

fn color() -> bool {
    static C: OnceLock<bool> = OnceLock::new();
    *C.get_or_init(|| std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal() && enable_vt())
}

#[cfg(windows)]
fn enable_vt() -> bool {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_ERROR_HANDLE,
    };
    // SAFETY: plain console API calls on this process's own stderr handle.
    unsafe {
        let h = GetStdHandle(STD_ERROR_HANDLE);
        let mut mode = 0;
        GetConsoleMode(h, &mut mode) != 0 && SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0
    }
}

#[cfg(not(windows))]
fn enable_vt() -> bool {
    true
}

fn paint(code: &str, s: &str) -> String {
    if color() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn dim(s: &str) -> String {
    paint("2", s)
}

pub fn bold(s: &str) -> String {
    paint("1", s)
}

pub fn info(msg: &str) {
    eprintln!("{msg}");
}

pub fn ok(msg: &str) {
    eprintln!("{} {msg}", paint("32", "✓"));
}

pub fn warn(msg: &str) {
    eprintln!("{} {msg}", paint("33", "!"));
}

pub fn bad(msg: &str) {
    eprintln!("{} {msg}", paint("31", "✗"));
}

pub fn die(msg: &str) -> ! {
    eprintln!("{} {msg}", paint("31", "error:"));
    std::process::exit(1);
}
