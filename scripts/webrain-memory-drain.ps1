# webrain-memory-drain.ps1 - long-running daemon wrapper around
# `webrain_domain_memory op=drain`.
#
# The MCP tool drains a bounded number of queued jobs per call; this script is
# the supervisor loop: it invokes the CLI, sleeps, and repeats until stopped
# (Ctrl+C) or `-Batches` is exhausted. It is intentionally thin: all storage,
# claiming, fetching, ingesting and completion live in the Rust tools.
#
# Usage:
#   $env:CDP_URL = "http://127.0.0.1:9222"
#   .\scripts\webrain-memory-drain.ps1 -Db "C:\path\web-memory.sqlite" -IntervalSec 30
#
# Stop the loop with Ctrl+C. Use Task Scheduler/systemd for a real service.
param(
    [Parameter(Mandatory = $true)][string]$Db,
    [string]$Webrain = "webrain",
    [int]$IntervalSec = 30,
    [int]$Limit = 5,
    [int]$MaxBatches = 1,
    [int]$Batches = 0  # 0 = run forever
)

$argsJson = @{
    op         = "drain"
    db         = $Db
    limit      = $Limit
    max_batches = $MaxBatches
} | ConvertTo-Json -Compress

$run = 0
while ($true) {
    $run++
    Write-Host "[$(Get-Date -Format o)] drain #$run db=$Db"
    & $Webrain tool webrain_domain_memory $argsJson
    $code = $LASTEXITCODE
    if ($code -ne 0) {
        Write-Warning "drain #$run exited $code; continuing after sleep"
    }
    if ($Batches -gt 0 -and $run -ge $Batches) { break }
    Start-Sleep -Seconds ([Math]::Max(1, $IntervalSec))
}
