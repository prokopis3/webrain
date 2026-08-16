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
# profile so the binary still lands in an absolute location.
$local = if ($env:LOCALAPPDATA) { $env:LOCALAPPDATA } else { Join-Path $env:USERPROFILE 'AppData\Local' }
$InstallDir = Join-Path $local 'Programs\webrain'
$exe = Join-Path $InstallDir 'webrain.exe'

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$url = "https://github.com/$Repo/releases/latest/download/webrain-windows.exe"
Write-Host "webrain: downloading $url"
# Download to a temp file, sanity-check it, then atomically move into place so
# an interrupted download (or a running webrain.exe lock) can't leave a
# truncated/corrupt binary at the final path.
$tmp = Join-Path $InstallDir (".webrain.tmp." + [System.Guid]::NewGuid().ToString('N'))
Invoke-WebRequest $url -OutFile $tmp -UseBasicParsing
if ((Get-Item $tmp).Length -eq 0) {
    Remove-Item -Force $tmp -ErrorAction SilentlyContinue
    throw "webrain: downloaded file is empty — possible truncated/HTML error page."
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
Move-Item -Force $tmp $exe

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
