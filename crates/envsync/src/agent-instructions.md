# envsync: instructions for AI agents

envsync stores secrets as Windows user environment variables (HKCU\Environment) and lists each one in the user `WSLENV` with `/u`, so PowerShell, Git Bash, WSL and GUI apps all read the same value. It knows nothing about MCP; mcpsync uses it for that.

Run `envsync help` for the command list. This page covers what `help` doesn't: how to handle secrets, and what to expect.

## Secret names only

You work with secret **names**. The person types every **value** themselves.

- To store or change a secret, give the person the exact command, `envsync set NAME`, and wait for them to confirm it's done. It opens a hidden prompt.
- Ask for the name and where the secret comes from, never the value. If the person pastes a value into the chat anyway, tell them to rotate it.
- Keep values out of command lines, scripts, files and logs. `envsync set` also reads one line from stdin, but that's for the person's own scripts: a value an agent pipes in lands in the transcript and the shell history.
- Read-only commands are yours to run: `envsync has NAME`, `envsync list`, `envsync wslenv`. They print names only.

## Exit codes

- `envsync has NAME`: 0 if NAME is set in the user environment, 1 if not.
- Every other command: 0 on success, 1 on an error (the message starts with `error:`).

## What to expect

- A new or changed value only reaches programs started afterwards. The person reopens terminals and editors; already-running ones keep the old environment, including the shell you are running in.
- WSL picks up changes after `wsl --shutdown` and reopening WSL.
- Names are case-insensitive, as everywhere on Windows, and must match `[A-Za-z_][A-Za-z0-9_]*`.
- `envsync unset NAME` removes the variable and its `WSLENV` entry. Ask the person before you remove anything.
- `envsync uninstall` prints a command for the person to paste; it removes nothing itself. It refuses while mcpsync is installed, because mcpsync needs envsync.
- `ENVSYNC_TEST_STORE=<file>` makes envsync use a JSON file instead of the user environment. It exists for tests only and stores values in plain text, so leave it unset on a real machine.
