# mcpsync

One MCP config, synced to every editor you use. Secrets stay out of the files: you write `${VAR}`, mcpsync translates it to each editor's syntax, and the values live in your environment.

Supported editors: `claude-code`, `cursor`, `codex`, `claude-desktop`, `vscode`, `gemini` (Gemini CLI), `devin` (Devin Desktop / CLI, formerly Windsurf).

## Install

PowerShell:

```powershell
irm https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.ps1 | iex
```

Git Bash or any `sh`:

```bash
curl -fsSL https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.sh | sh
```

Both install `mcpsync.exe` and `envsync.exe` to `%LOCALAPPDATA%\Programs\win-mcpsync\bin` and add it to the user PATH. Options: `-EnvSync` / `--envsync` installs only envsync, and `-Version X.Y.Z` / `--version X.Y.Z` pins a release. With `irm | iex`, set `$env:WIN_MCPSYNC_APP = 'envsync'` or `$env:WIN_MCPSYNC_VERSION` first, or use the `[scriptblock]::Create(...)` form shown in the README to pass flags.

Installer environment variables, mainly for testing an unpublished build:

| Variable | Effect |
|---|---|
| `WIN_MCPSYNC_DIST` | Folder with release assets (`*.zip`, `SHA256SUMS`) to install from instead of GitHub |
| `WIN_MCPSYNC_INSTALL_DIR` | Install somewhere other than `%LOCALAPPDATA%\Programs\win-mcpsync\bin` |
| `WIN_MCPSYNC_NO_PATH` | Any value: don't touch the user PATH |

Node.js is only needed by MCP servers that start with `npx`. mcpsync itself doesn't need Node or Git Bash.

## First run

```powershell
mcpsync init                          # asks which editors you use
mcpsync import cursor                 # optional: start from what an editor already has
mcpsync edit                          # write each secret as ${NAME}
envsync set CONTEXT7_API_KEY          # repeat for each ${NAME}
mcpsync check                         # anything still missing?
mcpsync sync --dry-run                # see the diff per editor
mcpsync sync                          # write (backups in ~/.config/mcpsync/backups)
```

Then fully quit and reopen Cursor, Claude and Codex.

## Daily use

```powershell
mcpsync edit                 # opens the source in $VISUAL/$EDITOR (Notepad if unset), then validates
mcpsync sync                 # push changes to every editor in your settings
mcpsync status               # in sync / out of sync per editor
mcpsync check                # ${NAME}s missing from the user environment (exit 1 if any)
mcpsync doctor               # node, settings, variables, editors
mcpsync --project sync       # project files from ./.mcpsync.json
```

mcpsync only manages the servers it wrote. Servers you added in an editor by hand are left alone, and a server you delete from the source is removed from every editor on the next sync. The source can contain comments, because it's read as JSONC.

`~/.config/mcpsync/config` holds `EDITORS`, `FALLBACK`, `BACKUPS`, `NPX_WRAP_EDITORS` and optional `PATH_<EDITOR>` overrides as `KEY="value"` lines. It's the same file the bash version used, so either version reads it. `$HOME` and other `$VARS` in it are expanded, and Git Bash paths such as `/c/Users/me` are accepted.

## Secrets (envsync)

```powershell
envsync set NAME          # hidden prompt, or one line on stdin
envsync unset NAME
envsync has NAME          # exit 0 if set, 1 if not
envsync list              # names in your user environment and whether WSL gets them
envsync wslenv [NAME...]  # list, or share, names with WSL
```

`envsync set` stores the value as a **Windows user environment variable** and adds `NAME/u` to **WSLENV**, so PowerShell, Git Bash, WSL and every GUI app share one source of truth. Reopen terminals and editors afterwards, and run `wsl --shutdown` once for WSL. Values are never printed.

`mcpsync secret set|unset NAME` does the same as envsync and says so. `mcpsync secret list` shows where each `${NAME}` in the source is set, and `mcpsync secret wslenv` shares all of them with WSL.

For tests, `ENVSYNC_TEST_STORE=<file>` makes envsync (and mcpsync) use that JSON file instead of the user environment. Don't set it otherwise: values in that file are stored in plain text.

## Agent instructions

`mcpsync agent-instructions` and `envsync agent-instructions` (also `--agent-instructions`) print Markdown written for AI agents: secret handling, the safe order of work with a done-condition per step, exit codes and pitfalls. The text is built into each binary, so it always matches the installed version, and printing it needs no config. The sources are `crates/mcpsync/src/agent-instructions.md` and `crates/envsync/src/agent-instructions.md`.

## Uninstall

`mcpsync uninstall` and `envsync uninstall` print a PowerShell command and a Git Bash command to paste after the tool has exited. The command removes the `.exe` and, once the `bin` folder is empty, its PATH entry and the folder. `envsync uninstall` refuses while mcpsync is installed; add `--force` to remove it anyway. Your variables and `~/.config/mcpsync` are kept.

## How `${VAR}` is translated

| Editor | Written as | Notes |
|---|---|---|
| claude-code | `${VAR}`, `${VAR:-default}` | user: `~/.claude.json`, project: `.mcp.json` |
| cursor | `${env:VAR}` | defaults are dropped |
| codex | `env_vars`, `bearer_token_env_var`, `env_http_headers` | `~/.codex/config.toml` (or `$CODEX_HOME`); see below |
| vscode | `${env:VAR}`, top-level `servers` | |
| devin | `${env:VAR}`, `transport` | file is `mcp_config.json` (`%APPDATA%\devin` on Windows); remote servers take `url` and `transport` (`http` or `sse`) |
| gemini | `${VAR}`, `${VAR:-default}` | http → `httpUrl` |
| claude-desktop | nothing is expanded | every `${VAR}` follows `FALLBACK`; remote servers are bridged via `mcp-remote` |

**Codex** has no `${VAR}` expansion and reads secrets from its own environment instead. mcpsync maps these forms:

- `"env": { "NAME": "${NAME}" }` becomes `env_vars = ["NAME"]`.
- `"Authorization": "Bearer ${X}"` becomes `bearer_token_env_var = "X"`.
- `"Some-Header": "${X}"` becomes `env_http_headers = { "Some-Header" = "X" }`.
- Literal values are copied as they are.

Anything else that uses a `${VAR}` follows `FALLBACK`. Ref's key goes in an `x-ref-api-key` header, which Codex supports, so it works in Codex without a URL-embedded key. A server that puts a `${VAR}` in its URL (such as the old `?apiKey=` form) still follows `FALLBACK`. SSE servers are skipped, because Codex only supports stdio and streamable HTTP. mcpsync rewrites only the `[mcp_servers.*]` tables it manages; the rest of `config.toml` is left untouched.

`FALLBACK` in `~/.config/mcpsync/config` can be `skip` (the default, which leaves the server out for that editor), `resolve` (writes the value in plaintext, user files only), or `keep`.

Per-server targeting: add `"x-editors": ["cursor"]` or `"x-exclude-editors": ["claude-desktop"]` to a server in the source.

### WSL-only environment values

An `env` key starting with `WSL_` is meant for WSL only. mcpsync runs on the Windows host, so `WSL_` keys are left out of every file it writes, and `NAME` must be set there if you need it. (The bash version also ran inside WSL, where `WSL_NAME` replaced `NAME`; v1 of the Rust version supports the Windows host only.)

```json
"env": {
  "NODE_EXTRA_CA_CERTS": "C:/certs/corp-root.pem",
  "WSL_NODE_EXTRA_CA_CERTS": "/etc/ssl/certs/ca-certificates.crt"
}
```

This only applies to `env` blocks. Headers and URLs don't get it.

## Caveats

- v1 supports the Windows host only. WSL gets your secrets through WSLENV, but mcpsync doesn't write WSL-side files such as `~/.claude.json` inside WSL.
- Claude Code rewrites `~/.claude.json` while it runs, so sync with Claude Code closed.
- Claude Code never expands its own credential variables (`ANTHROPIC_*`, cloud provider keys, `NPM_TOKEN`, `*_PROXY`) in a remote server's `url`/`headers`. Use your own names; `doctor` warns about this.
- If an editor's file has comments (VS Code allows them), they're removed when mcpsync rewrites it. A backup is kept.
- Backups can contain plaintext secrets from before the migration. Delete `~/.config/mcpsync/backups` once you've rotated those keys.
- If an `npx` server fails with `MODULE_NOT_FOUND`, its cached install under `%LOCALAPPDATA%\npm-cache\_npx\<hash>` is probably broken; delete that folder and restart the editor.
