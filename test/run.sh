#!/usr/bin/env bash
# Smoke tests for mcpsync. Runs entirely in a throwaway HOME; never touches the real config
# or the real Windows user environment.
# Usage: test/run.sh [path/to/mcpsync | path/to/mcpsync.exe]
set -uo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
MS="${1:-$HERE/mcpsync}"
export MS="$(cd "$(dirname "$MS")" && pwd)/$(basename "$MS")"  # absolute: some tests cd elsewhere
PASS=0 FAIL=0

# Runs the binary directly, or the bash script through bash.
ms() { if [[ $MS == *.exe ]]; then "$MS" "$@"; else bash "$MS" "$@"; fi; }
export -f ms

check() { # check "description" command...
  local desc=$1; shift
  if "$@" >/dev/null 2>&1; then PASS=$((PASS + 1)); echo "ok   $desc"
  else FAIL=$((FAIL + 1)); echo "FAIL $desc"; fi
}

# Node/Python helpers. The expression sees the parsed document as `d`.
jsonq() { node -e 'const d=JSON.parse(require("fs").readFileSync(process.argv[1],"utf8")); process.exit(eval(process.argv[2]) ? 0 : 1)' "$(cygpath -m "$1" 2>/dev/null || echo "$1")" "$2"; }
PY="$(command -v python3 || command -v python)"
tomlq() { "$PY" -c 'import sys,tomllib; d=tomllib.load(open(sys.argv[1],"rb")); sys.exit(0 if eval(sys.argv[2]) else 1)' "$(cygpath -m "$1" 2>/dev/null || echo "$1")" "$2"; }

H="$(mktemp -d)"
export HOME="$H"
unset APPDATA CODEX_HOME CLAUDE_CONFIG_DIR PATH_CLAUDE_CODE PATH_CURSOR PATH_CODEX
export MCPSYNC_HOME="$H/.config/mcpsync"
# envsync keeps variables in this file instead of the Windows user environment
export ENVSYNC_TEST_STORE="$H/envsync-store.json"
echo "throwaway HOME: $H"

# --- basic flow
check "init"            ms init --editors "claude-code cursor codex gemini devin"
cp "$HERE/test/fixtures/mcp.json" "$MCPSYNC_HOME/mcp.json"
check "list"            ms list
check "sync --dry-run"  ms sync --dry-run
check "sync"            ms sync
check "claude-code json" jsonq "$H/.claude.json" 'd.mcpServers.context7 && d.mcpServers.Ref'
check "cursor header env syntax" jsonq "$H/.cursor/mcp.json" 'd.mcpServers.context7.headers.CONTEXT7_API_KEY==="${env:CONTEXT7_API_KEY}"'
check "cursor dokploy env syntax" jsonq "$H/.cursor/mcp.json" 'd.mcpServers["dokploy-mcp"].env.DOKPLOY_API_KEY==="${env:DOKPLOY_API_KEY}"'
check "codex toml"      tomlq "$H/.codex/config.toml" 'd["mcp_servers"]["github"]["bearer_token_env_var"]=="GITHUB_MCP_PAT" and d["mcp_servers"]["Ref"]["env_http_headers"]=={"x-ref-api-key":"REF_API_KEY"}'
check "gemini json"     jsonq "$H/.gemini/settings.json" 'd.mcpServers.context7'
check "devin json"      jsonq "$H/AppData/Roaming/devin/mcp_config.json" 'd.mcpServers.context7'
check "status in sync"  bash -c "ms status | grep -q 'in sync'"
check "second sync up to date" bash -c "ms sync 2>&1 | grep -q 'up to date'"

# --- removal and hand-added content survive
node -e 'const f=process.argv[1];const d=JSON.parse(require("fs").readFileSync(f,"utf8"));d.mcpServers.handmade={command:"echo"};require("fs").writeFileSync(f,JSON.stringify(d,null,2))' "$H/.claude.json"
printf '\n[profile.custom]\n# my comment\nmodel = "x"\n' >> "$H/.codex/config.toml"
node -e 'const f=process.argv[1];const fs=require("fs");const d=JSON.parse(fs.readFileSync(f,"utf8"));delete d.mcpServers.Ref;fs.writeFileSync(f,JSON.stringify(d,null,2))' "$MCPSYNC_HOME/mcp.json"
check "remove Ref from source, sync" ms sync
check "removed from claude-code" jsonq "$H/.claude.json" 'd.mcpServers.Ref===undefined'
check "hand-added survives in claude-code" jsonq "$H/.claude.json" 'd.mcpServers.handmade'
check "unrelated toml content and comments survive" bash -c "grep -q '# my comment' '$H/.codex/config.toml' && grep -q 'profile.custom' '$H/.codex/config.toml'"

# --- import round-trips
check "import cursor" ms import cursor
check "import codex"  ms import codex
mkdir -p "$H/.cursor"
printf '{"mcpServers":{"lit":{"url":"https://x.test/mcp","headers":{"Authorization":"Bearer sk-TESTLITERAL"}}}}' > "$H/.cursor/mcp.json"
check "literal-secret warning on import" bash -c "ms import cursor 2>&1 | grep -q 'hardcoded secret'"

# --- project mode
P="$(mktemp -d)"
cat > "$P/.mcpsync.json" <<'JSON'
{ "mcpServers": { "proj": { "command": "npx", "args": ["-y", "x"] }, "onlycursor": { "command": "y", "x-editors": ["cursor"] } } }
JSON
check "--project sync" bash -c "cd '$P' && ms --project sync"
check "project .mcp.json has proj" jsonq "$P/.mcp.json" 'd.mcpServers.proj'
check "x-editors includes cursor" jsonq "$P/.cursor/mcp.json" 'd.mcpServers.onlycursor'
check "x-editors excludes claude-code" bash -c "! grep -q onlycursor '$P/.mcp.json'"

# --- FALLBACK policies (Ref comes back into the source for these)
cp "$HERE/test/fixtures/mcp.json" "$MCPSYNC_HOME/mcp.json"
printf 'EDITORS="codex"\nFALLBACK=resolve\n' > "$MCPSYNC_HOME/config"
check "FALLBACK=resolve keeps codex Ref (no plaintext)" bash -c "ms sync >/dev/null 2>&1; grep -q 'mcp_servers.Ref' '$H/.codex/config.toml' && ! grep -q 'sk-' '$H/.codex/config.toml'"
printf 'EDITORS="codex"\nFALLBACK=keep\n' > "$MCPSYNC_HOME/config"
check "FALLBACK=keep runs" ms sync
printf 'EDITORS="codex"\nFALLBACK=skip\n' > "$MCPSYNC_HOME/config"
check "FALLBACK=skip keeps Ref when it has no URL vars" bash -c "ms sync >/dev/null 2>&1; grep -q 'mcp_servers.Ref' '$H/.codex/config.toml'"

# --- WSL_ env prefix: WSL_NAME overrides NAME on WSL; WSL_ keys never reach other platforms
cat > "$MCPSYNC_HOME/mcp.json" <<'JSON'
{ "mcpServers": { "wslenv-test": { "command": "node", "args": ["x.js"], "env": { "ABC": "from-base", "WSL_ABC": "from-wsl", "WSL_ONLY": "wsl-only" } } } }
JSON
printf 'EDITORS="cursor"\nFALLBACK=skip\n' > "$MCPSYNC_HOME/config"
check "sync with WSL_ keys" ms sync
check "WSL_ keys never leak into output" jsonq "$H/.cursor/mcp.json" "!('WSL_ABC' in d.mcpServers['wslenv-test'].env) && !('WSL_ONLY' in d.mcpServers['wslenv-test'].env)"
if grep -qi microsoft /proc/version 2>/dev/null; then want=from-wsl; else want=from-base; fi
check "WSL_ value applies only on WSL (expects $want here)" jsonq "$H/.cursor/mcp.json" "d.mcpServers['wslenv-test'].env.ABC==='$want'"

# --- Windows drive path in "command": converted to /mnt/c on WSL, kept as-is elsewhere
cat > "$MCPSYNC_HOME/mcp.json" <<'JSON'
{ "mcpServers": { "exe-test": { "command": "C:\\tools\\x.exe", "args": [] } } }
JSON
check "sync with drive-path command" ms sync
if grep -qi microsoft /proc/version 2>/dev/null; then
  check "drive path becomes /mnt/c on WSL" jsonq "$H/.cursor/mcp.json" "d.mcpServers['exe-test'].command==='/mnt/c/tools/x.exe'"
else
  check "drive path kept on this platform" jsonq "$H/.cursor/mcp.json" "d.mcpServers['exe-test'].command==='C:\\\\tools\\\\x.exe'"
fi

# --- doctor
check "doctor runs" ms doctor

# --- Rust build only: envsync, mcpsync check, secret forwarding, uninstall
if [[ $MS == *.exe ]]; then
  export ES="$(dirname "$MS")/envsync.exe"
  es() { "$ES" "$@"; }
  export -f es
  cp "$HERE/test/fixtures/mcp.json" "$MCPSYNC_HOME/mcp.json"
  check "check fails while secrets are missing" bash -c "! ms check"
  check "check names a missing secret"   bash -c "ms check 2>&1 | grep -q 'CONTEXT7_API_KEY — not set'"
  check "secret set forwards to envsync" bash -c "printf 'test-value-1\n' | ms secret set CONTEXT7_API_KEY 2>&1 | grep -q 'envsync'"
  check "envsync has the forwarded secret" es has CONTEXT7_API_KEY
  check "envsync set from stdin"         bash -c "printf 'test-value-2\n' | es set GITHUB_MCP_PAT"
  check "envsync set shares with WSL"    bash -c "es wslenv | grep -qx GITHUB_MCP_PAT"
  check "envsync list hides values"      bash -c "es list | grep -q GITHUB_MCP_PAT && ! es list | grep -q test-value"
  for n in DOKPLOY_API_KEY REF_API_KEY CLOUDREVE_CLIENT_ID CLOUDREVE_CLIENT_SECRET; do printf 'x\n' | es set "$n" 2>/dev/null; done
  check "check passes once all are set"  ms check
  check "no value in any output"         bash -c "! { ms check; ms secret list; ms doctor; es list; } 2>&1 | grep -q test-value"
  check "envsync unset"                  bash -c "es unset GITHUB_MCP_PAT && ! es has GITHUB_MCP_PAT && ! es wslenv | grep -qx GITHUB_MCP_PAT"
  check "envsync rejects a bad name"     bash -c "! printf 'x\n' | es set 1BAD"
  check "envsync uninstall refuses while mcpsync exists" bash -c "! es uninstall"
  check "envsync uninstall --force prints the command" bash -c "es uninstall --force | grep -q 'Remove-Item -Force -LiteralPath'"
  check "mcpsync uninstall prints the command" bash -c "ms uninstall | grep -q 'mcpsync.exe'"
  check "mcpsync agent-instructions"     bash -c "ms agent-instructions | grep -q '^# mcpsync: instructions for AI agents'"
  check "envsync agent-instructions"     bash -c "es --agent-instructions | grep -q '^## Secret names only'"
  check "agent-instructions needs no config" bash -c "MCPSYNC_HOME=/nonexistent ms agent-instructions >/dev/null"
  check "nothing was removed by uninstall" test -f "$ES"
  CANARY="MCPSYNC_TEST_CANARY_$$"
  printf 'x\n' | es set "$CANARY" 2>/dev/null
  check "real user environment untouched" bash -c "es has $CANARY && [[ -z \$(powershell.exe -NoProfile -Command \"[Environment]::GetEnvironmentVariable('$CANARY','User')\" | tr -d '\r') ]]"
fi

rm -rf "$H" "$P"
echo
echo "passed: $PASS  failed: $FAIL"
[[ $FAIL -eq 0 ]]
