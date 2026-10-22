# Smoke-test the MCP HTTP transport's shared-instance guards.
# Usage: pwsh scripts/probe_http_transport.ps1 [-Port 9333] [-Token test-secret-123]
param(
    [int]$Port = 9333,
    [string]$Token = "test-secret-123"
)

$base = "http://127.0.0.1:$Port"
$json = "application/json"

function Probe {
    param([string]$Label, [string]$Path, [string]$Method, [string]$Body, [hashtable]$Headers)
    $args = @{ Uri = "$base$Path"; Method = $Method; ContentType = $json; UseBasicParsing = $true }
    if ($Body) { $args.Body = $Body }
    if ($Headers.Count -gt 0) { $args.Headers = $Headers }
    try {
        $r = Invoke-WebRequest @args
        $preview = $r.Content
        if ($preview.Length -gt 200) { $preview = $preview.Substring(0, 200) }
        Write-Host "$Label -> $($r.StatusCode) $preview"
    }
    catch {
        $code = $_.Exception.Response.StatusCode.value__
        $text = ""
        try {
            $sr = New-Object System.IO.StreamReader($_.Exception.Response.GetResponseStream())
            $text = $sr.ReadToEnd()
            if ($text.Length -gt 200) { $text = $text.Substring(0, 200) }
        }
        catch { }
        Write-Host "$Label -> $code $text"
    }
}

$list = '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
$profiles = '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"webrain_profiles","arguments":{}}}'
$login = '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"webrain_session","arguments":{"op":"login"}}}'
$readonly = '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"webrain_eval","arguments":{"expression":"1+1"}}}'

# The session family. `op=list` is the leak (it enumerates other tenants'
# sessions); cookies/save_state are the ones that hand over credentials. All of
# them fold to executors whose names are NOT in any stateful-tools list, which is
# why the guard matches the umbrella instead.
$sessList = '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"webrain_session","arguments":{"op":"list"}}}'
$sessCookies = '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"webrain_session","arguments":{"op":"cookies","session_id":"s"}}}'
$sessState = '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"webrain_session","arguments":{"op":"save_state","service":"x","profile":"y"}}}'

Probe "1  GET /health (no auth)      " "/health" "GET" $null @{}
Probe "2  tools/list (no auth)      " "/mcp" "POST" $list @{}
Probe "3  tools/list (bad token)    " "/mcp" "POST" $list @{ Authorization = "Bearer wrong-token" }
Probe "4  tools/list (good token)   " "/mcp" "POST" $list @{ Authorization = "Bearer $Token" }
Probe "5  webrain_profiles (NO_STATE) " "/mcp" "POST" $profiles @{ Authorization = "Bearer $Token" }
Probe "6  session(op=login) -> login  " "/mcp" "POST" $login @{ Authorization = "Bearer $Token" }
Probe "7  webrain_eval (stateless ok) " "/mcp" "POST" $readonly @{ Authorization = "Bearer $Token" }
Probe "8  session(op=list)  [the leak]" "/mcp" "POST" $sessList @{ Authorization = "Bearer $Token" }
Probe "9  session(op=cookies)          " "/mcp" "POST" $sessCookies @{ Authorization = "Bearer $Token" }
Probe "10 session(op=save_state)       " "/mcp" "POST" $sessState @{ Authorization = "Bearer $Token" }
