//! Source servers -> each editor's format (sync), and back (import). No file I/O except the
//! JSON/TOML readers at the bottom.

use regex::{Captures, Regex};
use serde_json::{Map, Value};
use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::path::Path;
use std::sync::LazyLock;

use crate::toml;

pub type Obj = Map<String, Value>;

fn re(s: &str) -> Regex {
    Regex::new(s).unwrap()
}

static REF: LazyLock<Regex> = LazyLock::new(|| re(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}"));
static IDENT: LazyLock<Regex> = LazyLock::new(|| re(r"^\$\{([A-Za-z_][A-Za-z0-9_]*)\}$"));
static BEARER: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^Bearer \$\{([A-Za-z_][A-Za-z0-9_]*)\}$"));
static DEFAULT_REF: LazyLock<Regex> = LazyLock::new(|| re(r"\$\{[A-Za-z_][A-Za-z0-9_]*:-"));
static ENV_REF: LazyLock<Regex> = LazyLock::new(|| re(r"\$\{env:([A-Za-z_][A-Za-z0-9_]*)\}"));
static DOLLAR_REF: LazyLock<Regex> = LazyLock::new(|| re(r"\$([A-Za-z_][A-Za-z0-9_]*)"));
static RISKY: LazyLock<Regex> =
    LazyLock::new(|| re(r"^(ANTHROPIC_|CLAUDE_|AWS_|GOOGLE_APPLICATION)|_PROXY$|^NPM_TOKEN$"));
static SECRETISH: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(key|token|secret|passw)|^authorization$"));
static URL_SECRET: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)[?&][^=&]*(key|token|secret)[^=&]*=[^&]"));
static ARG_SECRET: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(key|token|secret)[^=]*=."));

pub const SUPPORTED_EDITORS: [&str; 7] =
    ["claude-code", "cursor", "codex", "claude-desktop", "vscode", "gemini", "devin"];

// ---------- helpers

#[derive(Clone, PartialEq)]
pub enum Seg {
    Key(String),
    Idx(usize),
}

impl fmt::Display for Seg {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Seg::Key(k) => f.write_str(k),
            Seg::Idx(i) => write!(f, "{i}"),
        }
    }
}

type SegPath = Vec<Seg>;

pub fn has_ref(s: &str) -> bool {
    REF.is_match(s)
}

/// JavaScript truthiness, for the `x || y` checks the source format was defined with.
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(_) => true,
    }
}

/// JavaScript `String(v)`.
fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "null".into(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => {
            a.iter().map(|x| if x.is_null() { String::new() } else { js_string(x) }).collect::<Vec<_>>().join(",")
        }
        Value::Object(_) => "[object Object]".into(),
    }
}

fn map_str(v: &Value, f: &dyn Fn(&str) -> String) -> Value {
    match v {
        Value::String(s) => Value::String(f(s)),
        Value::Array(a) => Value::Array(a.iter().map(|x| map_str(x, f)).collect()),
        Value::Object(o) => Value::Object(map_obj(o, f)),
        _ => v.clone(),
    }
}

fn map_obj(o: &Obj, f: &dyn Fn(&str) -> String) -> Obj {
    o.iter().map(|(k, x)| (k.clone(), map_str(x, f))).collect()
}

fn strings(v: &Value) -> Vec<(SegPath, String)> {
    fn walk(v: &Value, p: &mut SegPath, acc: &mut Vec<(SegPath, String)>) {
        match v {
            Value::String(s) => acc.push((p.clone(), s.clone())),
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    p.push(Seg::Idx(i));
                    walk(x, p, acc);
                    p.pop();
                }
            }
            Value::Object(o) => {
                for (k, x) in o {
                    p.push(Seg::Key(k.clone()));
                    walk(x, p, acc);
                    p.pop();
                }
            }
            _ => {}
        }
    }
    let mut acc = Vec::new();
    walk(v, &mut Vec::new(), &mut acc);
    acc
}

fn any_default(v: &Value) -> bool {
    strings(v).iter().any(|(_, s)| DEFAULT_REF.is_match(s))
}

/// Every ${NAME} used anywhere in `v`, sorted.
pub fn ref_names(v: &Value) -> Vec<String> {
    let mut set = BTreeSet::new();
    for (_, s) in strings(v) {
        for c in REF.captures_iter(&s) {
            set.insert(c[1].to_string());
        }
    }
    set.into_iter().collect()
}

fn resolve(s: &str) -> String {
    REF.replace_all(s, |c: &Captures| match std::env::var(&c[1]) {
        Ok(v) if !v.is_empty() => v,
        _ => c.get(2).map_or(String::new(), |m| m.as_str().to_string()),
    })
    .into_owned()
}

fn env_colon(s: &str) -> String {
    REF.replace_all(s, |c: &Captures| format!("${{env:{}}}", &c[1])).into_owned()
}

fn un_env(s: &str) -> String {
    ENV_REF.replace_all(s, |c: &Captures| format!("${{{}}}", &c[1])).into_owned()
}

fn get_path<'a>(v: &'a Value, p: &[Seg]) -> Option<&'a Value> {
    p.iter().try_fold(v, |a, s| match s {
        Seg::Key(k) => a.get(k.as_str()),
        Seg::Idx(i) => a.get(*i),
    })
}

fn get_path_mut<'a>(v: &'a mut Value, p: &[Seg]) -> Option<&'a mut Value> {
    p.iter().try_fold(v, |a, s| match s {
        Seg::Key(k) => a.get_mut(k.as_str()),
        Seg::Idx(i) => a.get_mut(*i),
    })
}

pub fn sort_keys(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.iter().map(sort_keys).collect()),
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            Value::Object(keys.into_iter().map(|k| (k.clone(), sort_keys(&o[k]))).collect())
        }
        _ => v.clone(),
    }
}

fn without(o: &Obj, keys: &[&str]) -> Obj {
    let mut c = o.clone();
    for k in keys {
        c.shift_remove(*k);
    }
    c
}

pub fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap() + "\n"
}

fn join_path(p: &[Seg]) -> String {
    p.iter().map(ToString::to_string).collect::<Vec<_>>().join(".")
}

// ---------- source -> editor

pub struct Tr {
    pub server: Option<Obj>,
    pub warn: Vec<String>,
}

fn tr(server: Obj) -> Tr {
    Tr { server: Some(server), warn: vec![] }
}

fn for_editor(s: &Obj, ed: &str) -> bool {
    let has = |k: &str| s.get(k).and_then(Value::as_array).map(|a| a.iter().any(|x| x == ed));
    has("x-editors") != Some(false) && has("x-exclude-editors") != Some(true)
}

fn norm(s: &Obj) -> Obj {
    let mut o: Obj = s.iter().filter(|(k, _)| !k.starts_with("x-")).map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut t = if truthy(s.get("command")) {
        "stdio".to_string()
    } else if truthy(s.get("type")) {
        js_string(&s["type"])
    } else {
        "http".to_string()
    };
    if t == "streamable-http" || t == "streamableHttp" {
        t = "http".into();
    }
    o.insert("type".into(), Value::String(t));
    o
}

fn type_of(o: &Obj) -> &str {
    o.get("type").and_then(Value::as_str).unwrap_or("")
}

/// Refs an editor can't expand go through the FALLBACK policy.
fn apply_fb(server: Obj, pol: &str, paths: Vec<SegPath>) -> Tr {
    if paths.is_empty() {
        return tr(server);
    }
    let where_ = paths.iter().map(|p| join_path(p)).collect::<Vec<_>>().join(", ");
    match pol {
        "resolve" => {
            let mut s = Value::Object(server);
            for p in &paths {
                if let Some(Value::String(cur)) = get_path(&s, p) {
                    let r = resolve(cur);
                    if let Some(slot) = get_path_mut(&mut s, p) {
                        *slot = Value::String(r);
                    }
                }
            }
            Tr {
                server: s.as_object().cloned(),
                warn: vec![format!("wrote resolved values IN PLAINTEXT for {where_} (FALLBACK=resolve)")],
            }
        }
        "keep" => Tr {
            server: Some(server),
            warn: vec![format!("${{VAR}} left unexpanded in {where_}; this editor won't expand it there")],
        },
        _ => Tr {
            server: None,
            warn: vec![format!(
                "skipped: this editor can't expand ${{VAR}} in {where_} (set FALLBACK=resolve or keep to include it)"
            )],
        },
    }
}

fn ref_paths(server: &Obj, roots: &[&str]) -> Vec<SegPath> {
    strings(&Value::Object(server.clone()))
        .into_iter()
        .filter(|(p, s)| has_ref(s) && matches!(p.first(), Some(Seg::Key(k)) if roots.contains(&k.as_str())))
        .map(|(p, _)| p)
        .collect()
}

/// Cursor, VS Code, Devin: ${env:NAME} in command/args/env/url/headers.
fn tr_env_colon(s: &Obj, ed: &str) -> Tr {
    let mut o = map_obj(s, &env_colon);
    if ed == "cursor" {
        o.shift_remove("type");
    } else if ed == "devin" {
        let t = o.shift_remove("type").unwrap_or(Value::Null);
        if t != "stdio" {
            o.insert("transport".into(), t);
        }
    }
    let warn = if any_default(&Value::Object(s.clone())) {
        vec![format!("${{VAR:-default}} fallbacks aren't supported by {ed}; the defaults were dropped")]
    } else {
        vec![]
    };
    Tr { server: Some(o), warn }
}

/// Gemini CLI expands $VAR, ${VAR} and ${VAR:-default} in every string of settings.json.
fn tr_gemini(s: &Obj) -> Tr {
    let mut o = without(s, &["type"]);
    if type_of(s) == "http" {
        o.shift_remove("url");
        o.insert("httpUrl".into(), s.get("url").cloned().unwrap_or(Value::Null));
    }
    tr(o)
}

/// Claude Desktop: no expansion, stdio only (remote servers are bridged via mcp-remote).
fn tr_desktop(s: &Obj, pol: &str) -> Tr {
    if type_of(s) == "stdio" {
        let mut r = apply_fb(s.clone(), pol, ref_paths(s, &["command", "args", "env"]));
        if let Some(x) = r.server.as_mut() {
            x.shift_remove("type");
        }
        return r;
    }
    let mut r = apply_fb(s.clone(), pol, ref_paths(s, &["url", "headers"]));
    let Some(x) = r.server.take() else { return r };
    let empty = Obj::new();
    let headers = x.get("headers").and_then(Value::as_object).unwrap_or(&empty);
    let mut args: Vec<Value> = vec!["-y".into(), "mcp-remote".into(), x.get("url").cloned().unwrap_or(Value::Null)];
    if type_of(&x) == "sse" {
        args.push("--transport".into());
        args.push("sse-only".into());
    }
    for (k, v) in headers {
        args.push("--header".into());
        args.push(format!("{k}:{}", js_string(v)).into());
    }
    r.warn.push("bridged through mcp-remote (Claude Desktop's config file only starts local servers)".into());
    if headers.values().any(|v| js_string(v).contains(' ')) {
        r.warn.push(
            "a header value contains a space; Claude Desktop on Windows can split it when starting npx (see the mcp-remote README)"
                .into(),
        );
    }
    let mut o = Obj::new();
    o.insert("command".into(), "npx".into());
    o.insert("args".into(), Value::Array(args));
    r.server = Some(o);
    r
}

/// Codex: no ${VAR} expansion; secrets come in through env_vars (forwarded from Codex's own
/// environment), bearer_token_env_var and env_http_headers. Streamable HTTP only, no SSE.
fn tr_codex(s: &Obj, pol: &str) -> Tr {
    if type_of(s) == "sse" {
        return Tr {
            server: None,
            warn: vec!["skipped: Codex supports stdio and streamable HTTP servers, not SSE".into()],
        };
    }
    let mut unsupported = Vec::new();
    for (p, st) in strings(&Value::Object(s.clone())) {
        if !has_ref(&st) {
            continue;
        }
        let root = p.first().map(ToString::to_string).unwrap_or_default();
        let key = p.get(1).map(ToString::to_string).unwrap_or_default();
        let ident = IDENT.captures(&st);
        if root == "env" && p.len() == 2 && ident.as_ref().is_some_and(|m| m[1] == key) {
            continue;
        }
        if root == "headers"
            && p.len() == 2
            && (ident.is_some() || (key.eq_ignore_ascii_case("authorization") && BEARER.is_match(&st)))
        {
            continue;
        }
        unsupported.push(p);
    }
    let mut r = apply_fb(s.clone(), pol, unsupported);
    let Some(x) = r.server.take() else { return r };
    let mut o = Obj::new();
    if type_of(&x) == "stdio" {
        if let Some(c) = x.get("command") {
            o.insert("command".into(), c.clone());
        }
        if truthy(x.get("args")) {
            o.insert("args".into(), x["args"].clone());
        }
        let mut env = Obj::new();
        let mut env_vars = Vec::new();
        for (k, v) in x.get("env").and_then(Value::as_object).into_iter().flatten() {
            match v.as_str().and_then(|s| IDENT.captures(s)) {
                Some(m) if &m[1] == k => env_vars.push(Value::String(k.clone())),
                _ => {
                    env.insert(k.clone(), v.clone());
                }
            }
        }
        if !env.is_empty() {
            o.insert("env".into(), Value::Object(env));
        }
        if !env_vars.is_empty() {
            o.insert("env_vars".into(), Value::Array(env_vars));
        }
    } else {
        if let Some(u) = x.get("url") {
            o.insert("url".into(), u.clone());
        }
        let mut http = Obj::new();
        let mut env_http = Obj::new();
        for (k, v) in x.get("headers").and_then(Value::as_object).into_iter().flatten() {
            let s = v.as_str();
            if let Some(m) = s.filter(|_| k.eq_ignore_ascii_case("authorization")).and_then(|s| BEARER.captures(s)) {
                o.insert("bearer_token_env_var".into(), m[1].to_string().into());
            } else if let Some(m) = s.and_then(|s| IDENT.captures(s)) {
                env_http.insert(k.clone(), m[1].to_string().into());
            } else {
                http.insert(k.clone(), v.clone());
            }
        }
        if !http.is_empty() {
            o.insert("http_headers".into(), Value::Object(http));
        }
        if !env_http.is_empty() {
            o.insert("env_http_headers".into(), Value::Object(env_http));
        }
    }
    // Only copy keys Codex documents; others (e.g. Cursor/Gemini-specific ones) could make Codex reject the file.
    const CODEX_KEYS: [&str; 13] = [
        "cwd",
        "startup_timeout_sec",
        "tool_timeout_sec",
        "enabled",
        "required",
        "enabled_tools",
        "disabled_tools",
        "default_tools_approval_mode",
        "tools",
        "scopes",
        "auth",
        "environment_id",
        "startup_readiness",
    ];
    for (k, v) in &x {
        if CODEX_KEYS.contains(&k.as_str()) {
            o.insert(k.clone(), v.clone());
        }
    }
    if x.get("disabled") == Some(&Value::Bool(true)) && !o.contains_key("enabled") {
        o.insert("enabled".into(), false.into());
    }
    r.server = Some(o);
    r
}

fn translate(s: &Obj, ed: &str, pol: &str) -> Tr {
    match ed {
        "cursor" | "vscode" | "devin" => tr_env_colon(s, ed),
        "gemini" => tr_gemini(s),
        "claude-desktop" => tr_desktop(s, pol),
        "codex" => tr_codex(s, pol),
        _ => tr(s.clone()),
    }
}

fn wrap_npx(mut r: Tr, wrap: bool) -> Tr {
    if let Some(s) = r.server.as_mut().filter(|s| wrap && s.get("command") == Some(&Value::from("npx"))) {
        let mut args: Vec<Value> = vec!["/c".into(), "npx".into()];
        args.extend(s.get("args").and_then(Value::as_array).cloned().unwrap_or_default());
        s.insert("command".into(), "cmd".into());
        s.insert("args".into(), Value::Array(args));
    }
    r
}

/// WSL_NAME keys only apply inside WSL; on the Windows host they are left out.
fn drop_wsl_env(s: &Obj) -> Obj {
    let is_wsl = |k: &str| k.len() > 4 && k.starts_with("WSL_");
    match s.get("env").and_then(Value::as_object) {
        Some(env) if env.keys().any(|k| is_wsl(k)) => {
            let kept: Obj = env.iter().filter(|(k, _)| !is_wsl(k)).map(|(k, v)| (k.clone(), v.clone())).collect();
            let mut o = s.clone();
            o.insert("env".into(), Value::Object(kept));
            o
        }
        _ => s.clone(),
    }
}

pub struct Gen {
    pub servers: Obj,
    pub warnings: Vec<String>,
    pub skipped: Vec<String>,
}

pub fn generate(src: &Value, ed: &str, pol: &str, wrap: bool) -> Gen {
    let mut g = Gen { servers: Obj::new(), warnings: vec![], skipped: vec![] };
    for (name, raw) in src.get("mcpServers").and_then(Value::as_object).into_iter().flatten() {
        let Some(raw) = raw.as_object() else { continue };
        if !for_editor(raw, ed) {
            continue;
        }
        let r = wrap_npx(translate(&norm(&drop_wsl_env(raw)), ed, pol), wrap);
        for w in &r.warn {
            g.warnings.push(format!("{name}: {w}"));
        }
        match r.server {
            Some(s) => {
                g.servers.insert(name.clone(), Value::Object(s));
            }
            None => g.skipped.push(name.clone()),
        }
    }
    g
}

// ---------- planning: remove what we wrote last time, keep the rest, add the current set

pub struct Plan {
    pub new_text: String,
    pub takeover: Vec<String>,
    pub jsonc: bool,
    pub before: String,
    pub after: String,
}

pub fn plan_json(file: &Path, key: &str, gen: &Gen, prev: &[String]) -> Result<Plan, String> {
    let (data, jsonc) = parse_json_file(file)?;
    let Value::Object(data) = data else { return Err(format!("{} must contain a JSON object", file.display())) };
    let cur = data.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
    let mut next = Obj::new();
    for (k, v) in &cur {
        if !prev.contains(k) && !gen.servers.contains_key(k) {
            next.insert(k.clone(), v.clone());
        }
    }
    for (k, v) in &gen.servers {
        next.insert(k.clone(), v.clone());
    }
    let mut res = data.clone();
    res.insert(key.to_string(), Value::Object(next.clone()));
    Ok(Plan {
        new_text: pretty(&Value::Object(res)),
        takeover: gen.servers.keys().filter(|n| cur.contains_key(*n) && !prev.contains(n)).cloned().collect(),
        jsonc,
        before: pretty(&sort_keys(&Value::Object(cur))),
        after: pretty(&sort_keys(&Value::Object(next))),
    })
}

static BLANK_OR_COMMENT: LazyLock<Regex> = LazyLock::new(|| re(r"^\s*(#.*)?$"));

pub fn plan_toml(text: &str, gen: &Gen, prev: &[String]) -> Plan {
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let blocks = toml::blocks(text);
    let existing: HashSet<&str> = blocks.iter().filter_map(toml::Block::server_name).collect();
    let drop: HashSet<&str> = prev.iter().map(String::as_str).chain(gen.servers.keys().map(String::as_str)).collect();
    // A dropped block keeps its trailing comment lines: they usually describe the next table.
    let tail = |b: &toml::Block| -> Vec<String> {
        let mut k = b.lines.len();
        while k > 1 && BLANK_OR_COMMENT.is_match(&b.lines[k - 1]) {
            k -= 1;
        }
        let t = &b.lines[k..];
        if t.iter().any(|l| l.trim().starts_with('#')) {
            t.to_vec()
        } else {
            vec![]
        }
    };
    let mut lines: Vec<String> = Vec::new();
    for b in &blocks {
        match b.server_name() {
            Some(n) if drop.contains(n) => lines.extend(tail(b)),
            _ => lines.extend(b.lines.iter().cloned()),
        }
    }
    toml::trim_end(&mut lines);
    for (name, o) in &gen.servers {
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.extend(toml::server_toml(name, o.as_object().unwrap()));
    }
    let new_text = if lines.is_empty() { String::new() } else { lines.join(eol) + eol };
    Plan {
        takeover: gen.servers.keys().filter(|n| existing.contains(n.as_str()) && !prev.contains(n)).cloned().collect(),
        jsonc: false,
        before: toml::section(text),
        after: toml::section(&new_text),
        new_text,
    }
}

// ---------- editor -> source (import)

fn from_editor(ed: &str, s: &Obj) -> Obj {
    let mut o = map_obj(s, &un_env);
    if ed == "gemini" {
        o = map_obj(&o, &|x: &str| DOLLAR_REF.replace_all(x, |c: &Captures| format!("${{{}}}", &c[1])).into_owned());
        if truthy(o.get("httpUrl")) {
            let u = o.shift_remove("httpUrl").unwrap();
            o.insert("url".into(), u);
            o.insert("type".into(), "http".into());
        } else if truthy(o.get("url")) {
            o.insert("type".into(), "sse".into());
        }
    } else if ed == "devin" && truthy(o.get("transport")) {
        let t = o.shift_remove("transport").unwrap();
        o.insert("type".into(), t);
    } else if ed == "codex" {
        if let Some(vars) = o.get("env_vars").and_then(Value::as_array).cloned() {
            let mut env = o.get("env").and_then(Value::as_object).cloned().unwrap_or_default();
            for v in &vars {
                let n = v.as_str().or_else(|| v.get("name").and_then(Value::as_str));
                if let Some(n) = n.filter(|n| !n.is_empty()) {
                    env.insert(n.to_string(), format!("${{{n}}}").into());
                }
            }
            o.insert("env".into(), Value::Object(env));
            o.shift_remove("env_vars");
        }
        if truthy(o.get("url")) {
            o.insert("type".into(), "http".into());
            let mut h = o.get("http_headers").and_then(Value::as_object).cloned().unwrap_or_default();
            for (k, n) in o.get("env_http_headers").and_then(Value::as_object).into_iter().flatten() {
                h.insert(k.clone(), format!("${{{}}}", js_string(n)).into());
            }
            if let Some(b) = o.get("bearer_token_env_var").filter(|v| truthy(Some(v))) {
                h.insert("Authorization".into(), format!("Bearer ${{{}}}", js_string(b)).into());
            }
            if !h.is_empty() {
                o.insert("headers".into(), Value::Object(h));
            }
            for k in ["http_headers", "env_http_headers", "bearer_token_env_var"] {
                o.shift_remove(k);
            }
        }
    }
    if truthy(o.get("command")) {
        o.shift_remove("type");
    }
    o
}

fn literal_secrets(s: &Obj) -> Vec<String> {
    let mut found = Vec::new();
    for (k, v) in s.get("env").and_then(Value::as_object).into_iter().flatten() {
        if SECRETISH.is_match(k) && v.as_str().is_some_and(|v| !v.is_empty() && !has_ref(v)) {
            found.push(format!("env.{k}"));
        }
    }
    for (k, v) in s.get("headers").and_then(Value::as_object).into_iter().flatten() {
        if SECRETISH.is_match(k) && v.as_str().is_some_and(|v| !has_ref(v)) {
            found.push(format!("headers.{k}"));
        }
    }
    if s.get("url").and_then(Value::as_str).is_some_and(|u| URL_SECRET.is_match(u) && !has_ref(u)) {
        found.push("url".into());
    }
    let args = s.get("args").and_then(Value::as_array);
    if args.into_iter().flatten().any(|x| x.as_str().is_some_and(|x| ARG_SECRET.is_match(x) && !has_ref(x))) {
        found.push("args".into());
    }
    found
}

pub enum ImportNote {
    Added(String),
    Replaced(String),
    Kept(String),
    Secret(String),
}

/// Merges the editor's servers into the source. Returns the new source document.
pub fn import(
    ed: &str,
    toml_fmt: bool,
    key: &str,
    ed_file: &Path,
    src_file: &Path,
    force: bool,
) -> Result<(Value, Vec<ImportNote>), String> {
    let txt = read_text(ed_file)?.ok_or_else(|| format!("not found: {}", ed_file.display()))?;
    let servers = if toml_fmt {
        toml::parse_servers(&txt)?
    } else {
        parse_json_file(ed_file)?.0.get(key).and_then(Value::as_object).cloned().unwrap_or_default()
    };
    let mut src = if read_text(src_file)?.is_none() {
        serde_json::json!({ "mcpServers": {} })
    } else {
        parse_json_file(src_file)?.0
    };
    if !src.is_object() {
        return Err(format!("{} must contain a JSON object", src_file.display()));
    }
    let cur = src.get("mcpServers").and_then(Value::as_object).cloned().unwrap_or_default();
    let mut merged = cur.clone();
    let mut notes = Vec::new();
    for (n, s) in &servers {
        let Some(s) = s.as_object() else { continue };
        if cur.contains_key(n) && !force {
            notes.push(ImportNote::Kept(n.clone()));
            continue;
        }
        let conv = from_editor(ed, s);
        notes.push(if cur.contains_key(n) { ImportNote::Replaced(n.clone()) } else { ImportNote::Added(n.clone()) });
        for p in literal_secrets(&conv) {
            notes.push(ImportNote::Secret(format!("{n}.{p}")));
        }
        merged.insert(n.clone(), Value::Object(conv));
    }
    src.as_object_mut().unwrap().insert("mcpServers".into(), Value::Object(merged));
    Ok((src, notes))
}

// ---------- source checks

pub fn servers(src: &Value) -> Obj {
    src.get("mcpServers").and_then(Value::as_object).cloned().unwrap_or_default()
}

pub fn validate(src: &Value, file: &Path) -> Result<Vec<String>, String> {
    let Some(o) = src.as_object() else { return Err(format!("{} must contain a JSON object", file.display())) };
    let mut probs = Vec::new();
    if o.get("mcpServers").is_some_and(|v| !v.is_object()) {
        probs.push("  \"mcpServers\" must be an object".to_string());
    }
    for (n, s) in &servers(src) {
        let Some(s) = s.as_object() else {
            probs.push(format!("  {n}: must be an object"));
            continue;
        };
        if !truthy(s.get("command")) && !truthy(s.get("url")) {
            probs.push(format!("  {n}: needs \"command\" (local server) or \"url\" (remote server)"));
        }
        for k in ["x-editors", "x-exclude-editors"] {
            let list: Vec<&Value> = match s.get(k) {
                Some(Value::Array(a)) => a.iter().collect(),
                Some(v) if truthy(Some(v)) => vec![v],
                _ => vec![],
            };
            for e in list {
                let e = js_string(e);
                if !SUPPORTED_EDITORS.contains(&e.as_str()) {
                    probs.push(format!("  {n}: unknown editor \"{e}\""));
                }
            }
        }
    }
    Ok(probs)
}

/// One row per server: name, type, editors, variables.
pub fn list_rows(src: &Value) -> Vec<[String; 4]> {
    let mut rows = Vec::new();
    for (n, s) in &servers(src) {
        let Some(s) = s.as_object() else { continue };
        let names = |k: &str| -> Option<Vec<String>> {
            s.get(k).and_then(Value::as_array).map(|a| a.iter().map(js_string).collect())
        };
        let mut eds = names("x-editors").unwrap_or_else(|| vec!["all".into()]).join(",");
        if let Some(ex) = names("x-exclude-editors").filter(|e| !e.is_empty()) {
            eds.push_str(&format!(" -{}", ex.join(",-")));
        }
        let vars = ref_names(&Value::Object(s.clone())).join(",");
        rows.push([n.clone(), type_of(&norm(s)).to_string(), eds, if vars.is_empty() { "-".into() } else { vars }]);
    }
    rows
}

/// Credential variables Claude Code sends as empty to remote servers.
pub fn risky(src: &Value) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for (n, s) in &servers(src) {
        let mut strs: Vec<&Value> = s.get("url").into_iter().collect();
        strs.extend(s.get("headers").and_then(Value::as_object).into_iter().flat_map(|h| h.values()));
        for st in strs.into_iter().filter_map(Value::as_str) {
            for c in REF.captures_iter(st) {
                let item = format!("{n}: {}", &c[1]);
                if RISKY.is_match(&c[1]) && !found.contains(&item) {
                    found.push(item);
                }
            }
        }
    }
    found
}

// ---------- files

pub fn read_text(f: &Path) -> Result<Option<String>, String> {
    match std::fs::read(f) {
        Ok(b) => {
            let t = String::from_utf8_lossy(&b).into_owned();
            Ok(Some(t.strip_prefix('\u{feff}').map(String::from).unwrap_or(t)))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("can't read {}: {e}", f.display())),
    }
}

/// Reads JSON, falling back to JSONC (comments, trailing commas). Missing or empty file = {}.
pub fn parse_json_file(f: &Path) -> Result<(Value, bool), String> {
    let t = match read_text(f)? {
        Some(t) if !t.trim().is_empty() => t,
        _ => return Ok((Value::Object(Obj::new()), false)),
    };
    if let Ok(v) = serde_json::from_str(&t) {
        return Ok((v, false));
    }
    serde_json::from_str(&strip_jsonc(&t))
        .map(|v| (v, true))
        .map_err(|e| format!("{} isn't valid JSON: {e}", f.display()))
}

/// Removes // and /* */ comments and trailing commas outside strings (JSONC -> JSON).
pub fn strip_jsonc(t: &str) -> String {
    let c: Vec<char> = t.chars().collect();
    let n = c.len();
    let at = |k: usize| c.get(k).copied();
    let mut o = String::with_capacity(t.len());
    let mut i = 0;
    let mut in_str = false;
    while i < n {
        let ch = c[i];
        let nx = at(i + 1);
        if in_str {
            o.push(ch);
            if ch == '\\' {
                if let Some(x) = nx {
                    o.push(x);
                }
                i += 2;
                continue;
            }
            if ch == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if ch == '"' {
            in_str = true;
            o.push(ch);
            i += 1;
            continue;
        }
        if ch == '/' && nx == Some('/') {
            while i < n && c[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && nx == Some('*') {
            i += 2;
            while i < n && !(c[i] == '*' && at(i + 1) == Some('/')) {
                i += 1;
            }
            i += 2;
            continue;
        }
        if ch == ',' {
            let mut j = i + 1;
            loop {
                while j < n && c[j].is_whitespace() {
                    j += 1;
                }
                if at(j) == Some('/') && at(j + 1) == Some('/') {
                    while j < n && c[j] != '\n' {
                        j += 1;
                    }
                    continue;
                }
                if at(j) == Some('/') && at(j + 1) == Some('*') {
                    j += 2;
                    while j < n && !(c[j] == '*' && at(j + 1) == Some('/')) {
                        j += 1;
                    }
                    j += 2;
                    continue;
                }
                break;
            }
            if matches!(at(j), Some('}') | Some(']')) {
                i += 1;
                continue;
            }
        }
        o.push(ch);
        i += 1;
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn gen_for(src: Value, ed: &str, pol: &str) -> Gen {
        generate(&src, ed, pol, false)
    }

    #[test]
    fn cursor_gets_env_colon_and_drops_type() {
        let g = gen_for(
            json!({"mcpServers": {"a": {"type": "http", "url": "https://x", "headers": {"K": "${K}"}}}}),
            "cursor",
            "skip",
        );
        assert_eq!(g.servers["a"], json!({"url": "https://x", "headers": {"K": "${env:K}"}}));
    }

    #[test]
    fn codex_maps_env_and_bearer() {
        let src = json!({"mcpServers": {
            "s": {"command": "npx", "args": ["-y", "p"], "env": {"TOKEN": "${TOKEN}", "URL": "https://u"}},
            "h": {"type": "http", "url": "https://h", "headers": {"Authorization": "Bearer ${PAT}", "X-K": "${K}"}},
            "bad": {"type": "http", "url": "https://h?key=${K}"},
            "sse": {"type": "sse", "url": "https://s"}
        }});
        let g = gen_for(src, "codex", "skip");
        assert_eq!(
            g.servers["s"],
            json!({"command": "npx", "args": ["-y", "p"], "env": {"URL": "https://u"}, "env_vars": ["TOKEN"]})
        );
        assert_eq!(
            g.servers["h"],
            json!({"url": "https://h", "bearer_token_env_var": "PAT", "env_http_headers": {"X-K": "K"}})
        );
        assert_eq!(g.skipped, vec!["bad", "sse"]);
    }

    #[test]
    fn desktop_bridges_remote_servers() {
        let g = gen_for(
            json!({"mcpServers": {"r": {"type": "sse", "url": "https://r", "headers": {"A": "b"}}}}),
            "claude-desktop",
            "skip",
        );
        assert_eq!(
            g.servers["r"],
            json!({"command": "npx", "args": ["-y", "mcp-remote", "https://r", "--transport", "sse-only", "--header", "A:b"]})
        );
    }

    #[test]
    fn x_editors_and_wsl_keys() {
        let src = json!({"mcpServers": {
            "only": {"command": "y", "x-editors": ["cursor"]},
            "w": {"command": "n", "env": {"A": "1", "WSL_A": "2", "WSL_ONLY": "3"}}
        }});
        let g = gen_for(src.clone(), "claude-code", "skip");
        assert!(!g.servers.contains_key("only"));
        assert_eq!(g.servers["w"], json!({"command": "n", "env": {"A": "1"}, "type": "stdio"}));
        assert!(gen_for(src, "cursor", "skip").servers.contains_key("only"));
    }

    #[test]
    fn npx_wrap() {
        let g = generate(
            &json!({"mcpServers": {"a": {"command": "npx", "args": ["-y", "p"]}}}),
            "claude-code",
            "skip",
            true,
        );
        assert_eq!(g.servers["a"], json!({"command": "cmd", "args": ["/c", "npx", "-y", "p"], "type": "stdio"}));
    }

    #[test]
    fn jsonc_is_stripped() {
        let t = "{\n // c\n \"a\": \"x//y\", /* b */ \"b\": [1, 2,],\n}";
        let v: Value = serde_json::from_str(&strip_jsonc(t)).unwrap();
        assert_eq!(v, json!({"a": "x//y", "b": [1, 2]}));
    }

    #[test]
    fn toml_plan_keeps_unmanaged_content() {
        let text =
            "model = \"x\"\n\n[mcp_servers.old]\ncommand = \"a\"\n# about profile\n\n[profile.custom]\nmodel = \"y\"\n";
        let mut servers = Obj::new();
        servers.insert("new".into(), json!({"command": "b"}));
        let g = Gen { servers, warnings: vec![], skipped: vec![] };
        let p = plan_toml(text, &g, &["old".to_string()]);
        assert_eq!(
            p.new_text,
            "model = \"x\"\n\n# about profile\n\n[profile.custom]\nmodel = \"y\"\n\n[mcp_servers.new]\n# managed by mcpsync: edit the mcpsync source, then run `mcpsync sync`\ncommand = \"b\"\n"
        );
    }

    #[test]
    fn import_from_codex_restores_refs() {
        let o = from_editor(
            "codex",
            json!({"url": "https://h", "bearer_token_env_var": "PAT", "env_http_headers": {"X": "K"}})
                .as_object()
                .unwrap(),
        );
        assert_eq!(
            Value::Object(o),
            json!({"url": "https://h", "type": "http", "headers": {"X": "${K}", "Authorization": "Bearer ${PAT}"}})
        );
    }

    #[test]
    fn literal_secret_detection() {
        let s = json!({"url": "https://x?apiKey=abc", "headers": {"Authorization": "Bearer sk-1"}, "env": {"API_TOKEN": "t", "HOST": "h"}});
        assert_eq!(literal_secrets(s.as_object().unwrap()), vec!["env.API_TOKEN", "headers.Authorization", "url"]);
    }
}
