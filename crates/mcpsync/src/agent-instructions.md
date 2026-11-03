# mcpsync: instructions for AI agents

mcpsync keeps one MCP server list (the **source**) and writes it into each editor's own config file. Secrets appear in the source only as `${NAME}`; their values live in the Windows user environment, managed by envsync. Run `envsync agent-instructions` for how to handle those.

Run `mcpsync help` for the command list. This page covers what `help` doesn't: the safe order of work, and what each step touches.

## Files

| Path | What it is |
|---|---|
| `~/.config/mcpsync/mcp.json` | The source: `{ "mcpServers": { ... } }`, Claude Code style, JSONC allowed |
| `~/.config/mcpsync/config` | Settings: `EDITORS`, `FALLBACK`, `BACKUPS`, `NPX_WRAP_EDITORS`, `PATH_<EDITOR>` |
| `~/.config/mcpsync/state.json` | Which servers mcpsync wrote to each editor; it removes only those |
| `~/.config/mcpsync/backups` | A copy of each editor file before every change; can hold old plaintext secrets |
| `<project>/.mcpsync.json` | The source for `--project` mode, which writes `.mcp.json`, `.cursor/mcp.json` and friends |

`mcpsync editors` shows where each editor's file lives on this machine.

## Workflow

1. **Look.** Run `mcpsync doctor` and `mcpsync status`. Done when you know which editors are enabled and whether they're in sync.
2. **Edit the source.** Use `mcpsync edit`, or edit `mcp.json` directly after backing it up. Write every secret as `${NAME}`, one clear uppercase name per secret, reused across servers. `mcpsync import <editor>` copies an editor's existing servers in and warns about hardcoded secrets. Done when `mcpsync list` shows every server the person asked for.
3. **Secrets.** Run `mcpsync check`. For each ✗, give the person `envsync set NAME` and wait for them to confirm. Done when `mcpsync check` exits 0.
4. **Preview.** Run `mcpsync sync --dry-run`. Report only the summary lines (starting with ✓, `!`, ✗ or `error:`) and explain in plain words what will be added, replaced or removed. The diff lines can contain config values, so leave them out of your reply.
5. **Apply.** Only after the person approves what you previewed, run `mcpsync sync`. Done when every editor reports ✓.
6. **Restart.** Tell the person to fully quit and reopen each editor, then check that each server connects.

## Exit codes

- `mcpsync check`: 0 when every `${NAME}` in the source is set in the user environment, 1 when any is missing.
- Every other command: 0 on success, 1 on an error (the message starts with `error:`). `sync --dry-run` exits 0 whether or not there are changes; read its lines.

## What to expect

- sync only touches the servers it wrote last time. Servers the person added in an editor by hand stay, and a server removed from the source disappears from every editor on the next sync.
- A `!` line names something that needs the person's attention: a server skipped for one editor (see `FALLBACK` in the settings file), a hand-added server about to be replaced, or comments that will be removed from an editor file.
- Claude Code rewrites `~/.claude.json` while it runs and can undo a sync. If you run inside Claude Code, tell the person to restart it right after syncing, or have them sync with it closed.
- Secrets reach editors through their environment, so an editor opened before `envsync set` doesn't see the value until it's restarted.
- An `npx` server that fails with `MODULE_NOT_FOUND` usually has a broken cached install under `%LOCALAPPDATA%\npm-cache\_npx\<hash>`. Tell the person which folder; delete it only if they say so.
- Backups can hold plaintext secrets from before the migration. Suggest deleting `~/.config/mcpsync/backups` once those keys are rotated, and leave the deleting to the person.
- `mcpsync uninstall` prints a command for the person to paste; it removes nothing itself.

Full reference, including how `${NAME}` is translated per editor: https://github.com/SanderCokart/win-mcpsync/blob/main/docs/mcpsync-reference.md
