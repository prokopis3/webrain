<#
.SYNOPSIS
    Install webrain on Windows.
.DESCRIPTION
    Downloads the latest webrain release binary and installs it to
    %LOCALAPPDATA%\Programs\webrain, adding that dir to the user PATH.

    One-liner:
      irm https://raw.githubusercontent.com/prokopis3/webrain/main/scripts/install.ps1 | iex
#>
$ErrorActionPreference = 'Stop'
$Repo = 'prokopis3/webrain'
# LOCALAPPDATA can be unset in SYSTEM/service contexts — fall back to the user
# profile, then to SystemDrive if BOTH are unset (Join-Path of $null yields a
# RELATIVE path that would land in the process CWD).
$local = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { Join-Path $env:USERPROFILE 'AppData\Local' }
if (-not $local -or -not [System.IO.Path]::IsPathRooted($local)) {
    $local = Join-Path $env:SystemDrive 'webrain'
}
$InstallDir = Join-Path $local 'Programs\webrain'
$exe = Join-Path $InstallDir 'webrain.exe'

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$url = "https://github.com/$Repo/releases/latest/download/webrain-windows.exe"
Write-Host "webrain: downloading $url"
# Download to a temp file, sanity-check it, then atomically move into place so
# an interrupted download (or a running webrain.exe lock) can't leave a
# truncated/corrupt binary at the final path.
$tmp = Join-Path $InstallDir (".webrain.tmp." + [System.Guid]::NewGuid().ToString('N'))
Invoke-WebRequest $url -OutFile $tmp -UseBasicParsing -TimeoutSec 60
if ((Get-Item $tmp).Length -eq 0) {
    Remove-Item -Force $tmp -ErrorAction SilentlyContinue
    throw "webrain: downloaded file is empty — possible truncated/HTML error page."
}
# Size alone doesn't catch a non-empty HTML error body returned with HTTP 200 —
# verify the file is a real Windows PE executable ("MZ" header) so a skipped
# checksum can't move a non-binary over the working webrain.exe.
$head = Get-Content $tmp -Encoding Byte -TotalCount 2
if ($head[0] -ne 0x4D -or $head[1] -ne 0x5A) {
    Remove-Item -Force $tmp -ErrorAction SilentlyContinue
    throw "webrain: downloaded file is not a Windows PE executable — possible HTML error page."
}
# Verify against the release's published checksums.txt (the release workflow
# publishes it): a tampered release/mirror/MITM fails the check instead of
# silently installing arbitrary code that runs on the next launch.
$sumUrl = "https://github.com/$Repo/releases/latest/download/checksums.txt"
try {
    $sum = Invoke-WebRequest -Uri $sumUrl -UseBasicParsing -TimeoutSec 30
    $expected = (($sum.Content -split "`r?`n" | Where-Object { $_ -match 'webrain-windows\.exe' } | Select-Object -First 1) -split '\s+') | Select-Object -First 1
    if ($expected) {
        $actual = (Get-FileHash -Algorithm SHA256 $tmp).Hash.ToLowerInvariant()
        if ($actual -ne $expected.ToLowerInvariant()) {
            Remove-Item -Force $tmp -ErrorAction SilentlyContinue
            throw "webrain: SHA-256 mismatch — aborting (tampered download?)."
        }
        Write-Host "webrain: SHA-256 verified."
    } else {
        Write-Host "webrain: checksums.txt has no entry for webrain-windows.exe — skipping verification." -ForegroundColor Yellow
    }
} catch {
    Write-Host "webrain: could not fetch checksums.txt — skipping verification." -ForegroundColor Yellow
}
try {
    Move-Item -Force $tmp $exe
} finally {
    # A running webrain.exe (sharing violation) or any failure leaves $tmp —
    # clean it up so the install dir doesn't accumulate .webrain.tmp.* files.
    Remove-Item -Force $tmp -ErrorAction SilentlyContinue
}

# Exact PATH segment matching (no substring wildcard that matches a sibling
# dir) and no empty leading segment (which some shells read as the CWD).
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
$norm = $InstallDir.TrimEnd('\')
$segments = @($userPath -split ';' | Where-Object { $_ -ne '' } | ForEach-Object { $_.TrimEnd('\') })
if ($segments -notcontains $norm) {
    $newPath = (($segments + $norm) | Select-Object -Unique) -join ';'
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    Write-Host "webrain: added $InstallDir to your user PATH (open a new terminal)."
}

Write-Host "webrain: installed. Next steps:"
Write-Host "  webrain install         # download Chrome for Testing (first run)"
Write-Host "  webrain doctor          # verify the install"
Write-Host "  webrain mcp --http 9223 # start the MCP server"
