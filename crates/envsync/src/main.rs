//! envsync — store secrets as Windows user environment variables and share them with WSL.

use envsync_core::cli::{set_secret, unset_secret};
use envsync_core::install::{find_on_path, print_uninstall};
use envsync_core::ui::{bad, die, dim, info, ok};
use envsync_core::{valid_name, Store, VERSION};

fn usage() {
    println!(
        "envsync {VERSION} — Windows user environment variables, shared with WSL.

Usage: envsync <command> [args]

Commands
  set NAME              store a value (hidden prompt, or one line on stdin) in your
                        Windows user environment and share it with WSL (WSLENV)
  unset NAME            remove it from the user environment and WSLENV
  has NAME              exit 0 if NAME is set in the user environment, 1 if not
  list                  variable names in your user environment (values are never shown)
  wslenv [NAME...]      list the names shared with WSL, or share the given names
  uninstall [--force]   print the command that removes envsync
  agent-instructions    how AI agents should use envsync (Markdown)
  version

Values are never printed. Reopen terminals and editors after a change;
in WSL, run 'wsl --shutdown' once first."
    );
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");
    let rest = &args[args.len().min(1)..];
    let store = Store::open();
    match cmd {
        "help" | "-h" | "--help" => usage(),
        "version" | "-v" | "--version" => println!("envsync {VERSION}"),
        "agent-instructions" | "--agent-instructions" => print!("{}", include_str!("agent-instructions.md")),
        "set" => cmd_set(&store, rest),
        "unset" | "rm" => cmd_unset(&store, rest),
        "has" => cmd_has(&store, rest),
        "list" | "ls" => cmd_list(&store),
        "wslenv" => cmd_wslenv(&store, rest),
        "uninstall" => cmd_uninstall(rest),
        _ => die(&format!("unknown command: {cmd} (see: envsync help)")),
    }
}

fn one_name<'a>(rest: &'a [String], usage: &str) -> &'a str {
    match rest {
        [n] if valid_name(n) => n,
        _ => die(&format!("usage: {usage}")),
    }
}

fn cmd_set(store: &Store, rest: &[String]) {
    set_secret(store, one_name(rest, "envsync set NAME   (value is read from a hidden prompt or stdin)"));
}

fn cmd_unset(store: &Store, rest: &[String]) {
    unset_secret(store, one_name(rest, "envsync unset NAME"));
}

fn cmd_has(store: &Store, rest: &[String]) {
    let n = one_name(rest, "envsync has NAME");
    if store.is_set(n).unwrap_or_else(|e| die(&e)) {
        ok(&format!("{n} is set"));
    } else {
        bad(&format!("{n} is not set   → envsync set {n}"));
        std::process::exit(1);
    }
}

fn cmd_list(store: &Store) {
    let mut names = store.names().unwrap_or_else(|e| die(&e));
    names.sort_by_key(|n| n.to_ascii_lowercase());
    let shared = store.wslenv().unwrap_or_else(|e| die(&e));
    let width = names.iter().map(String::len).max().unwrap_or(4).max(4);
    println!("{:<width$}  WSL", "NAME");
    for n in &names {
        let wsl = if shared.iter().any(|s| s.eq_ignore_ascii_case(n)) { "shared" } else { "-" };
        println!("{n:<width$}  {wsl}");
    }
}

fn cmd_wslenv(store: &Store, rest: &[String]) {
    if rest.is_empty() {
        for n in store.wslenv().unwrap_or_else(|e| die(&e)) {
            println!("{n}");
        }
        return;
    }
    if let Some(bad_name) = rest.iter().find(|n| !valid_name(n)) {
        die(&format!("not a valid variable name: {bad_name}"));
    }
    let names: Vec<&str> = rest.iter().map(String::as_str).collect();
    let added = store.wslenv_add(&names).unwrap_or_else(|e| die(&e));
    if added.is_empty() {
        ok("already shared with WSL");
    } else {
        ok(&format!("WSLENV now shares: {}", added.join(" ")));
        info(&dim("Run 'wsl --shutdown' and reopen WSL to pick it up."));
    }
}

fn cmd_uninstall(rest: &[String]) {
    let force = rest.iter().any(|a| a == "--force");
    if let Some(a) = rest.iter().find(|a| *a != "--force") {
        die(&format!("unknown option: {a}"));
    }
    if !force {
        let sibling = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("mcpsync.exe")));
        if sibling.is_some_and(|p| p.is_file()) || find_on_path("mcpsync").is_some() {
            die("mcpsync is installed and needs envsync. Uninstall mcpsync first (mcpsync uninstall), or use: envsync uninstall --force");
        }
    }
    print_uninstall("envsync").unwrap_or_else(|e| die(&e));
}
