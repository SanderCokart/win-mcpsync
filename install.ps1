<#
Installs mcpsync and envsync (win-mcpsync) for the current Windows user. No admin rights.

  irm https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.ps1 | iex

envsync only, or a pinned version:

  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.ps1))) -EnvSync
  & ([scriptblock]::Create((irm https://raw.githubusercontent.com/SanderCokart/win-mcpsync/main/install.ps1))) -Version 1.0.0

With `| iex`, set $env:WIN_MCPSYNC_APP = 'envsync' or $env:WIN_MCPSYNC_VERSION = '1.0.0' first.
Installing mcpsync also installs envsync, which it needs. Installing envsync never installs mcpsync.
Running it again upgrades in place.
#>
param(
    [switch] $EnvSync,
    [string] $Version = $env:WIN_MCPSYNC_VERSION
)

& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'  # Invoke-WebRequest is very slow with the progress bar in 5.1

    $repo = 'SanderCokart/win-mcpsync'
    $target = 'x86_64-pc-windows-msvc'

    $app = if ($EnvSync -or $env:WIN_MCPSYNC_APP -eq 'envsync') { 'envsync' } else { 'mcpsync' }
    $apps = if ($app -eq 'mcpsync') { @('envsync', 'mcpsync') } else { @('envsync') }

    if (-not ($env:PROCESSOR_ARCHITECTURE -in @('AMD64', 'ARM64'))) {
        throw "win-mcpsync needs 64-bit Windows (found $env:PROCESSOR_ARCHITECTURE)."
    }

    $bin = if ($env:WIN_MCPSYNC_INSTALL_DIR) { $env:WIN_MCPSYNC_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'Programs\win-mcpsync\bin' }
    $base = if ($Version) { "https://github.com/$repo/releases/download/v$($Version.TrimStart('v'))" } else { "https://github.com/$repo/releases/latest/download" }

    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

    $tmp = Join-Path ([IO.Path]::GetTempPath()) ("win-mcpsync-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Force -Path $tmp | Out-Null
    try {
        # WIN_MCPSYNC_DIST: a local folder with release assets, for testing an unpublished build.
        function Get-Asset([string] $name) {
            $out = Join-Path $tmp $name
            if ($env:WIN_MCPSYNC_DIST) {
                Copy-Item -LiteralPath (Join-Path $env:WIN_MCPSYNC_DIST $name) -Destination $out
            } else {
                Invoke-WebRequest -UseBasicParsing -Uri "$base/$name" -OutFile $out
            }
            $out
        }

        $sums = @{}
        foreach ($line in Get-Content -LiteralPath (Get-Asset 'SHA256SUMS')) {
            $parts = $line.Trim() -split '\s+', 2
            if ($parts.Count -eq 2) { $sums[$parts[1].TrimStart('*')] = $parts[0].ToLowerInvariant() }
        }

        # Verify every download before touching the install folder.
        foreach ($a in $apps) {
            $zipName = "$a-$target.zip"
            $zip = Get-Asset $zipName
            $want = $sums[$zipName]
            $got = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
            if (-not $want) { throw "SHA256SUMS has no entry for $zipName; nothing was installed" }
            if ($got -ne $want) { throw "checksum mismatch for $zipName (expected $want, got $got); nothing was installed" }
            Expand-Archive -LiteralPath $zip -DestinationPath (Join-Path $tmp $a) -Force
        }

        New-Item -ItemType Directory -Force -Path $bin | Out-Null
        foreach ($a in $apps) {
            $unpacked = Join-Path $tmp $a
            $dest = Join-Path $bin "$a.exe"
            # A running .exe can be renamed but not overwritten, so move the old one aside first.
            if (Test-Path -LiteralPath $dest) { Move-Item -Force -LiteralPath $dest -Destination "$dest.old" }
            Copy-Item -LiteralPath (Join-Path $unpacked "$a.exe") -Destination $dest
            Remove-Item -Force -LiteralPath "$dest.old" -ErrorAction SilentlyContinue
            Write-Host "installed $(& $dest --version) -> $dest"
        }
    } finally {
        Remove-Item -Recurse -Force -LiteralPath $tmp -ErrorAction SilentlyContinue
    }

    $pathChanged = $false
    if (-not $env:WIN_MCPSYNC_NO_PATH) {
        $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
        try {
            $raw = [string] $key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
            $kind = if ($raw) { $key.GetValueKind('Path') } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
            $entries = @($raw -split ';' | Where-Object { $_ })
            if (-not ($entries | Where-Object { $_.TrimEnd('\') -eq $bin.TrimEnd('\') })) {
                $key.SetValue('Path', (($entries + $bin) -join ';'), $kind)
                $pathChanged = $true
            }
        } finally {
            $key.Close()
        }
        if ($pathChanged) {
            # Setting and clearing a variable through .NET broadcasts the change to Explorer.
            [Environment]::SetEnvironmentVariable('WIN_MCPSYNC_REFRESH', '1', 'User')
            [Environment]::SetEnvironmentVariable('WIN_MCPSYNC_REFRESH', $null, 'User')
        }
        if (-not (($env:Path -split ';') | Where-Object { $_.TrimEnd('\') -eq $bin.TrimEnd('\') })) {
            $env:Path = "$bin;$env:Path"
        }
    }

    Write-Host ''
    if ($pathChanged) {
        Write-Host "Added $bin to your user PATH. This window already has it; open a new terminal everywhere else."
    }
    if ($app -eq 'mcpsync') {
        Write-Host 'Next: mcpsync init, then envsync set NAME for each secret, then mcpsync check and mcpsync sync.'
    } else {
        Write-Host 'Next: envsync set NAME stores a secret in your Windows user environment and shares it with WSL.'
    }
}
