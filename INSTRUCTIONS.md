# Setup instructions (Windows)

Time: about 10 minutes. Have your MCP server tokens ready.

## 1. Install

In PowerShell:

```powershell
irm https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.ps1 | iex
```

Or in Git Bash:

```bash
curl -fsSL https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.sh | sh
```

This installs `mcpsync` and `envsync` for your user only and adds them to your PATH. Open a new terminal afterwards and check:

```powershell
mcpsync --version
envsync --version
```

## 2. Create the settings and the server list

```powershell
mcpsync init
```

It asks which editors you use (detected ones default to yes) and creates `%USERPROFILE%\.config\mcpsync\mcp.json`. Re-running it is safe: existing files are kept. To start from the template instead of an empty list, copy `examples\mcp.json` over it, or import what an editor already has:

```powershell
mcpsync import cursor
```

## 3. Write your server list

```powershell
mcpsync edit
```

It uses the standard `mcpServers` layout. Use `${NAME}` for every secret:

```json
"Authorization": "Bearer ${GITHUB_MCP_PAT}"
```

Pick names you'll recognise. Use one name per secret, and reuse it across servers.

## 4. Store your secrets

Run one command per secret. You type each value at a hidden prompt; it's stored as a Windows user environment variable and shared with WSL:

```powershell
envsync set GITHUB_MCP_PAT
envsync set DOKPLOY_API_KEY
```

## 5. Check

```powershell
mcpsync check
```

Every name should have a ✓. Anything marked ✗ still needs `envsync set NAME`.

## 6. Preview and apply

```powershell
mcpsync sync --dry-run    # shows what will change, per editor
mcpsync sync              # writes it
```

The diff from `--dry-run` can show current values, so check it on your own screen only.

## 7. Restart the editors

Fully quit Cursor and Claude Code (check the system tray), then open them again. Each MCP server should now show as connected.

If a server doesn't connect, check its name in the editor's MCP panel first, then run `mcpsync list` to confirm the source matches. `mcpsync doctor` checks Node, settings, variables and editors in one go.

If an `npx` server fails to start with `MODULE_NOT_FOUND`, its cached install may be broken. Delete that package's folder under `%LOCALAPPDATA%\npm-cache\_npx\` and restart the editor.

## 8. Clean up

When your tokens are rotated (recommended if they were ever pasted into a chat or shared), delete the backups:

```powershell
Remove-Item -Recurse -Force "$env:USERPROFILE\.config\mcpsync\backups"
```

## Changing things later

- Edit the list with `mcpsync edit`, then run `mcpsync sync --dry-run` and `mcpsync sync`.
- Change a secret: run `envsync set NAME` again.
- Remove a server: delete it from the list and sync. mcpsync removes what it wrote. Servers you added by hand in an editor are left alone, so remove those in the editor.
- Upgrade: run the install command again.
- Uninstall: `mcpsync uninstall`, then `envsync uninstall`. Each prints a command to paste.
