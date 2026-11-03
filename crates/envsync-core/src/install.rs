//! Install location helpers and the copy-paste uninstall command.

use std::path::{Path, PathBuf};

/// Folder the installers create under %LOCALAPPDATA%\Programs.
pub const INSTALL_DIR_NAME: &str = "win-mcpsync";

/// Finds a program on PATH the way Windows does (PATHEXT), e.g. "code" -> code.cmd.
pub fn find_on_path(cmd: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into())
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| e.to_ascii_lowercase())
        .collect();
    let has_ext = Path::new(cmd).extension().is_some();
    for dir in std::env::split_paths(&path) {
        if has_ext {
            let p = dir.join(cmd);
            if p.is_file() {
                return Some(p);
            }
        }
        for e in &exts {
            let p = dir.join(format!("{cmd}{e}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// Prints commands that remove `exe` and, once its folder is empty, the PATH entry.
/// A running .exe can't delete itself on Windows, so the user runs this after we exit.
pub fn print_uninstall(app: &str) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("can't find {app}.exe: {e}"))?;
    let (ps, sh) = uninstall_commands(&exe);
    println!("To remove {app}, close it and paste this into PowerShell:");
    println!();
    println!("{ps}");
    println!();
    println!("or into Git Bash:");
    println!();
    println!("{sh}");
    println!();
    println!("Your variables and ~/.config/mcpsync are kept.");
    Ok(())
}

pub fn uninstall_commands(exe: &Path) -> (String, String) {
    let bin = exe.parent().unwrap_or(exe);
    let mut ps = format!(
        "Remove-Item -Force -LiteralPath {exe}; \
         if (-not (Get-ChildItem -Force -LiteralPath {bin})) {{ \
         $k = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey(\"Environment\", $true); \
         $p = $k.GetValue(\"Path\", \"\", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames); \
         if ($p) {{ $k.SetValue(\"Path\", (($p -split \";\" | Where-Object {{ $_ -and $_.TrimEnd(\"\\\") -ne {bin_cmp} }}) -join \";\"), $k.GetValueKind(\"Path\")) }}; \
         $k.Close(); \
         Remove-Item -LiteralPath {bin}",
        exe = ps_quote(&exe.display().to_string()),
        bin = ps_quote(&bin.display().to_string()),
        bin_cmp = ps_quote(bin.display().to_string().trim_end_matches('\\')),
    );
    if let Some(top) = bin.parent().filter(|p| p.file_name().is_some_and(|n| n == INSTALL_DIR_NAME)) {
        let top = ps_quote(&top.display().to_string());
        ps.push_str(&format!(
            "; if (-not (Get-ChildItem -Force -LiteralPath {top})) {{ Remove-Item -LiteralPath {top} }}"
        ));
    }
    ps.push_str(" }");
    let sh = format!("powershell.exe -NoProfile -Command '{}'", ps.replace('\'', "'\\''"));
    (ps, sh)
}

/// A PowerShell double-quoted string literal.
fn ps_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('`', "``").replace('$', "`$").replace('"', "`\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uninstall_command_targets_only_our_files() {
        let exe = Path::new(r"C:\Users\x\AppData\Local\Programs\win-mcpsync\bin\mcpsync.exe");
        let (ps, sh) = uninstall_commands(exe);
        assert!(ps.starts_with(
            r#"Remove-Item -Force -LiteralPath "C:\Users\x\AppData\Local\Programs\win-mcpsync\bin\mcpsync.exe""#
        ));
        assert!(ps.contains(r#"-ne "C:\Users\x\AppData\Local\Programs\win-mcpsync\bin""#));
        assert!(ps.contains(r#"Remove-Item -LiteralPath "C:\Users\x\AppData\Local\Programs\win-mcpsync" }"#));
        assert!(sh.starts_with("powershell.exe -NoProfile -Command 'Remove-Item"));
        assert!(!ps.contains("-Recurse"));
    }

    #[test]
    fn ps_quote_escapes_specials() {
        assert_eq!(ps_quote("a$b`c\"d"), "\"a`$b``c`\"d\"");
    }
}
