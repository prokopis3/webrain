# Feed & News Aggregation (RSS) + Block Bypass

Collect latest news/headlines across many sources with webrain tools, and get
through the WAF blocks you'll actually hit. Verified live Aug 2026 (BBC,
Guardian, NPR, Al Jazeera, eKathimerini, Google News).

## A. Discovery — find the REAL feed
A site's `/rss/` path is often a WAF-blocked decoy. Find the real feed:
1. `webrain_navigate(homepage)` — must load (see §D if blocked).
2. `webrain_eval` on the loaded page:
   `JSON.stringify([...document.querySelectorAll('link[rel="alternate"]')].map(l=>({type:l.type,href:l.href})))`
   → keep entries whose `type` is rss/atom/xml.
3. Confirm it's the only one: `webrain_crawl(mode=spider, seed=homepage,
   allow=["(?i)rss|feed|atom|\\.xml"], no_content=true, max_pages=8)`.
4. The real feed often lives OFF-domain (FeedBurner/CDN) — that's why it is not
   blocked (eKathimerini: `/rss/` is an absent path that is ALSO WAF-blocked —
   a random nonexistent path 404s normally while `/rss/` gets the Cloudflare
   block page and `/feed/` an nginx 403; real feed `feeds.feedburner.com/ekathimerini/sKip`).

## B. Fetch a feed (choose path)
| Feed type | Path |
|---|---|
| Plain XML, no auth | `webrain_download(engine=http, urls=[feed])` → file → parse locally |
| Many feeds at once | `webrain_batch(op=fetch, urls=[...])` → raw XML in `text`. CSS selectors FAIL on namespaced XML DOM — do NOT use `op=extract` here |
| Non-namespaced feeds (BBC/Guardian/NPR) | `webrain_batch(op=extract, base_selector="item", fields=[{title,date,link}])` → structured `data` |
| Namespaced feeds (Al Jazeera/MEE) | `op=extract` returns `[]` (selector doesn't match) → `op=fetch` + parse XML locally |

`webrain_batch` needs a CDP browser: start one first
(`chrome --remote-debugging-port=9222 --user-data-dir=...`) or it errors
"CDP endpoint not reachable". Navigating a URL first (same session) also warms
it — `navigate` → `batch` is the reliable recipe.

## C. Parse locally (fast, no browser)
```powershell
[xml]$x = Get-Content -Raw "output/news/0_feed"; $x.rss.channel.item | Select-Object -First 25 | ForEach-Object { "{0}  |  {1}" -f $_.pubDate, $_.title }
```
Check item dates — some feeds (Middle East Eye `/rss.xml`) are popularity-ordered
STALE dumps (2016–2018 on top): filter `pubDate -match 'Aug 2026'` before
trusting "latest".

## D. Bypass when a source hard-blocks
Full detail: `references/challenges.md` §5b. Quick version:
- `crippled:true` + "Attention Required!" = HARD block → real HEADED Chrome +
  persistent profile (`--user-data-dir=... --disable-blink-features=AutomationControlled`),
  NOT headless (headless is detectable and triggers the block).
- Still blocked on every path/IP → clean-IP proxy (`proxy` param) or report. Don't loop.
- Blocked AND absent `/rss/` ≠ blocked site: navigate homepage → `eval` the
  `link[rel="alternate"]` → fetch the off-domain feed plain-HTTP → done.
  (Control-test first: a random nonexistent path 404s normally → a block page on
  `/rss/` means a path-specific WAF rule, not a dead site.)

## E. Source list (verified Aug 2026)
| Source | Feed URL |
|---|---|
| BBC World | `https://feeds.bbci.co.uk/news/world/rss.xml` |
| Guardian World | `https://www.theguardian.com/world/rss` |
| NPR World | `https://feeds.npr.org/1004/rss.xml` |
| Al Jazeera | `https://www.aljazeera.com/xml/rss/all.xml` |
| eKathimerini (GR) | `https://feeds.feedburner.com/ekathimerini/sKip` (`/rss/` = absent + WAF-blocked) |
| Google News topic | `https://news.google.com/rss/search?q=<query>%20when:7d&hl=en-US&gl=US&ceid=US:en` |
| Turkey–Greece topic | `https://www.newsnow.com/us/World/Middle+East/Turkey/Greece~Turkey` |
