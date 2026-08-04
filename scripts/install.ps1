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
$InstallDir = Join-Path $env:LOCALAPPDATA 'Programs\webrain'
$exe = Join-Path $InstallDir 'webrain.exe'

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$url = "https://github.com/$Repo/releases/latest/download/webrain-windows.exe"
Write-Host "webrain: downloading $url"
Invoke-WebRequest $url -OutFile $exe -UseBasicParsing

$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$InstallDir*") {
    [Environment]::SetEnvironmentVariable('Path', "$userPath;$InstallDir", 'User')
    Write-Host "webrain: added $InstallDir to your user PATH (open a new terminal)."
}

Write-Host "webrain: installed. Next steps:"
Write-Host "  webrain install         # download Chrome for Testing (first run)"
Write-Host "  webrain doctor          # verify the install"
Write-Host "  webrain mcp --http 9223 # start the MCP server"
