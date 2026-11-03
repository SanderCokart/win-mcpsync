//! Just enough TOML for Codex's [mcp_servers.*] tables. Everything else in config.toml is
//! kept line for line.

use regex::Regex;
use serde_json::{Map, Number, Value};
use std::sync::LazyLock;

static HEADER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[\[?\s*([^\]]*?)\s*\]\]?\s*(#.*)?$").unwrap());
static BARE_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]+$").unwrap());
static FLOAT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$").unwrap());

pub struct Block {
    pub path: Option<Vec<String>>,
    pub lines: Vec<String>,
}

impl Block {
    pub fn is_server(&self) -> bool {
        self.path.as_ref().is_some_and(|p| p.len() >= 2 && p[0] == "mcp_servers")
    }

    pub fn server_name(&self) -> Option<&str> {
        if self.is_server() {
            self.path.as_ref().map(|p| p[1].as_str())
        } else {
            None
        }
    }
}

/// Splits a line on \n, dropping the \r of each \r\n (like JS `split(/\r?\n/)`).
pub fn split_lines(text: &str) -> Vec<String> {
    let parts: Vec<&str> = text.split('\n').collect();
    let last = parts.len() - 1;
    parts
        .iter()
        .enumerate()
        .map(|(i, p)| if i < last { p.strip_suffix('\r').unwrap_or(p) } else { p }.to_string())
        .collect()
}

/// The preamble, then one block per [table] header.
pub fn blocks(text: &str) -> Vec<Block> {
    let mut out = vec![Block { path: None, lines: vec![] }];
    for line in split_lines(text) {
        if let Some(c) = HEADER.captures(&line) {
            let path = parse_key_path(&c[1]);
            out.push(Block { path: Some(path), lines: vec![line] });
        } else {
            out.last_mut().unwrap().lines.push(line);
        }
    }
    out
}

pub fn parse_key_path(s: &str) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let n = c.len();
    let mut segs = Vec::new();
    let mut i = 0;
    while i < n {
        while i < n && (c[i] == ' ' || c[i] == '\t') {
            i += 1;
        }
        if i < n && c[i] == '"' {
            let mut j = i + 1;
            while j < n && c[j] != '"' {
                j += if c[j] == '\\' { 2 } else { 1 };
            }
            let end = j.min(n - 1);
            let lit: String = c[i..=end].iter().collect();
            segs.push(serde_json::from_str::<String>(&lit).unwrap_or_else(|_| c[i + 1..end].iter().collect()));
            i = j + 1;
        } else if i < n && c[i] == '\'' {
            let j = (i + 1..n).find(|&k| c[k] == '\'').unwrap_or(n);
            segs.push(c[i + 1..j].iter().collect());
            i = j + 1;
        } else {
            let mut j = i;
            while j < n && (c[j].is_ascii_alphanumeric() || c[j] == '_' || c[j] == '-') {
                j += 1;
            }
            if j == i {
                break;
            }
            segs.push(c[i..j].iter().collect());
            i = j;
        }
        while i < n && (c[i] == ' ' || c[i] == '\t') {
            i += 1;
        }
        if i < n && c[i] == '.' {
            i += 1;
        } else {
            break;
        }
    }
    segs
}

/// JSON string escapes are valid TOML, except that TOML also forbids a raw DEL (U+007F).
pub fn toml_str(s: &str) -> String {
    serde_json::to_string(s).unwrap().replace('\u{7f}', "\\u007f")
}

pub fn toml_key(k: &str) -> String {
    if BARE_KEY.is_match(k) {
        k.to_string()
    } else {
        toml_str(k)
    }
}

pub fn toml_val(v: &Value) -> String {
    match v {
        Value::String(s) => toml_str(s),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(a) => format!("[{}]", a.iter().map(toml_val).collect::<Vec<_>>().join(", ")),
        Value::Object(o) => format!(
            "{{ {} }}",
            o.iter().map(|(k, x)| format!("{} = {}", toml_key(k), toml_val(x))).collect::<Vec<_>>().join(", ")
        ),
        Value::Null => "\"\"".into(),
    }
}

pub fn server_toml(name: &str, o: &Map<String, Value>) -> Vec<String> {
    let mut lines = vec![
        format!("[mcp_servers.{}]", toml_key(name)),
        "# managed by mcpsync: edit the mcpsync source, then run `mcpsync sync`".to_string(),
    ];
    for (k, v) in o {
        lines.push(format!("{} = {}", toml_key(k), toml_val(v)));
    }
    lines
}

pub fn trim_end(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
}

/// The managed tables only, sorted, so "before" and "after" compare cleanly.
pub fn section(text: &str) -> String {
    let mut parts: Vec<String> = blocks(text)
        .into_iter()
        .filter(Block::is_server)
        .map(|b| {
            let mut l = b.lines;
            trim_end(&mut l);
            l.join("\n")
        })
        .collect();
    parts.sort();
    parts.join("\n\n") + "\n"
}

/// Parses the key = value lines of one table.
pub fn parse_kv(t: &str) -> Result<Map<String, Value>, String> {
    let mut p = Kv { c: t.chars().collect(), i: 0 };
    let mut o = Map::new();
    loop {
        p.ws();
        if p.i >= p.c.len() {
            break;
        }
        let k = p.key()?;
        let v = p.val()?;
        set_dotted(&mut o, &k, v);
    }
    Ok(o)
}

fn set_dotted(obj: &mut Map<String, Value>, path: &[String], v: Value) {
    let Some((last, head)) = path.split_last() else { return };
    let mut a = obj;
    for k in head {
        if !a.get(k).is_some_and(Value::is_object) {
            a.insert(k.clone(), Value::Object(Map::new()));
        }
        a = a.get_mut(k).and_then(Value::as_object_mut).unwrap();
    }
    a.insert(last.clone(), v);
}

struct Kv {
    c: Vec<char>,
    i: usize,
}

impl Kv {
    fn at(&self, k: usize) -> Option<char> {
        self.c.get(k).copied()
    }

    fn near(&self) -> String {
        self.c[self.i.min(self.c.len())..(self.i + 30).min(self.c.len())].iter().collect()
    }

    fn starts_with(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(k, ch)| self.at(self.i + k) == Some(ch))
    }

    fn find(&self, s: &str, from: usize) -> Option<usize> {
        let pat: Vec<char> = s.chars().collect();
        (from..self.c.len()).find(|&k| self.c[k..].starts_with(&pat))
    }

    fn ws(&mut self) {
        loop {
            while self.at(self.i).is_some_and(|ch| matches!(ch, ' ' | '\t' | '\r' | '\n')) {
                self.i += 1;
            }
            if self.at(self.i) == Some('#') {
                while self.at(self.i).is_some_and(|ch| ch != '\n') {
                    self.i += 1;
                }
            } else {
                break;
            }
        }
    }

    fn key(&mut self) -> Result<Vec<String>, String> {
        self.ws();
        let mut j = self.i;
        let mut q: Option<char> = None;
        while j < self.c.len() {
            let ch = self.c[j];
            if let Some(qc) = q {
                if ch == '\\' && qc == '"' {
                    j += 2;
                    continue;
                }
                if ch == qc {
                    q = None;
                }
            } else if ch == '"' || ch == '\'' {
                q = Some(ch);
            } else if ch == '=' {
                break;
            }
            j += 1;
        }
        if j >= self.c.len() {
            return Err(format!("expected \"=\" near: {}", self.near()));
        }
        let raw: String = self.c[self.i..j].iter().collect();
        self.i = j + 1;
        Ok(parse_key_path(raw.trim()))
    }

    fn val(&mut self) -> Result<Value, String> {
        self.ws();
        let Some(ch) = self.at(self.i) else { return Err(format!("unexpected end of TOML near: {}", self.near())) };
        if self.starts_with("\"\"\"") || self.starts_with("'''") {
            let q: String = self.c[self.i..self.i + 3].iter().collect();
            let j = self.find(&q, self.i + 3).ok_or_else(|| format!("unterminated string near: {}", self.near()))?;
            let s: String = self.c[self.i + 3..j].iter().collect();
            let s = s.strip_prefix("\r\n").or_else(|| s.strip_prefix('\n')).unwrap_or(&s).to_string();
            self.i = j + 3;
            return Ok(Value::String(s));
        }
        if ch == '"' {
            let mut j = self.i + 1;
            while j < self.c.len() && self.c[j] != '"' {
                j += if self.c[j] == '\\' { 2 } else { 1 };
            }
            let lit: String = self.c[self.i..(j + 1).min(self.c.len())].iter().collect();
            let s = serde_json::from_str::<String>(&lit).map_err(|e| e.to_string())?;
            self.i = j + 1;
            return Ok(Value::String(s));
        }
        if ch == '\'' {
            let j = self.find("'", self.i + 1).ok_or_else(|| format!("unterminated string near: {}", self.near()))?;
            let s: String = self.c[self.i + 1..j].iter().collect();
            self.i = j + 1;
            return Ok(Value::String(s));
        }
        if ch == '[' {
            self.i += 1;
            let mut arr = Vec::new();
            loop {
                self.ws();
                if self.at(self.i) == Some(']') {
                    self.i += 1;
                    return Ok(Value::Array(arr));
                }
                arr.push(self.val()?);
                self.ws();
                if self.at(self.i) == Some(',') {
                    self.i += 1;
                }
            }
        }
        if ch == '{' {
            self.i += 1;
            let mut obj = Map::new();
            loop {
                self.ws();
                if self.at(self.i) == Some('}') {
                    self.i += 1;
                    return Ok(Value::Object(obj));
                }
                if self.i >= self.c.len() {
                    return Err(format!("unterminated inline table near: {}", self.near()));
                }
                let k = self.key()?;
                let v = self.val()?;
                set_dotted(&mut obj, &k, v);
                self.ws();
                if self.at(self.i) == Some(',') {
                    self.i += 1;
                }
            }
        }
        let mut j = self.i;
        while self.at(j).is_some_and(|c| !(c == ',' || c == ']' || c == '}' || c == '#' || c.is_whitespace())) {
            j += 1;
        }
        if j == self.i {
            return Err(format!("unexpected TOML near: {}", self.near()));
        }
        let word: String = self.c[self.i..j].iter().collect();
        self.i = j;
        Ok(scalar(&word))
    }
}

fn scalar(w: &str) -> Value {
    match w {
        "true" => return Value::Bool(true),
        "false" => return Value::Bool(false),
        _ => {}
    }
    let s = w.replace('_', "");
    if let Ok(i) = s.parse::<i64>() {
        return Value::Number(i.into());
    }
    for (prefix, radix) in [("0x", 16), ("0o", 8), ("0b", 2)] {
        if let Some(rest) = s.strip_prefix(prefix) {
            if let Ok(i) = i64::from_str_radix(rest, radix) {
                return Value::Number(i.into());
            }
        }
    }
    if FLOAT.is_match(&s) {
        if let Some(n) = s.parse::<f64>().ok().and_then(Number::from_f64) {
            return Value::Number(n);
        }
    }
    Value::String(w.to_string())
}

/// Every [mcp_servers.NAME] table (and its sub-tables) as JSON objects.
pub fn parse_servers(text: &str) -> Result<Map<String, Value>, String> {
    let mut servers = Map::new();
    for b in blocks(text) {
        if !b.is_server() {
            continue;
        }
        let path = b.path.as_ref().unwrap();
        let kv =
            parse_kv(&b.lines[1..].join("\n")).map_err(|e| format!("could not parse [{}]: {e}", path.join(".")))?;
        let mut tgt = servers.entry(path[1].clone()).or_insert_with(|| Value::Object(Map::new()));
        if !tgt.is_object() {
            *tgt = Value::Object(Map::new());
        }
        for k in &path[2..] {
            let o = tgt.as_object_mut().unwrap();
            if !o.get(k).is_some_and(Value::is_object) {
                o.insert(k.clone(), Value::Object(Map::new()));
            }
            tgt = o.get_mut(k).unwrap();
        }
        let o = tgt.as_object_mut().unwrap();
        for (k, v) in kv {
            o.insert(k, v);
        }
    }
    Ok(servers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_paths() {
        assert_eq!(parse_key_path("mcp_servers.\"a.b\""), vec!["mcp_servers", "a.b"]);
        assert_eq!(parse_key_path("mcp_servers . 'x' .env"), vec!["mcp_servers", "x", "env"]);
    }

    #[test]
    fn parses_server_tables() {
        let t = "model = \"x\"\n[mcp_servers.gh]\nurl = \"https://h\"\nenv_http_headers = { \"X-K\" = \"K\" }\nstartup_timeout_sec = 20\n[mcp_servers.gh.env]\nA = 'b'\n";
        let s = parse_servers(t).unwrap();
        let gh = s["gh"].as_object().unwrap();
        assert_eq!(gh["url"], "https://h");
        assert_eq!(gh["env_http_headers"]["X-K"], "K");
        assert_eq!(gh["startup_timeout_sec"], 20);
        assert_eq!(gh["env"]["A"], "b");
    }

    #[test]
    fn values_round_trip() {
        let v: Value = serde_json::json!({"a": ["x", 1, true], "b c": {"d": "\u{7f}"}});
        assert_eq!(toml_val(&v), "{ a = [\"x\", 1, true], \"b c\" = { d = \"\\u007f\" } }");
    }
}
