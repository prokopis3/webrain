# Merge all task2 crawl outputs into one combined product dataset with source tags + dedupe.
$ErrorActionPreference = 'Stop'
$out = Join-Path $PSScriptRoot '..\output\task2_ALL_PRODUCTS.json'

$all = [System.Collections.Generic.List[object]]::new()
$tableRows = Get-Content (Join-Path $PSScriptRoot '..\output\task2_table.json') -Raw | ConvertFrom-Json
foreach ($t in $tableRows) { $all.Add([pscustomobject]@{ source='table'; url=''; name=$t.Name; price=$t.Price }) }

function Add-FromBatch($path, $source) {
    if (-not (Test-Path $path)) { return }
    $j = Get-Content $path -Raw | ConvertFrom-Json
    if ($j -is [System.Array]) {
        # top-level array (e.g. deduped infinite-scroll list)
        foreach ($p in $j) {
            $all.Add([pscustomobject]@{ source=$source; url=[string]$p.url; name=[string]$p.name; price=[string]$p.price })
        }
    } elseif ($j.results) {
        foreach ($r in $j.results) {
            if ($r.text) {
                try {
                    $arr = $r.text | ConvertFrom-Json
                    foreach ($p in $arr) {
                        $all.Add([pscustomobject]@{ source=$source; url=[string]$p.url; name=[string]$p.name; price=[string]$p.price })
                    }
                } catch {}
            }
        }
    }
}

Add-FromBatch (Join-Path $PSScriptRoot '..\output\task2_ecommerce.json') 'ecommerce'
Add-FromBatch (Join-Path $PSScriptRoot '..\output\task2_pagination.json') 'pagination'
Add-FromBatch (Join-Path $PSScriptRoot '..\output\task2_buttonclick.json') 'button-click'
Add-FromBatch (Join-Path $PSScriptRoot '..\output\task2_infinitescroll_unique.json') 'infinite-scroll'
Add-FromBatch (Join-Path $PSScriptRoot '..\output\task2_jsrendering2.json') 'js-rendering'

$rows = $all | Where-Object { $_.name -ne '' -or $_.url -ne '' }
$bySource = $rows | Group-Object source | ForEach-Object { "$($_.Name)=$($_.Count)" }
"PER-SOURCE: $($bySource -join ', ')"
$uniqByName = $rows | Sort-Object name -Unique
$uniqByUrl = $rows | Where-Object { $_.url } | Sort-Object url -Unique
"UNIQUE_BY_NAME=$(@($uniqByName).Count)"
"UNIQUE_BY_URL=$(@($uniqByUrl).Count)"
$rows | ConvertTo-Json -Depth 3 | Set-Content $out
"merged -> $out"
