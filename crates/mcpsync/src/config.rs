//! ~/.config/mcpsync/config (KEY="value" lines, the format the bash version sourced) and
//! where each editor keeps its MCP file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct Settings {
    pub editors: Vec<String>,
    pub fallback: String,
    pub backups: usize,
    pub npx_wrap: Vec<String>,
    vars: HashMap<String, String>,
}

impl Settings {
    pub fn load(file: &Path) -> Settings {
        let vars = std::fs::read_to_string(file).map(|t| parse(&t)).unwrap_or_default();
        let words = |k: &str| -> Vec<String> {
            vars.get(k).map(|v| v.split_whitespace().map(String::from).collect()).unwrap_or_default()
        };
        Settings {
            editors: words("EDITORS"),
            fallback: vars.get("FALLBACK").cloned().unwrap_or_else(|| "skip".into()),
            backups: vars.get("BACKUPS").and_then(|v| v.trim().parse().ok()).unwrap_or(10),
            npx_wrap: words("NPX_WRAP_EDITORS"),
            vars,
        }
    }

    /// PATH_<EDITOR> from the settings file, else from the environment.
    pub fn path_override(&self, ed: &str) -> Option<PathBuf> {
        let key = format!("PATH_{}", ed.to_ascii_uppercase().replace('-', "_"));
        self.vars
            .get(&key)
            .cloned()
            .or_else(|| std::env::var(&key).ok())
            .filter(|v| !v.is_empty())
            .map(|v| win_path(&v))
    }
}

/// Parses `NAME=value` lines with shell quoting and $VAR / ${VAR} expansion.
fn parse(text: &str) -> HashMap<String, String> {
    let mut vars: HashMap<String, String> = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, rest)) = line.split_once('=') else { continue };
        if !envsync_core::valid_name(name) {
            continue;
        }
        let value = shell_word(rest, &vars);
        vars.insert(name.to_string(), value);
    }
    vars
}

fn shell_word(s: &str, vars: &HashMap<String, String>) -> String {
    let c: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let mut quote: Option<char> = None;
    while i < c.len() {
        let ch = c[i];
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    out.push(ch);
                }
            }
            q => {
                if q.is_none() && (ch.is_whitespace() || ch == '#') {
                    break;
                }
                if ch == '\\' && i + 1 < c.len() {
                    out.push(c[i + 1]);
                    i += 2;
                    continue;
                }
                if q == Some('"') && ch == '"' {
                    quote = None;
                } else if q.is_none() && (ch == '"' || ch == '\'') {
                    quote = Some(ch);
                } else if ch == '$' {
                    let (name, next) = var_name(&c, i + 1);
                    if name.is_empty() {
                        out.push('$');
                    } else {
                        let v = vars.get(&name).cloned().or_else(|| std::env::var(&name).ok());
                        out.push_str(&v.unwrap_or_default());
                    }
                    i = next;
                    continue;
                } else {
                    out.push(ch);
                }
            }
        }
        i += 1;
    }
    out
}

fn var_name(c: &[char], start: usize) -> (String, usize) {
    if c.get(start) == Some(&'{') {
        if let Some(end) = (start + 1..c.len()).find(|&k| c[k] == '}') {
            return (c[start + 1..end].iter().collect(), end + 1);
        }
        return (String::new(), start);
    }
    let mut j = start;
    while j < c.len() && (c[j] == '_' || c[j].is_ascii_alphanumeric()) {
        j += 1;
    }
    (c[start..j].iter().collect(), j)
}

/// Turns a Git Bash path (/c/Users/x) into a Windows one (C:/Users/x); other paths pass through.
pub fn win_path(s: &str) -> PathBuf {
    let b = s.as_bytes();
    if b.len() >= 2 && b[0] == b'/' && b[1].is_ascii_alphabetic() && (b.len() == 2 || b[2] == b'/') {
        return PathBuf::from(format!("{}:/{}", (b[1] as char).to_ascii_uppercase(), &s[b.len().min(3)..]));
    }
    PathBuf::from(s)
}

/// The Git Bash spelling of a Windows path (C:\Users\x -> /c/Users/x). Used for state keys
/// so project state written by the bash version still matches.
pub fn msys_path(p: &Path) -> String {
    let s = p.display().to_string().replace('\\', "/");
    let b = s.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        format!("/{}{}", (b[0] as char).to_ascii_lowercase(), &s[2..])
    } else {
        s
    }
}

fn env_path(k: &str) -> Option<PathBuf> {
    std::env::var(k).ok().filter(|v| !v.is_empty()).map(|v| win_path(&v))
}

/// HOME as Git Bash sets it, else USERPROFILE.
pub fn home_dir() -> PathBuf {
    env_path("HOME").or_else(|| env_path("USERPROFILE")).unwrap_or_else(|| PathBuf::from("."))
}

pub fn appdata() -> PathBuf {
    env_path("APPDATA").unwrap_or_else(|| home_dir().join("AppData").join("Roaming"))
}

pub fn codex_home() -> PathBuf {
    env_path("CODEX_HOME").unwrap_or_else(|| home_dir().join(".codex"))
}

pub fn mcpsync_home() -> PathBuf {
    env_path("MCPSYNC_HOME")
        .or_else(|| env_path("XDG_CONFIG_HOME").map(|x| x.join("mcpsync")))
        .unwrap_or_else(|| home_dir().join(".config").join("mcpsync"))
}

pub fn source_override() -> Option<PathBuf> {
    env_path("MCPSYNC_SOURCE")
}

/// The user-level file an editor reads.
pub fn user_editor_file(ed: &str, s: &Settings) -> PathBuf {
    if let Some(p) = s.path_override(ed) {
        return p;
    }
    let home = home_dir();
    match ed {
        "claude-code" => env_path("CLAUDE_CONFIG_DIR").unwrap_or(home).join(".claude.json"),
        "cursor" => home.join(".cursor").join("mcp.json"),
        "codex" => codex_home().join("config.toml"),
        "claude-desktop" => appdata().join("Claude").join("claude_desktop_config.json"),
        "vscode" => appdata().join("Code").join("User").join("mcp.json"),
        "gemini" => home.join(".gemini").join("settings.json"),
        "devin" => appdata().join("devin").join("mcp_config.json"),
        _ => unreachable!("unsupported editor {ed}"),
    }
}

/// The project-level file an editor reads (None: Claude Desktop has none).
pub fn project_editor_file(ed: &str, dir: &Path) -> Option<PathBuf> {
    Some(match ed {
        "claude-code" => dir.join(".mcp.json"),
        "cursor" => dir.join(".cursor").join("mcp.json"),
        "codex" => dir.join(".codex").join("config.toml"),
        "vscode" => dir.join(".vscode").join("mcp.json"),
        "gemini" => dir.join(".gemini").join("settings.json"),
        "devin" => dir.join(".devin").join("mcp_config.json"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bash_style_settings() {
        let v =
            parse("# c\nEDITORS=\"cursor codex\"\nFALLBACK=resolve # x\nBACKUPS='5'\nPATH_CURSOR=\"${EDITORS}/x\"\n");
        assert_eq!(v["EDITORS"], "cursor codex");
        assert_eq!(v["FALLBACK"], "resolve");
        assert_eq!(v["BACKUPS"], "5");
        assert_eq!(v["PATH_CURSOR"], "cursor codex/x");
    }

    #[test]
    fn git_bash_paths() {
        assert_eq!(win_path("/c/Users/x"), PathBuf::from("C:/Users/x"));
        assert_eq!(win_path("C:\\x"), PathBuf::from("C:\\x"));
        assert_eq!(msys_path(Path::new("C:\\Users\\x")), "/c/Users/x");
    }
}
