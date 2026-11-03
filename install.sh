#!/bin/sh
# Installs mcpsync and envsync (win-mcpsync) for the current Windows user. No admin rights.
# POSIX sh: works from Git Bash, MSYS2 and any sh on Windows.
#
#   curl -fsSL https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.sh | sh
#   curl -fsSL https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.sh | sh -s -- --envsync
#   curl -fsSL https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.sh | sh -s -- --version 1.0.0
#
# Installing mcpsync also installs envsync, which it needs. Installing envsync never installs
# mcpsync. Running it again upgrades in place.
set -eu

REPO="SanderCokart/win-mcpsync"
TARGET="x86_64-pc-windows-msvc"
APP="${WIN_MCPSYNC_APP:-mcpsync}"
VERSION="${WIN_MCPSYNC_VERSION:-}"

die() { printf 'error: %s\n' "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
Usage: install.sh [--envsync] [--version X.Y.Z]

  --envsync        install only envsync (default: mcpsync, which also installs envsync)
  --version X.Y.Z  install that release instead of the latest
EOF
}

while [ $# -gt 0 ]; do
  case $1 in
    --envsync) APP=envsync ;;
    --version) [ $# -ge 2 ] || die "--version needs a value"; VERSION=$2; shift ;;
    --version=*) VERSION=${1#*=} ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown option: $1 (see --help)" ;;
  esac
  shift
done
case $APP in mcpsync|envsync) ;; *) die "WIN_MCPSYNC_APP must be mcpsync or envsync" ;; esac

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*) ;;
  Linux)
    if grep -qi microsoft /proc/version 2>/dev/null; then
      die "this installs the Windows tools; run it from Git Bash or PowerShell on Windows, not inside WSL"
    fi
    die "win-mcpsync only supports Windows" ;;
  *) die "win-mcpsync only supports Windows" ;;
esac
case "$(uname -m)" in
  x86_64|amd64|aarch64|arm64) ;;
  *) die "win-mcpsync needs 64-bit Windows (found $(uname -m))" ;;
esac
command -v cygpath >/dev/null 2>&1 || die "cygpath not found; run this from Git Bash"

if [ -n "${WIN_MCPSYNC_INSTALL_DIR:-}" ]; then
  BIN="$(cygpath -u "$WIN_MCPSYNC_INSTALL_DIR")"
else
  [ -n "${LOCALAPPDATA:-}" ] || die "LOCALAPPDATA is not set"
  BIN="$(cygpath -u "$LOCALAPPDATA")/Programs/win-mcpsync/bin"
fi
BIN_WIN="$(cygpath -w "$BIN")"

if [ -n "$VERSION" ]; then
  BASE="https://github.com/$REPO/releases/download/v${VERSION#v}"
else
  BASE="https://github.com/$REPO/releases/latest/download"
fi

if [ "$APP" = mcpsync ]; then APPS="envsync mcpsync"; else APPS="envsync"; fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT INT TERM

# WIN_MCPSYNC_DIST: a local folder with release assets, for testing an unpublished build.
fetch() {
  if [ -n "${WIN_MCPSYNC_DIST:-}" ]; then
    cp "$(cygpath -u "$WIN_MCPSYNC_DIST")/$1" "$TMP/$1"
  elif command -v curl >/dev/null 2>&1; then
    curl -fsSL -o "$TMP/$1" "$BASE/$1"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$TMP/$1" "$BASE/$1"
  else
    die "curl or wget is required"
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else die "sha256sum or shasum is required"
  fi
}

unpack() { # unpack <zip> <dir>
  if command -v unzip >/dev/null 2>&1; then
    unzip -oq "$1" -d "$2"
  else
    powershell.exe -NoProfile -NonInteractive -Command \
      "Expand-Archive -Force -LiteralPath '$(cygpath -w "$1")' -DestinationPath '$(cygpath -w "$2")'"
  fi
}

fetch SHA256SUMS
# Verify every download before touching the install folder.
for a in $APPS; do
  zip="$a-$TARGET.zip"
  fetch "$zip"
  want="$(tr -d '\r' < "$TMP/SHA256SUMS" | awk -v f="$zip" '{ n = $2; sub(/^\*/, "", n); if (n == f) print tolower($1) }')"
  [ -n "$want" ] || die "SHA256SUMS has no entry for $zip; nothing was installed"
  got="$(sha256 "$TMP/$zip")"
  [ "$got" = "$want" ] || die "checksum mismatch for $zip (expected $want, got $got); nothing was installed"
  mkdir -p "$TMP/$a"
  unpack "$TMP/$zip" "$TMP/$a"
done

mkdir -p "$BIN"
for a in $APPS; do
  dest="$BIN/$a.exe"
  # A running .exe can be renamed but not overwritten, so move the old one aside first.
  if [ -e "$dest" ]; then mv -f "$dest" "$dest.old"; fi
  cp "$TMP/$a/$a.exe" "$dest"
  rm -f "$dest.old" 2>/dev/null || true
  printf 'installed %s -> %s\n' "$("$dest" --version)" "$(cygpath -w "$dest")"
done

# Adds BIN to the Windows user PATH (raw registry value, so %VARS% in it are kept).
PS_ADD_PATH='
$bin = $env:WIN_MCPSYNC_BIN
$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey("Environment", $true)
$raw = [string] $key.GetValue("Path", "", [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
$kind = if ($raw) { $key.GetValueKind("Path") } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
$entries = @($raw -split ";" | Where-Object { $_ })
if ($entries | Where-Object { $_.TrimEnd("\") -eq $bin.TrimEnd("\") }) { "present" } else {
  $key.SetValue("Path", (($entries + $bin) -join ";"), $kind)
  [Environment]::SetEnvironmentVariable("WIN_MCPSYNC_REFRESH", "1", "User")
  [Environment]::SetEnvironmentVariable("WIN_MCPSYNC_REFRESH", $null, "User")
  "added"
}
$key.Close()
'

echo
if [ -z "${WIN_MCPSYNC_NO_PATH:-}" ]; then
  result="$(WIN_MCPSYNC_BIN="$BIN_WIN" powershell.exe -NoProfile -NonInteractive -Command "$PS_ADD_PATH" | tr -d '\r')"
  if [ "$result" = added ]; then
    echo "Added $BIN_WIN to your user PATH. Open a new terminal to use it, or run this in the current one:"
    echo "  export PATH=\"$BIN:\$PATH\""
  fi
fi
if [ "$APP" = mcpsync ]; then
  echo "Next: mcpsync init, then envsync set NAME for each secret, then mcpsync check and mcpsync sync."
else
  echo "Next: envsync set NAME stores a secret in your Windows user environment and shares it with WSL."
fi
