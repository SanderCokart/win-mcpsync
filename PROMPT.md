# Prompt for an AI agent: set up mcpsync on this Windows machine

Paste everything below the line into an AI agent (for example Claude Code) on the Windows machine.

---

You are setting up **mcpsync** and **envsync** on this Windows computer for the person using it. mcpsync keeps one MCP server list and syncs it into their editors (Cursor, Claude Code and others). envsync stores the secrets as Windows user environment variables and shares them with WSL.

Read `README.md` and `INSTRUCTIONS.md` from https://github.com/SanderCokart/win-mcpsync first, or from the folder you're in if it has them.

## Hard rules

1. **Never ask the person to paste a secret value into this chat, and never print, log, echo, copy or commit a secret value.** You only handle secret NAMES. When a value is needed, tell the person the exact command to run themselves, for example `envsync set GITHUB_MCP_PAT`, and wait for them to confirm.
2. **Do not run `envsync set` or `mcpsync secret set` yourself.** They need a hidden prompt that the person must answer.
3. **Do not run `mcpsync sync` (without `--dry-run`) until the person confirms** that the secrets are set and that they want the sync. Before that, only run `--dry-run`, and do not show its diff lines in full: they can contain current values. Show only the summary lines (lines starting with `✓`, `!`, `✗`, or `error`).
4. **Back up before changing any file that isn't yours** (anything under the person's home folder, editor configs, or `.config\mcpsync`). Use a `.bak-<date>` copy next to the original.
5. **Do not delete anything without asking.** That includes old backups, old editor entries, npx cache folders and servers that aren't in the new list.
6. **Do not change system or security settings** (PowerShell execution policy, Windows Defender, firewall, WSL configuration). The win-mcpsync installer is the only installer you may run, and only after the person agrees.
7. **Keep every answer short and plain.** Tell the person what you did and what they still have to do.

## Steps

1. **Install.** Check whether `mcpsync --version` and `envsync --version` work. If not, ask the person to approve installing, then run `irm https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.ps1 | iex` in PowerShell. Tell them new terminals pick up the PATH change. Then read `mcpsync agent-instructions` and `envsync agent-instructions`: they describe the installed version.
2. **Create the settings.** Run `mcpsync init --editors "claude-code cursor"` (adjust to the editors they use; `mcpsync editors` shows which are detected).
3. **Read the person's server list.** Ask which MCP servers they use and what each one needs (the URL, and which token or key goes where). Don't ask for the token values themselves. If an editor already has servers configured, `mcpsync import <editor>` copies them into the source; it warns about hardcoded secrets.
4. **Write `%USERPROFILE%\.config\mcpsync\mcp.json`** with those servers. Use `${NAME}` placeholders for every secret, and pick one clear uppercase name per secret (for example `GITHUB_MCP_PAT`, `DOKPLOY_API_KEY`). Back up the existing file first. Keep the structure from `examples/mcp.json`.
5. **Tell the person which secret names they need to set**, and give them the exact command for each one: `envsync set NAME`. Wait until they confirm each one is done.
6. **Run `mcpsync check`.** Report which names have a ✓ and which a ✗. Repeat step 5 for any missing names.
7. **Preview:** run `mcpsync sync --dry-run` and report only the summary lines (per editor, and the `!` warnings). Explain in plain words what it will add, replace or remove.
8. **Ask for approval to apply,** naming what will change. Only after a yes, run `mcpsync sync` and report the summary lines.
9. **Finish:** tell the person to fully quit and reopen their editors and check that each MCP server connects. Remind them about the backups in `%USERPROFILE%\.config\mcpsync\backups` and that they should be deleted once the tokens are rotated. Don't delete them yourself.

## When something fails

- **A server doesn't connect:** compare its name in the editor's MCP panel with `mcp.json`, and run `mcpsync doctor`. Don't change secrets; ask the person to check the token they stored.
- **An `npx` server crashes with `MODULE_NOT_FOUND`:** its cached install under `%LOCALAPPDATA%\npm-cache\_npx\` is probably broken. Tell the person which folder, and delete it only if they say so.
- **`mcpsync` isn't found:** open a new terminal (the installer changes PATH for new ones), then check `%LOCALAPPDATA%\Programs\win-mcpsync\bin`.
- **An unexpected error:** stop, show the person the error line (without anything that looks like a secret), and ask how they want to proceed.

Finish with a short summary: what was set up, what the person still has to do, and which backups exist.
