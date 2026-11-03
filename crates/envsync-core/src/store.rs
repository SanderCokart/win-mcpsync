//! Where variables live: the Windows user environment (HKCU\Environment) plus WSLENV.
//!
//! Setting ENVSYNC_TEST_STORE to a file path swaps the registry for a JSON file, so tests
//! never touch the real user environment.

use serde_json::{Map, Value};
use std::path::PathBuf;

pub const TEST_STORE_VAR: &str = "ENVSYNC_TEST_STORE";
const WSLENV: &str = "WSLENV";

pub type Result<T> = std::result::Result<T, String>;

/// A name usable as an environment variable and as a ${NAME} reference.
pub fn valid_name(n: &str) -> bool {
    let mut c = n.chars();
    matches!(c.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && c.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

enum Backend {
    Registry,
    File(PathBuf),
}

pub struct Store {
    backend: Backend,
}

impl Store {
    pub fn open() -> Store {
        match std::env::var_os(TEST_STORE_VAR) {
            Some(p) if !p.is_empty() => Store { backend: Backend::File(PathBuf::from(p)) },
            _ => Store { backend: Backend::Registry },
        }
    }

    /// Every variable name in the user environment.
    pub fn names(&self) -> Result<Vec<String>> {
        match &self.backend {
            Backend::Registry => registry::names(),
            Backend::File(f) => Ok(file_read(f)?.keys().cloned().collect()),
        }
    }

    /// Whether NAME is set in the user environment (names are case-insensitive on Windows).
    pub fn is_set(&self, name: &str) -> Result<bool> {
        Ok(self.names()?.iter().any(|n| n.eq_ignore_ascii_case(name)))
    }

    /// Names listed in the user WSLENV (flags such as /u are dropped).
    pub fn wslenv(&self) -> Result<Vec<String>> {
        Ok(wslenv_parts(&self.get(WSLENV)?.unwrap_or_default()).iter().map(|p| part_name(p).to_string()).collect())
    }

    /// Stores NAME in the user environment and shares it with WSL.
    pub fn set(&self, name: &str, value: &str) -> Result<()> {
        check_name(name)?;
        self.put(name, Some(value))?;
        self.wslenv_add(&[name])?;
        notify();
        Ok(())
    }

    /// Removes NAME from the user environment and from WSLENV.
    pub fn unset(&self, name: &str) -> Result<()> {
        check_name(name)?;
        self.put(name, None)?;
        let parts = wslenv_parts(&self.get(WSLENV)?.unwrap_or_default());
        let kept: Vec<String> = parts.into_iter().filter(|p| !part_name(p).eq_ignore_ascii_case(name)).collect();
        self.put(WSLENV, Some(&kept.join(":")))?;
        notify();
        Ok(())
    }

    /// Adds NAME/u to WSLENV for each name not already listed. Returns the names it added.
    pub fn wslenv_add(&self, names: &[&str]) -> Result<Vec<String>> {
        let mut parts = wslenv_parts(&self.get(WSLENV)?.unwrap_or_default());
        let mut added = Vec::new();
        for &n in names {
            check_name(n)?;
            if !parts.iter().any(|p| part_name(p).eq_ignore_ascii_case(n)) {
                parts.push(format!("{n}/u"));
                added.push(n.to_string());
            }
        }
        if !added.is_empty() {
            self.put(WSLENV, Some(&parts.join(":")))?;
            notify();
        }
        Ok(added)
    }

    fn get(&self, name: &str) -> Result<Option<String>> {
        match &self.backend {
            Backend::Registry => registry::get(name),
            Backend::File(f) => Ok(find_key(&file_read(f)?, name).and_then(|(_, v)| v.as_str().map(String::from))),
        }
    }

    fn put(&self, name: &str, value: Option<&str>) -> Result<()> {
        match &self.backend {
            Backend::Registry => registry::put(name, value),
            Backend::File(f) => {
                let mut m = file_read(f)?;
                if let Some((k, _)) = find_key(&m, name) {
                    let k = k.clone();
                    m.shift_remove(&k);
                }
                if let Some(v) = value {
                    m.insert(name.to_string(), Value::String(v.to_string()));
                }
                let text = serde_json::to_string_pretty(&Value::Object(m)).map_err(|e| e.to_string())?;
                std::fs::write(f, text + "\n").map_err(|e| format!("can't write {}: {e}", f.display()))
            }
        }
    }
}

fn check_name(n: &str) -> Result<()> {
    if valid_name(n) {
        Ok(())
    } else {
        Err(format!("not a valid variable name: {n}"))
    }
}

fn wslenv_parts(v: &str) -> Vec<String> {
    v.split(':').filter(|p| !p.is_empty()).map(String::from).collect()
}

fn part_name(p: &str) -> &str {
    p.split('/').next().unwrap_or(p)
}

fn find_key<'a>(m: &'a Map<String, Value>, name: &str) -> Option<(&'a String, &'a Value)> {
    m.iter().find(|(k, _)| k.eq_ignore_ascii_case(name))
}

fn file_read(f: &PathBuf) -> Result<Map<String, Value>> {
    match std::fs::read_to_string(f) {
        Ok(t) if t.trim().is_empty() => Ok(Map::new()),
        Ok(t) => match serde_json::from_str::<Value>(&t) {
            Ok(Value::Object(m)) => Ok(m),
            _ => Err(format!("{} isn't a JSON object", f.display())),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Map::new()),
        Err(e) => Err(format!("can't read {}: {e}", f.display())),
    }
}

/// Tells running programs (Explorer, new terminals) that the environment changed.
fn notify() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
        };
        if std::env::var_os(TEST_STORE_VAR).is_some_and(|v| !v.is_empty()) {
            return;
        }
        let what: Vec<u16> = "Environment\0".encode_utf16().collect();
        let mut result = 0usize;
        // SAFETY: `what` is a NUL-terminated UTF-16 string that outlives the call.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                what.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                5000,
                &mut result,
            );
        }
    }
}

#[cfg(windows)]
mod registry {
    use super::Result;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;

    fn key(write: bool) -> Result<RegKey> {
        let flags = if write { KEY_READ | KEY_WRITE } else { KEY_READ };
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Environment", flags)
            .map_err(|e| format!("can't open HKCU\\Environment: {e}"))
    }

    pub fn names() -> Result<Vec<String>> {
        let k = key(false)?;
        Ok(k.enum_values().filter_map(|r| r.ok()).map(|(n, _)| n).collect())
    }

    pub fn get(name: &str) -> Result<Option<String>> {
        match key(false)?.get_value::<String, _>(name) {
            Ok(v) => Ok(Some(v)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("can't read user variable {name}: {e}")),
        }
    }

    pub fn put(name: &str, value: Option<&str>) -> Result<()> {
        let k = key(true)?;
        match value {
            Some(v) => k.set_value(name, &v.to_string()).map_err(|e| format!("can't set user variable {name}: {e}")),
            None => match k.delete_value(name) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("can't remove user variable {name}: {e}")),
            },
        }
    }
}

#[cfg(not(windows))]
mod registry {
    use super::Result;
    const MSG: &str = "envsync v1 only supports Windows";
    pub fn names() -> Result<Vec<String>> {
        Err(MSG.into())
    }
    pub fn get(_: &str) -> Result<Option<String>> {
        Err(MSG.into())
    }
    pub fn put(_: &str, _: Option<&str>) -> Result<()> {
        Err(MSG.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(tag: &str) -> Store {
        let f = std::env::temp_dir().join(format!("envsync-test-{tag}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&f);
        Store { backend: Backend::File(f) }
    }

    #[test]
    fn names_are_validated() {
        assert!(valid_name("GITLAB_TOKEN"));
        assert!(valid_name("_x1"));
        assert!(!valid_name("1X"));
        assert!(!valid_name("A-B"));
        assert!(!valid_name(""));
    }

    #[test]
    fn set_shares_with_wsl_once_and_unset_removes() {
        let s = temp_store("set");
        s.set("ABC", "secret").unwrap();
        s.set("abc", "again").unwrap();
        assert!(s.is_set("ABC").unwrap());
        assert_eq!(s.wslenv().unwrap(), vec!["ABC"]);
        s.unset("Abc").unwrap();
        assert!(!s.is_set("ABC").unwrap());
        assert!(s.wslenv().unwrap().is_empty());
    }

    #[test]
    fn wslenv_add_keeps_existing_flags() {
        let s = temp_store("flags");
        s.put(WSLENV, Some("X/p:Y")).unwrap();
        assert_eq!(s.wslenv_add(&["X", "Z"]).unwrap(), vec!["Z"]);
        assert_eq!(s.get(WSLENV).unwrap().unwrap(), "X/p:Y:Z/u");
    }
}
