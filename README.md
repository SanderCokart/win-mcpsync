# win-mcpsync — one MCP config for every editor, secrets in your Windows environment

Two small Windows tools:

| Tool | What it does |
|---|---|
| **mcpsync** | Keeps your MCP server list in one file and writes it into each editor's own config: Claude Code, Cursor, Codex, Claude Desktop, VS Code, Gemini CLI and Devin. |
| **envsync** | Stores secrets as Windows user environment variables and shares them with WSL. mcpsync uses it, and it also works on its own. |

Secrets stay out of the config files: you write `${NAME}` in the server list, and the value lives in a Windows user environment variable.

## Install

PowerShell:

```powershell
irm https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.ps1 | iex
```

Git Bash (or any `sh`):

```bash
curl -fsSL https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.sh | sh
```

This installs `mcpsync.exe` and `envsync.exe` to `%LOCALAPPDATA%\Programs\win-mcpsync\bin` and adds that folder to your user PATH. No admin rights are needed. Open a new terminal afterwards. Running the installer again upgrades in place.

To install only envsync:

```powershell
& ([scriptblock]::Create((irm https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.ps1))) -EnvSync
```

```bash
curl -fsSL https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.sh | sh -s -- --envsync
```

Pin a version with `-Version 1.0.0` (PowerShell) or `--version 1.0.0` (sh). Both installers check every download against the release's `SHA256SUMS` and install nothing if one doesn't match.

## Quick start

```powershell
mcpsync init                 # asks which editors you use, creates the server list
mcpsync edit                 # list your servers; write every secret as ${NAME}
envsync set GITHUB_MCP_PAT   # store each secret once (hidden prompt)
mcpsync check                # which ${NAME}s are still missing
mcpsync sync --dry-run       # preview the change per editor
mcpsync sync                 # write it
```

Then fully quit and reopen your editors. `INSTRUCTIONS.md` has the step-by-step version, and `PROMPT.md` is a prompt you can give an AI agent to do the setup for you.

**For AI agents:** `mcpsync agent-instructions` and `envsync agent-instructions` print Markdown guidance for agents: how to handle secrets, the safe order of work, exit codes and pitfalls. Point an agent at them, for example in an `AGENTS.md` or `CLAUDE.md`:

```markdown
Before changing MCP servers or secrets, run `mcpsync agent-instructions` and `envsync agent-instructions` and follow them.
```

## Requirements

- Windows 10 or 11, 64-bit
- Node.js, only if your MCP servers start with `npx`
- Git Bash only if you install with the `curl` line; the tools themselves run in any terminal

## Uninstall

```powershell
mcpsync uninstall
envsync uninstall
```

Each prints a command to paste: a running program can't delete itself on Windows. `envsync uninstall` refuses while mcpsync is installed, because mcpsync needs it (use `--force` to remove it anyway). Your variables and `~/.config/mcpsync` are kept.

## Good to know

- **Project files are separate.** Per-project MCP files (`.mcp.json`, `.cursor/mcp.json`) are managed with `mcpsync --project sync` from a `.mcpsync.json` in the project. A user-level sync never touches them.
- **Backups contain old values.** mcpsync backs up each editor file before it changes it, and those backups can hold plaintext secrets from before the migration. Delete `%USERPROFILE%\.config\mcpsync\backups` once you have rotated those keys.
- **Secrets are never printed.** `envsync list`, `mcpsync check` and `mcpsync secret list` show names only.
- `mcpsync secret set NAME` still works and does the same as `envsync set NAME`.

## Repository layout

| Path | What it is |
|---|---|
| `crates/envsync-core` | Shared library: the variable store (registry + WSLENV), prompts, uninstall helper |
| `crates/envsync` | The `envsync` command |
| `crates/mcpsync` | The `mcpsync` command |
| `install.ps1`, `install.sh` | The one-line installers |
| `.github/workflows/release.yml` | Builds, tests and publishes a release when a `v*` tag is pushed |
| `test/run.sh` | Acceptance suite. Runs in a throwaway home folder and a fake variable store; never touches your config or environment. |
| `docs/mcpsync-reference.md` | Every command and option, and how `${VAR}` is translated per editor |
| `examples/mcp.json` | Template server list with placeholders |
| `mcpsync` | The previous bash version, kept as the reference until it's retired |

## Development

```bash
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
bash test/run.sh target/release/mcpsync.exe   # acceptance suite (Git Bash)
bash test/run.sh                              # same suite against the bash version
```

To release: set the version in `Cargo.toml`, then push a matching tag (`git tag v1.0.0 && git push origin v1.0.0`).
