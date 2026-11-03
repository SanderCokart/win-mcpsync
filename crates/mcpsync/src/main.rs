//! mcpsync — define your MCP servers once, sync them to every editor you use (Windows).
//!
//! Source of truth : ~/.config/mcpsync/mcp.json   (or <project>/.mcpsync.json with --project)
//! Settings        : ~/.config/mcpsync/config      (which editors you use, fallback policy, ...)
//! Secrets         : referenced as ${VAR}; values live in the Windows user environment,
//!                   managed by envsync (shared library, also installed as envsync.exe)

mod config;
mod engine;
mod toml;

use config::Settings;
use engine::{Gen, ImportNote, Plan, SUPPORTED_EDITORS};
use envsync_core::cli::{set_secret, unset_secret};
use envsync_core::install::{find_on_path, print_uninstall};
use envsync_core::ui::{bad, bold, die, dim, info, ok, warn};
use envsync_core::{valid_name, Store, VERSION};
use serde_json::Value;
use std::io::{BufRead, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::Command;

struct Ctx {
    project: Option<PathBuf>,
    source: PathBuf,
    config_file: PathBuf,
    state_file: PathBuf,
    backup_dir: PathBuf,
    state_prefix: String,
    s: Settings,
}

fn main() {
    let mut project: Option<PathBuf> = None;
    let mut args: Vec<String> = Vec::new();
    for a in std::env::args().skip(1) {
        if a == "--project" {
            project = Some(std::env::current_dir().unwrap_or_else(|e| die(&e.to_string())));
        } else if let Some(d) = a.strip_prefix("--project=") {
            project = Some(config::win_path(d));
        } else {
            args.push(a);
        }
    }
    let cmd = args.first().cloned().unwrap_or_else(|| "help".into());
    let rest: Vec<String> = args.into_iter().skip(1).collect();
    match cmd.as_str() {
        "help" | "-h" | "--help" => return usage(),
        "version" | "-v" | "--version" => return println!("mcpsync {VERSION}"),
        "agent-instructions" | "--agent-instructions" => return print!("{}", include_str!("agent-instructions.md")),
        "uninstall" => return cmd_uninstall(&rest),
        _ => {}
    }

    let home = config::mcpsync_home();
    let config_file = home.join("config");
    let mut s = Settings::load(&config_file);
    let mut source = config::source_override().unwrap_or_else(|| home.join("mcp.json"));
    let mut state_prefix = String::new();
    if let Some(dir) = project.as_mut() {
        if !dir.is_dir() {
            die(&format!("not a directory: {}", dir.display()));
        }
        *dir = std::path::absolute(&*dir).unwrap_or_else(|e| die(&e.to_string()));
        source = dir.join(".mcpsync.json");
        // keep the key in the bash version's form so existing project state still matches
        let key: String =
            config::msys_path(dir).chars().map(|c| if matches!(c, '/' | '\\' | ':' | ' ') { '_' } else { c }).collect();
        state_prefix = format!("project:{key}:");
        if s.fallback == "resolve" {
            warn("FALLBACK=resolve is ignored for project files (they're often committed); using skip");
            s.fallback = "skip".into();
        }
    }
    if !matches!(s.fallback.as_str(), "skip" | "resolve" | "keep") {
        die(&format!("FALLBACK must be skip, resolve or keep (got '{}')", s.fallback));
    }
    let ctx = Ctx {
        project,
        source,
        config_file,
        state_file: home.join("state.json"),
        backup_dir: home.join("backups"),
        state_prefix,
        s,
    };

    match cmd.as_str() {
        "init" => cmd_init(&ctx, &rest),
        "edit" => cmd_edit(&ctx),
        "list" | "ls" => cmd_list(&ctx),
        "sync" => cmd_sync(&ctx, &rest, false),
        "diff" => cmd_sync(&ctx, &rest, true),
        "status" => cmd_status(&ctx),
        "import" => cmd_import(&ctx, &rest),
        "editors" => cmd_editors(&ctx),
        "check" => cmd_check(&ctx),
        "secret" | "secrets" => cmd_secret(&ctx, &rest),
        "doctor" => cmd_doctor(&ctx),
        _ => die(&format!("unknown command: {cmd} (see: mcpsync help)")),
    }
}

// ---------------------------------------------------------------- editors & files

fn is_supported(ed: &str) -> bool {
    SUPPORTED_EDITORS.contains(&ed)
}

fn is_enabled(ctx: &Ctx, ed: &str) -> bool {
    ctx.s.editors.iter().any(|e| e == ed)
}

fn servers_key(ed: &str) -> &'static str {
    if ed == "vscode" {
        "servers"
    } else {
        "mcpServers"
    }
}

fn editor_file(ctx: &Ctx, ed: &str) -> Option<PathBuf> {
    match &ctx.project {
        Some(dir) => config::project_editor_file(ed, dir),
        None => Some(config::user_editor_file(ed, &ctx.s)),
    }
}

fn editor_detected(ctx: &Ctx, ed: &str) -> bool {
    let f = editor_file(ctx, ed);
    if f.as_ref().is_some_and(|f| f.exists()) {
        return true;
    }
    let parent_is_dir = || f.as_ref().and_then(|f| f.parent()).is_some_and(Path::is_dir);
    let home = config::home_dir();
    match ed {
        "claude-code" => find_on_path("claude").is_some(),
        "cursor" => home.join(".cursor").is_dir() || find_on_path("cursor").is_some(),
        "codex" => config::codex_home().is_dir() || find_on_path("codex").is_some(),
        "claude-desktop" => parent_is_dir(),
        "vscode" => parent_is_dir() || find_on_path("code").is_some(),
        "gemini" => home.join(".gemini").is_dir() || find_on_path("gemini").is_some(),
        "devin" => parent_is_dir() || find_on_path("devin").is_some(),
        _ => false,
    }
}

/// Atomic write that follows symlinks.
fn write_file(target: &Path, text: &str) {
    let real = if target.is_symlink() {
        std::fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf())
    } else {
        target.to_path_buf()
    };
    let dir = real.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).unwrap_or_else(|e| die(&format!("can't create {}: {e}", dir.display())));
    let tmp = dir.join(format!(".mcpsync.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).unwrap_or_else(|e| die(&format!("can't write {}: {e}", tmp.display())));
    if let Err(e) = std::fs::rename(&tmp, &real) {
        let _ = std::fs::remove_file(&tmp);
        die(&format!("can't write {}: {e}", real.display()));
    }
}

fn backup_label(ctx: &Ctx, ed: &str) -> String {
    match &ctx.project {
        Some(dir) => {
            let b = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let b: String = b
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
                .collect();
            format!("{b}.{ed}")
        }
        None => ed.to_string(),
    }
}

/// Copies `f` into the backup folder and keeps the newest BACKUPS copies per label.
fn backup_file(ctx: &Ctx, f: &Path, label: &str) {
    if !f.is_file() {
        return;
    }
    let ext =
        f.file_name().and_then(|n| n.to_str()).and_then(|n| n.rsplit_once('.')).map_or("bak", |(_, e)| e).to_string();
    std::fs::create_dir_all(&ctx.backup_dir)
        .unwrap_or_else(|e| die(&format!("can't create {}: {e}", ctx.backup_dir.display())));
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let dest = ctx.backup_dir.join(format!("{label}.{stamp}.{ext}"));
    std::fs::copy(f, &dest).unwrap_or_else(|e| die(&format!("can't back up {}: {e}", f.display())));
    let (pre, suf) = (format!("{label}."), format!(".{ext}"));
    let mut old: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&ctx.backup_dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_name()
                .to_str()
                .is_some_and(|n| n.starts_with(&pre) && n.ends_with(&suf) && n.len() > pre.len() + suf.len())
        })
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    old.sort_by_key(|o| std::cmp::Reverse(o.0));
    for (_, p) in old.into_iter().skip(ctx.s.backups) {
        let _ = std::fs::remove_file(p);
    }
}

// ---------------------------------------------------------------- source & state

fn require_source(ctx: &Ctx) {
    if !ctx.source.is_file() {
        let p = if ctx.project.is_some() { " --project" } else { "" };
        die(&format!("no source config at {}. Run: mcpsync{p} init", ctx.source.display()));
    }
}

/// Reads and validates the source; dies with the problems found.
fn load_source(ctx: &Ctx) -> Value {
    let (src, _) = engine::parse_json_file(&ctx.source).unwrap_or_else(|e| die(&e));
    let probs = engine::validate(&src, &ctx.source).unwrap_or_else(|e| die(&e));
    if !probs.is_empty() {
        die(&format!("problems in {}:\n{}", ctx.source.display(), probs.join("\n")));
    }
    src
}

fn read_state(ctx: &Ctx) -> serde_json::Map<String, Value> {
    engine::read_text(&ctx.state_file)
        .ok()
        .flatten()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn prev_servers(ctx: &Ctx, ed: &str) -> Vec<String> {
    read_state(ctx)
        .get(&format!("{}{ed}", ctx.state_prefix))
        .and_then(|v| v.get("servers"))
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn state_set(ctx: &Ctx, ed: &str, file: &Path, gen: &Gen) {
    let mut st = read_state(ctx);
    let mut names: Vec<&String> = gen.servers.keys().collect();
    names.sort();
    st.insert(
        format!("{}{ed}", ctx.state_prefix),
        serde_json::json!({
            "file": file.display().to_string(),
            "servers": names,
            "synced_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        }),
    );
    if let Some(dir) = ctx.state_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    write_file(&ctx.state_file, &engine::pretty(&Value::Object(st)));
}

// ---------------------------------------------------------------- planning

struct PlanOne {
    file: PathBuf,
    gen: Gen,
    plan: Plan,
}

fn plan_one(ctx: &Ctx, src: &Value, ed: &str, file: PathBuf) -> PlanOne {
    let wrap = ctx.s.npx_wrap.iter().any(|w| w == ed);
    let gen = engine::generate(src, ed, &ctx.s.fallback, wrap);
    let prev = prev_servers(ctx, ed);
    let plan = if ed == "codex" {
        let text = engine::read_text(&file).unwrap_or_else(|e| die(&e)).unwrap_or_default();
        engine::plan_toml(&text, &gen, &prev)
    } else {
        engine::plan_json(&file, servers_key(ed), &gen, &prev).unwrap_or_else(|e| die(&e))
    };
    PlanOne { file, gen, plan }
}

fn sync_one(ctx: &Ctx, src: &Value, ed: &str, dry: bool) {
    if !is_supported(ed) {
        die(&format!("unknown editor: {ed} (supported: {})", SUPPORTED_EDITORS.join(" ")));
    }
    let Some(file) = editor_file(ctx, ed) else {
        warn(&format!("{ed} has no project-level MCP file; skipped"));
        return;
    };
    let p = plan_one(ctx, src, ed, file);
    for w in &p.gen.warnings {
        warn(&format!("{ed} › {w}"));
    }
    for n in &p.plan.takeover {
        warn(&format!("{ed} › '{n}' already exists there and will be replaced by the one in your source"));
    }
    if p.plan.jsonc {
        warn(&format!(
            "{ed} › {} has comments or trailing commas; they'll be removed on write (a backup is kept)",
            p.file.display()
        ));
    }
    if p.plan.before == p.plan.after {
        ok(&format!("{ed}: up to date  {}", dim(&p.file.display().to_string())));
    } else if dry {
        info(&format!("{} → {} {}", bold(ed), p.file.display(), dim("(dry run)")));
        let diff = similar::TextDiff::from_lines(&p.plan.before, &p.plan.after);
        print!("{}", diff.unified_diff().header("current", "after sync"));
    } else {
        backup_file(ctx, &p.file, &backup_label(ctx, ed));
        write_file(&p.file, &p.plan.new_text);
        ok(&format!("{ed}: {} server(s) → {}", p.gen.servers.len(), p.file.display()));
    }
    if !dry {
        state_set(ctx, ed, &p.file, &p.gen);
    }
}

// ---------------------------------------------------------------- commands

fn cmd_sync(ctx: &Ctx, rest: &[String], mut dry: bool) {
    let mut eds: Vec<String> = Vec::new();
    for a in rest {
        match a.as_str() {
            "-n" | "--dry-run" => dry = true,
            _ if a.starts_with('-') => die(&format!("unknown option: {a}")),
            _ => eds.push(a.clone()),
        }
    }
    require_source(ctx);
    let src = load_source(ctx);
    if eds.is_empty() {
        if ctx.s.editors.is_empty() {
            die("no editors configured. Run: mcpsync init   (or pass editor names)");
        }
        eds = ctx.s.editors.clone();
    }
    for ed in &eds {
        sync_one(ctx, &src, ed, dry);
    }
    if !dry {
        info(&dim("Fully quit and reopen the editors to load the changes."));
    }
}

fn cmd_status(ctx: &Ctx) {
    require_source(ctx);
    let src = load_source(ctx);
    println!("{:<15} {:<8} {:<12} FILE", "EDITOR", "ENABLED", "STATE");
    for ed in SUPPORTED_EDITORS {
        let en = is_enabled(ctx, ed);
        if !en && !editor_detected(ctx, ed) {
            continue;
        }
        let Some(file) = editor_file(ctx, ed) else { continue };
        let p = plan_one(ctx, &src, ed, file);
        let st = if !p.file.exists() {
            "not created"
        } else if p.plan.before != p.plan.after {
            "out of sync"
        } else {
            "in sync"
        };
        println!("{:<15} {:<8} {:<12} {}", ed, if en { "yes" } else { "no" }, st, p.file.display());
    }
}

fn cmd_list(ctx: &Ctx) {
    require_source(ctx);
    let src = load_source(ctx);
    let mut rows = vec![["NAME".to_string(), "TYPE".into(), "EDITORS".into(), "VARIABLES".into()]];
    rows.extend(engine::list_rows(&src));
    let w: Vec<usize> = (0..4).map(|i| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0)).collect();
    for r in &rows {
        println!("{:<a$}  {:<b$}  {:<c$}  {}", r[0], r[1], r[2], r[3], a = w[0], b = w[1], c = w[2]);
    }
}

fn cmd_editors(ctx: &Ctx) {
    for ed in SUPPORTED_EDITORS {
        let d = if editor_detected(ctx, ed) { "yes" } else { "no" };
        let en = if is_enabled(ctx, ed) { "yes" } else { "no" };
        let f = editor_file(ctx, ed).map_or("(no project-level file)".to_string(), |f| f.display().to_string());
        println!("{ed:<15} detected={d:<4} enabled={en:<4} {f}");
    }
}

fn write_config(ctx: &Ctx, editors: &str) {
    let text = format!(
        r#"# mcpsync settings (KEY="value" lines; the bash version sourced this file)

# Editors to sync. Supported: {supported}
EDITORS="{editors}"

# When an editor can't expand a ${{VAR}} reference
# (Claude Desktop: anywhere; Codex: anything except NAME="${{NAME}}" env entries,
#  Authorization="Bearer ${{X}}" and Header="${{X}}"):
#   skip    - leave that server out for that editor (default; never writes secrets to disk)
#   resolve - write the current value in plaintext (only for user-level files)
#   keep    - write ${{VAR}} as-is (the editor will see the literal text)
FALLBACK="skip"

# Timestamped backups kept per editor in {backups}
BACKUPS=10

# Editors where "npx ..." becomes "cmd /c npx ..." (e.g. "claude-code codex")
NPX_WRAP_EDITORS=""

# Override where an editor's user-level config lives (Windows or Git Bash paths both work):
# PATH_CURSOR="$HOME/.cursor/mcp.json"
# PATH_CODEX="$HOME/.codex/config.toml"
# PATH_VSCODE="$APPDATA/Code - Insiders/User/mcp.json"
"#,
        supported = SUPPORTED_EDITORS.join(" "),
        backups = ctx.backup_dir.display(),
    );
    write_file(&ctx.config_file, &text);
}

fn take_value(rest: &[String], i: &mut usize, flag: &str) -> String {
    *i += 1;
    rest.get(*i).cloned().unwrap_or_else(|| die(&format!("{flag} needs a value")))
}

fn cmd_init(ctx: &Ctx, rest: &[String]) {
    let (mut chosen, mut force, mut from): (Option<String>, bool, Option<String>) = (None, false, None);
    let mut i = 0;
    while i < rest.len() {
        let a = rest[i].as_str();
        match a {
            "--editors" => chosen = Some(take_value(rest, &mut i, a)),
            "--from" => from = Some(take_value(rest, &mut i, a)),
            "--force" => force = true,
            _ if a.starts_with("--editors=") => chosen = Some(a["--editors=".len()..].to_string()),
            _ if a.starts_with("--from=") => from = Some(a["--from=".len()..].to_string()),
            _ => die(&format!("unknown option: {a}")),
        }
        i += 1;
    }
    if let Some(dir) = ctx.config_file.parent() {
        std::fs::create_dir_all(dir).unwrap_or_else(|e| die(&format!("can't create {}: {e}", dir.display())));
    }

    if ctx.project.is_none() {
        if ctx.config_file.is_file() && !force {
            ok(&format!("settings already exist: {} (use --force to redo)", ctx.config_file.display()));
        } else {
            let chosen = chosen.unwrap_or_else(|| {
                let mut picked = Vec::new();
                let tty = std::io::stdin().is_terminal();
                if tty {
                    info("Which editors do you use? (detected ones default to yes)");
                }
                for ed in SUPPORTED_EDITORS {
                    let d = editor_detected(ctx, ed);
                    let yes = if tty {
                        eprint!("  {ed} [{}] ", if d { "Y/n" } else { "y/N" });
                        let mut ans = String::new();
                        let _ = std::io::stdin().lock().read_line(&mut ans);
                        let ans = ans.trim();
                        if ans.is_empty() {
                            d
                        } else {
                            ans.starts_with(['y', 'Y'])
                        }
                    } else {
                        d
                    };
                    if yes {
                        picked.push(ed);
                    }
                }
                picked.join(" ")
            });
            for ed in chosen.split_whitespace() {
                if !is_supported(ed) {
                    die(&format!("unknown editor: {ed}"));
                }
            }
            let chosen = chosen.split_whitespace().collect::<Vec<_>>().join(" ");
            write_config(ctx, &chosen);
            ok(&format!(
                "wrote {} (editors: {})",
                ctx.config_file.display(),
                if chosen.is_empty() { "none" } else { &chosen }
            ));
        }
    }

    if ctx.source.is_file() {
        ok(&format!("source already exists: {}", ctx.source.display()));
    } else {
        write_file(&ctx.source, "{\n  \"mcpServers\": {}\n}\n");
        ok(&format!("created {}", ctx.source.display()));
    }
    if let Some(ed) = from {
        cmd_import(ctx, &[ed]);
    }

    let p = if ctx.project.is_some() { " --project" } else { "" };
    info("");
    info("Next:");
    info(&format!("  mcpsync{p} edit               define servers; write secrets as ${{VAR}}"));
    info("  envsync set VAR              store each secret once");
    info(&format!("  mcpsync{p} check              see which secrets are still missing"));
    info(&format!("  mcpsync{p} sync --dry-run     preview"));
    info(&format!("  mcpsync{p} sync"));
}

fn cmd_import(ctx: &Ctx, rest: &[String]) {
    let mut ed: Option<&str> = None;
    let mut force = false;
    for a in rest {
        match a.as_str() {
            "--force" => force = true,
            _ if a.starts_with('-') => die(&format!("unknown option: {a}")),
            _ => ed = Some(a),
        }
    }
    let ed = ed.unwrap_or_else(|| die("usage: mcpsync import <editor> [--force]"));
    if !is_supported(ed) {
        die(&format!("unknown editor: {ed}"));
    }
    let f = editor_file(ctx, ed);
    let f = match f {
        Some(f) if std::fs::metadata(&f).is_ok_and(|m| m.len() > 0) => f,
        Some(f) => die(&format!("nothing to import: {} not found", f.display())),
        None => die(&format!("nothing to import: {ed} has no file in this scope not found")),
    };
    let (new_src, notes) =
        engine::import(ed, ed == "codex", servers_key(ed), &f, &ctx.source, force).unwrap_or_else(|e| die(&e));
    write_file(&ctx.source, &engine::pretty(&new_src));
    for n in notes {
        match n {
            ImportNote::Added(v) => ok(&format!("imported {v}")),
            ImportNote::Replaced(v) => warn(&format!("replaced {v}")),
            ImportNote::Kept(v) => info(&format!("  {v} is already in the source (use --force to overwrite)")),
            ImportNote::Secret(v) => warn(&format!(
                "{v} looks like a hardcoded secret → replace it with ${{VAR}} in the source, then: envsync set VAR"
            )),
        }
    }
}

fn cmd_edit(ctx: &Ctx) {
    require_source(ctx);
    let ed = std::env::var("VISUAL")
        .ok()
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var("EDITOR").ok().filter(|v| !v.is_empty()))
        .unwrap_or_else(|| "notepad".into());
    let mut words = ed.split_whitespace();
    let prog = words.next().unwrap_or("notepad");
    let prog = find_on_path(prog).unwrap_or_else(|| PathBuf::from(prog));
    let status = Command::new(&prog).args(words).arg(&ctx.source).status();
    if let Err(e) = status {
        die(&format!("can't start {}: {e}", prog.display()));
    }
    load_source(ctx);
    let p = if ctx.project.is_some() { " --project" } else { "" };
    ok(&format!("{} is valid. Preview with: mcpsync{p} sync --dry-run", ctx.source.display()));
}

// ---------------------------------------------------------------- secrets

fn source_var_names(ctx: &Ctx) -> Vec<String> {
    let src = load_source(ctx);
    engine::ref_names(&Value::Object(engine::servers(&src)))
}

fn report_vars(ctx: &Ctx) {
    let names = source_var_names(ctx);
    if names.is_empty() {
        info("  (the source references no ${VAR}s)");
        return;
    }
    let store = Store::open();
    let user = store.names().unwrap_or_default();
    let wsl = store.wslenv().unwrap_or_default();
    for n in &names {
        let mut where_: Vec<&str> = Vec::new();
        let shell = std::env::var(n).is_ok_and(|v| !v.is_empty());
        let in_user = user.iter().any(|u| u.eq_ignore_ascii_case(n));
        let in_wsl = wsl.iter().any(|u| u.eq_ignore_ascii_case(n));
        if shell {
            where_.push("this shell");
        }
        if in_user {
            where_.push("Windows user env");
        }
        if in_wsl {
            where_.push("WSLENV");
        }
        if where_.is_empty() {
            bad(&format!("{n} — not set   → envsync set {n}"));
            continue;
        }
        ok(&format!("{n} — {}", where_.join(", ")));
        if in_user && !shell {
            info(&format!("    {}", dim("set, but this shell started earlier; reopen terminals and editors")));
        }
        if in_user && !in_wsl {
            info(&format!("    {}", dim("not shared with WSL → mcpsync secret wslenv")));
        }
    }
}

/// Every ${NAME} in the source that is missing from the Windows user environment.
fn cmd_check(ctx: &Ctx) {
    require_source(ctx);
    let names = source_var_names(ctx);
    if names.is_empty() {
        ok("the source references no ${VAR}s");
        return;
    }
    let store = Store::open();
    let user = store.names().unwrap_or_else(|e| die(&e));
    let wsl = store.wslenv().unwrap_or_default();
    let mut missing = 0;
    for n in &names {
        if user.iter().any(|u| u.eq_ignore_ascii_case(n)) {
            let note = if wsl.iter().any(|u| u.eq_ignore_ascii_case(n)) {
                ""
            } else {
                "  (not shared with WSL → mcpsync secret wslenv)"
            };
            ok(&format!("{n}{}", dim(note)));
        } else {
            missing += 1;
            bad(&format!("{n} — not set   → envsync set {n}"));
        }
    }
    if missing > 0 {
        info(&format!("{missing} of {} variable(s) missing from your Windows user environment", names.len()));
        std::process::exit(1);
    }
    ok(&format!("all {} variable(s) are set", names.len()));
}

fn moved(sub: &str) {
    info(&dim(&format!("note: secrets are managed by envsync now; next time run: envsync {sub}")));
}

fn cmd_secret(ctx: &Ctx, rest: &[String]) {
    let sub = rest.first().map(String::as_str).unwrap_or("list");
    let name = |u: &str| -> String {
        match rest.get(1) {
            Some(n) if valid_name(n) && rest.len() == 2 => n.clone(),
            _ => die(&format!("usage: mcpsync secret {u} NAME")),
        }
    };
    match sub {
        "set" => {
            let n = name("set");
            moved(&format!("set {n}"));
            set_secret(&Store::open(), &n);
        }
        "unset" | "rm" => {
            let n = name("unset");
            moved(&format!("unset {n}"));
            unset_secret(&Store::open(), &n);
        }
        "list" | "ls" => {
            require_source(ctx);
            report_vars(ctx);
        }
        "wslenv" => {
            require_source(ctx);
            let names = source_var_names(ctx);
            if names.is_empty() {
                info("the source references no ${VAR}s");
                return;
            }
            let refs: Vec<&str> = names.iter().map(String::as_str).collect();
            Store::open().wslenv_add(&refs).unwrap_or_else(|e| die(&e));
            ok(&format!("WSLENV now shares: {}", names.join(" ")));
            info(&dim("Run 'wsl --shutdown' and reopen WSL to pick it up."));
        }
        _ => die("usage: mcpsync secret set|unset|list|wslenv [NAME]"),
    }
}

fn cmd_doctor(ctx: &Ctx) {
    let node = find_on_path("node")
        .and_then(|n| Command::new(n).arg("--version").output().ok())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty());
    let scope = if ctx.project.is_some() { "project" } else { "user" };
    info(&format!(
        "mcpsync {VERSION} · platform: windows · node {} · scope: {scope}",
        node.as_deref().unwrap_or("not found")
    ));
    if node.is_none() {
        warn("Node.js not found; servers that start with npx need it (winget install OpenJS.NodeJS.LTS)");
    }
    if find_on_path("envsync").is_none() {
        warn("envsync is not on PATH; mcpsync still works, but install it to manage secrets on their own");
    }
    if ctx.project.is_none() {
        if ctx.config_file.is_file() {
            let eds = ctx.s.editors.join(" ");
            ok(&format!(
                "settings: {} (editors: {})",
                ctx.config_file.display(),
                if eds.is_empty() { "none" } else { &eds }
            ));
        } else {
            warn("no settings yet → mcpsync init");
        }
    }
    if !ctx.source.is_file() {
        warn(&format!("no source at {} → mcpsync init", ctx.source.display()));
        return;
    }
    let src = load_source(ctx);
    ok(&format!("source: {} ({} servers)", ctx.source.display(), engine::servers(&src).len()));
    for line in engine::risky(&src) {
        warn(&format!("{line} — Claude Code sends credential variables like this as empty to remote servers; use a name of your own"));
    }
    info("variables:");
    report_vars(ctx);
    if ctx.project.is_none() && !ctx.s.editors.is_empty() {
        info("editors:");
        for ed in &ctx.s.editors {
            if !is_supported(ed) {
                bad(&format!("{ed} — unknown editor in {}", ctx.config_file.display()));
                continue;
            }
            let f = config::user_editor_file(ed, &ctx.s);
            if editor_detected(ctx, ed) {
                ok(&format!("{ed} — {}", f.display()));
            } else {
                warn(&format!("{ed} — not detected ({})", f.display()));
            }
        }
    }
}

fn cmd_uninstall(rest: &[String]) {
    if let Some(a) = rest.first() {
        die(&format!("unknown option: {a}"));
    }
    print_uninstall("mcpsync").unwrap_or_else(|e| die(&e));
    println!("envsync stays installed; remove it afterwards with: envsync uninstall");
}

fn usage() {
    let home = config::mcpsync_home();
    println!(
        r#"mcpsync {VERSION} — one MCP config, synced to every editor you use.

Usage: mcpsync [--project[=DIR]] <command> [args]

Commands
  init [--editors "a b"] [--from EDITOR] [--force]
                         create settings + source; asks which editors you use
  edit                   open the source in $VISUAL/$EDITOR (notepad if unset) and validate it
  list                   servers in the source, their type, editors and variables
  sync [--dry-run] [EDITOR...]
                         write the source into each editor's config (default: EDITORS)
  diff [EDITOR...]       same as sync --dry-run
  status                 per editor: in sync / out of sync / not created
  import EDITOR [--force]
                         copy servers from an editor's config into the source
  editors                supported editors and where their configs live
  check                  which ${{VAR}}s the source uses that aren't in your Windows user
                         environment (exit code 1 if any are missing)
  secret list            which ${{VAR}}s the source uses and where each one is set
  secret wslenv          share every ${{VAR}} the source uses with WSL
  secret set|unset NAME  same as envsync set|unset NAME
  doctor                 check node, settings, source, variables and editors
  uninstall              print the command that removes mcpsync
  agent-instructions     how AI agents should use mcpsync (Markdown)

Scope
  default                user-level configs; source: {source}
  --project[=DIR]        project files (.mcp.json, .cursor/mcp.json, .codex/config.toml,
                         .vscode/mcp.json, .gemini/settings.json, .devin/mcp_config.json);
                         source: DIR/.mcpsync.json

Editors: {editors}

Secrets are stored with envsync (envsync set NAME) as Windows user environment
variables and shared with WSL. Write them as ${{NAME}} in the source.

Source format (Claude Code style; mcpsync translates per editor):
  {{
    "mcpServers": {{
      "dokploy": {{ "command": "npx", "args": ["-y", "@ahdev/dokploy-mcp"],
                   "env": {{ "DOKPLOY_API_KEY": "${{DOKPLOY_API_KEY}}" }} }},
      "context7": {{ "type": "http", "url": "https://mcp.context7.com/mcp",
                    "headers": {{ "CONTEXT7_API_KEY": "${{CONTEXT7_API_KEY}}" }},
                    "x-editors": ["cursor", "claude-code"] }}
    }}
  }}"#,
        source = home.join("mcp.json").display(),
        editors = SUPPORTED_EDITORS.join(" "),
    );
}
