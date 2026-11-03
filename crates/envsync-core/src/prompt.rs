//! Reads a secret without echoing it: a console prompt, a Git Bash (mintty) prompt, or one
//! line from a pipe.

use std::io::{BufRead, IsTerminal};
use std::process::{Command, Stdio};

pub fn read_secret(prompt: &str) -> std::io::Result<String> {
    if std::io::stdin().is_terminal() {
        return rpassword::prompt_password(prompt);
    }
    // mintty hands native programs a pipe, not a console. `stty` only succeeds on a real
    // terminal, so it also tells a mintty prompt apart from `echo x | envsync set X`.
    if std::env::var_os("MSYSTEM").is_some() && stty("-echo") {
        eprint!("{prompt}");
        let line = read_line();
        stty("echo");
        eprintln!();
        return line;
    }
    read_line()
}

fn stty(arg: &str) -> bool {
    Command::new("stty")
        .arg(arg)
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn read_line() -> std::io::Result<String> {
    let mut s = String::new();
    std::io::stdin().lock().read_line(&mut s)?;
    while s.ends_with('\n') || s.ends_with('\r') {
        s.pop();
    }
    Ok(s)
}
