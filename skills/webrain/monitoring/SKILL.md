---
name: monitoring
version: "1.0.0"
description: Watch a live page for real change (price, score, order book, listing count, stock, queue depth) with the webrain MCP tools. Gives the self-contained eval tick (one JS pass, ≤25 s, samples + distinct count), the agent-driven repeat-and-diff loop, external cron/systemd for anything slower, re-pinning the target every tick, and the honest limits — no scheduler, no stored history, no change predicate in wait yet. Use when asked to monitor, watch, poll, track changes, follow a live number, or "tell me when X changes". For one-shot extraction use `web-research`; for feeds use `workflows/feed-and-news.md`.
argument-hint: "<what to watch> [from <url>] [every <N>s]"
allowed-tools: Bash, Read, Write
---

# /monitoring

**webrain has no scheduler and keeps no state between calls.** Monitoring here is
an agent loop, and saying anything else is a lie the runtime cannot back. There is
no `monitor` tool, no `schedule` op, no history store — the ADR's HTTP-cache
paragraph ("scheduled recurring monitoring does not pay for a full download") describes
a caller that does not exist in this build. Do not promise one.

Everything below was measured on live sites (CoinDesk, CoinMarketCap) while building
this skill, not inferred.

## The working recipe

1. **One source.** `webrain_eval` with a JS loop of **≤25 s** returning every sample
   and the distinct count — not a single read. One call, one series.
2. **The agent repeats that call on its own interval** and keeps the series, diffing
   consecutive samples.
3. **Longer than 30 s → external cron / `systemd` timer** driving `webrain mcp` or the
   CLI. webrain still remembers nothing between runs; the scheduler owns the history.
4. **Re-`navigate` (or assert `location.href`) at the start of every tick.** That is
   the only defence against the target-binding gap below.

## Step 1 — the tick

One `webrain_eval` whose JS samples internally and returns **all** samples plus the
distinct count. A single read cannot distinguish "unchanged" from "never read".

```js
(async () => {
  const sel = '<selector you verified with webrain_eval first>';
  const stop = Date.now() + 22000;          // stay under the 30 s CDP cap
  const seen = [], vals = [];
  while (Date.now() < stop) {
    const t = (document.querySelector(sel) || {}).textContent || '';
    const v = t.trim();
    if (v && v !== seen[seen.length - 1]) seen.push(v);
    vals.push(v);
    await new Promise(r => setTimeout(r, 2000));
  }
  return JSON.stringify({
    url: location.href,
    samples: vals.length,
    distinct: new Set(seen).size,
    series: seen.slice(0, 50),
    last: seen[seen.length - 1] || null
  });
})()
```

Read `distinct` first. `distinct === 0` means the selector matched nothing (wrong
element or unmounted page) — not "no change". `distinct === 1` over 20 s on a
live price means you sampled a stale node or a hidden duplicate; check the field
against `webrain_observe(what="clean")` before believing it.

**Prices are never read from bare text nodes.** Google renders date-grids of prices
that pollute raw-text scrapes; read the result card / row container instead. The
same rule applies to any grid page.

## Step 2 — repeat and diff

The agent re-issues step 1 each interval (`webrain_eval` again — no new browser, no
new tab, same page). Keep the series in your own reply or in a file; webrain will not.

Threshold on a **noise floor**, measured, not assumed:

| Signal | Measured | Meaning |
|---|---|---|
| BTC/USD on one source, 20 s | `77,021.60 → 77,022.49 → 77,026.68` | cents-to-dollar drift is noise |
| BTC/USD, CoinDesk vs CoinMarketCap, same minute | `77,015.14` vs `77,021.60` (~$6) | two sources disagree by more than one tick |
| Same page, next tick | `77,003.31` (down $20 from the run above) | the value genuinely moves |

So a tolerance below ~0.1 % will fire on noise, and comparing across two sources will
report a "change" that is just the sources disagreeing. Pick one source, one selector,
one tolerance, and write the tolerance down before you start.

## Step 3 — interval > 30 s

```cron
*/5 * * * * cd /path && webrain eval --url https://example.com/x --js-file tick.js >> series.jsonl
```

or a `systemd` timer calling the same CLI. Append one JSON line per run; that file is
the history webrain does not keep. For an MCP-only host, call the MCP server from the
scheduler — same tools, same absence of memory.

## Step 4 — re-pin the target every tick

Measured: right after `webrain_navigate` to CoinMarketCap, a follow-up `webrain_eval`
executed against the **arXiv tab** that was still open. The reply looked like a
monitoring result and was a different page's DOM. Nothing in the tool surface tells you
which target answered.

Defences, in order:

1. Assert the page inside the same eval: `if (!location.href.includes('coinmarketcap')) return JSON.stringify({error:'wrong tab', url: location.href})`.
2. `webrain_observe(what="state")` and compare `url` before trusting the series.
3. `webrain_navigate` at the top of the tick rather than relying on the tab still being there.

Treat a series whose `url` you did not verify as no data at all.

## Known gaps — do not design around them silently

| Gap | Evidence | Workaround |
|---|---|---|
| **`webrain_watch` is video-only** (`url` = video or local path) | tool schema; no generic page watch | poll with `webrain_eval` (step 1) |
| **`webrain_eval` has a ~30 s CDP timeout** | a loop over 30 s returns an error, not partial data | cap at 25 s, return early, repeat |
| **Eval may answer on the wrong target** | arXiv tab answered after navigating to CMC | assert `location.href` in the JS (step 4) |
| **`webrain_wait` has no change predicate** | only `ms` / `selector` / `text` | sample in JS, or the planned `until:"text_changed"` below |
| **No history between calls** | no store, no scheduler | the agent's own series / a JSONL append from cron |
| **`webrain_batch` can report success with zero work** | `status:"ok"` with `ok:0` and `Target.createTarget: Failed to open a new tab` | check `ok`/`errors`, not just `status` |

### The change predicate that does not exist yet

The laziest honest fix inside the tool (not in the agent) is a **predicate on the
existing wait** — no scheduler, no store, no 19th tool:

```
webrain_interact { action: "wait", until: "text_changed",
                   selector: "<node>", previous_value: "<last read>",
                   timeout_ms: 60000 }
→ { status: "ok", changed: true, value: "<new text>" }   or   { changed: false }
```

Until that ships, do it in one `webrain_eval`:

```js
(async () => {
  const sel = '<selector>', prev = '<previous value>';
  const stop = Date.now() + 22000;
  while (Date.now() < stop) {
    const v = ((document.querySelector(sel) || {}).textContent || '').trim();
    if (v && v !== prev) return JSON.stringify({ changed: true, value: v });
    await new Promise(r => setTimeout(r, 1000));
  }
  return JSON.stringify({ changed: false, value: prev });
})()
```

## Protected / authenticated live feeds

Order books, sports feeds and dashboards behind a wall are **real Chrome + persistent
profile + session** (`webrain launch` / `webrain_session(op=login)`), never the
anonymous retry loop. Read `challenge` on every navigate: a challenge page is not a
successful navigation, and a series sampled from it is not data.

## Inventory mode (links, images, video)

When "monitoring" really means "watch what the page links to / serves", use
`webrain_observe(what="assets")` — one pass returning:

| Group | What it catches that `navigate`'s `links` does not |
|---|---|
| `links.internal` / `.external` | **every** `<a>`/`<area>`: cross-site too, grouped by whether the hostname is the same site; `kind` = page/pdf/image/video/stream/audio/doc/archive |
| `links.other_schemes` | `mailto:`, `tel:`, `ftp:`, … counted by scheme instead of silently dropped |
| `images` | `<img>` + `srcset` + lazy `data-src` + `<picture>` + icons/`og:image` **and CSS `background-image`** |
| `videos` | `<video>`/`<audio>` src + `poster` + `<source>`, known embeds (YouTube/Vimeo/Twitch/…), direct `.mp4`/`.m3u8` links, and HLS/DASH the page actually requested |

`counts` are the real deduped totals; the arrays are capped by `max` (default 300).
Use `only=links|images|videos` when you want one group.

For a whole site in one call:

```
webrain_batch { op: "assets", urls: [...], max: 20, concurrency: 4 }
```

`max` defaults to **20** in batch on purpose: counts plus a sample. Measured on 20
SERP-discovered news sites (real Chrome, concurrency 4): **20/20 pages, 0 errors,
35.9 s wall**, and one page carried **116 CSS background images** while `<img>` alone
showed 98 — the backgrounds are not a rounding error.

### Three things that test taught, and that a fresh run must check

1. **`counts.internal: 0` is usually an empty shell, not an empty page.** Six of the
   twenty returned a blank title with `scanned < 100` (one had `scanned: 0`). A page
   that never mounted has no links to find. Read `title` and `scanned` before
   concluding anything: `title === ''` ⇒ the DOM you scanned is not the site.
2. **Dedupe before the cap.** Counting inside the cap guard makes every site report
   exactly the limit — 14 of 20 pages "had" exactly 6 internal links. Count into a
   seen-set, then push into the capped array.
3. **A long-lived Chrome can refuse new tabs.** `Target.createTarget: Failed to open
   a new tab` for all 20 URLs, `PUT /json/new` → HTTP 500, while `/json/version` and
   `/json/list` answered fine. The batch still reported `status:"ok"` with `ok:0`.
   Start a fresh engine on another port (`--remote-debugging-port=9333
   --user-data-dir=<fresh>`) — do not read `status` and assume it worked.

**YouTube watch pages report no `videos`.** The media is `blob:` URLs served from
`googlevideo.com` behind MSE, so there is no http URL in the DOM to collect. Use
`webrain_observe(what="media", url=...)` — that captures the network requests
instead. `assets` finds *embeds and files*, `media` finds *streams*.

## Reporting

State the source URL, the selector, the sampling window, the tolerance, and the tick
timestamps. A change claim without those five is unfalsifiable, and the first four
are exactly what a later run needs to reproduce it.
