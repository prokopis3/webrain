# Port a ~3500-domain tracker blocklist into webrain-core/data/tracker_domains.txt
# Source: anudeepND/blacklist adservers.txt (well-known, clean, ad/tracker domains)
$ErrorActionPreference = 'Stop'
$out = Join-Path $PSScriptRoot '..\webrain-core\data\tracker_domains.txt'
New-Item -ItemType Directory -Force -Path (Split-Path $out) | Out-Null
$url = 'https://raw.githubusercontent.com/anudeepND/blacklist/master/adservers.txt'
$r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 60
$domains = $r.Content -split "`n" | ForEach-Object {
    $_.Trim().ToLowerInvariant()
} | Where-Object {
    $_ -and -not $_.StartsWith('#') -and $_ -ne 'localhost' -and $_ -ne '0.0.0.0' -and $_ -ne '::'
} | ForEach-Object {
    # strip hosts-file prefixes and wildcard stars
    $_ -replace '^(0\.0\.0\.0\s+|::\s+|\*\.)', ''
} | Where-Object {
    $_ -match '^[a-z0-9.-]+\.[a-z]{2,}$'
} | Sort-Object -Unique
$take = $domains | Select-Object -First 3500
Set-Content -Path $out -Value $take -Encoding utf8
"ported $($take.Count) domains -> $out"
