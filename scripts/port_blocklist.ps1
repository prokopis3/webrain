# Port a ~3500-domain tracker blocklist into webrain-core/data/tracker_domains.txt
# Source: anudeepND/blacklist adservers.txt (well-known, clean, ad/tracker domains)
$ErrorActionPreference = 'Stop'
$out = Join-Path $PSScriptRoot '..\webrain-core\data\tracker_domains.txt'
New-Item -ItemType Directory -Force -Path (Split-Path $out) | Out-Null
$url = 'https://raw.githubusercontent.com/anudeepND/blacklist/master/adservers.txt'
# TLS 1.2 (PowerShell 5.1/.NET may not negotiate it by default) + retry/backoff.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$r = $null
for ($i = 0; $i -lt 3 -and -not $r; $i++) {
    try { $r = Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 60 }
    catch { if ($i -eq 2) { throw }; Start-Sleep -Seconds (5 * ($i + 1)) }
}
$domains = $r.Content -split "`n" | ForEach-Object {
    $_.Trim().ToLowerInvariant()
} | Where-Object {
    $_ -and -not $_.StartsWith('#') -and $_ -ne 'localhost' -and $_ -ne '0.0.0.0' -and $_ -ne '::'
} | ForEach-Object {
    # strip hosts-file prefixes and wildcard stars
    $_ -replace '^(0\.0\.0\.0\s+|::\s+|\*\.)', ''
} | Where-Object {
    # RFC 1123 hostname: >=2 labels, each label valid (no leading/trailing
    # hyphen, no consecutive dots) and <=63 chars — malformed entries would
    # never match a real host in cdp.rs's exact-match include_str! lookups.
    $labels = $_.Split('.')
    $labels.Count -ge 2 -and ($labels | Where-Object { $_ -notmatch '^[a-z0-9]([a-z0-9-]*[a-z0-9])?$' -or $_.Length -gt 63 }).Count -eq 0
} | Sort-Object -Unique
$take = $domains | Select-Object -First 3500
# Sanity guard: a transiently empty/404/HTML response must not clobber the
# production blocklist (the previous good list would be unrecoverable).
if ($take.Count -lt 1000) { throw "Parsed only $($take.Count) domains; aborting to avoid truncating tracker_domains.txt" }
# BOM-less UTF-8: PowerShell 5.1's Set-Content -Encoding utf8 writes a BOM that
# Rust include_str! does not strip — the first domain would never exact-match.
[System.IO.File]::WriteAllLines($out, [string[]]$take, (New-Object System.Text.UTF8Encoding($false)))
"ported $($take.Count) domains -> $out"
