use webrain_core::backends::cdp::CdpBackend;
use webrain_core::browser::BrowserBackend;

// Launched Chrome sessions held alive across tool calls (MCP server lifetime),
// keyed by "service:profile" so webrain_login/agents can re-attach.
static LAUNCHED: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, webrain_core::launch::Launched>>,
> = std::sync::OnceLock::new();

/// Keep a launched Chrome alive by moving it into the registry.
pub fn store_launched(key: &str, l: webrain_core::launch::Launched) {
    if let Ok(mut m) = LAUNCHED
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
    {
        m.insert(key.to_string(), l);
    }
}

/// Stop a launched Chrome (dropping it kills the child). Returns whether it existed.
pub fn close_launched(key: &str) -> bool {
    let mut m = LAUNCHED
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    m.remove(key).is_some()
}
use serde_json::{Value, json};
use webrain_core::engines::{
    BatchResult, CrawlStrategy, SpiderEngine, TileEngine, batch_eval, batch_extract, batch_fetch,
    batch_interact, batch_markdown, batch_read, batch_scrape, batch_screenshot, bm25_filter,
    build_adaptive_extract_js, build_clean_js, build_extract_js, download_files, regex_extract,
    sitemap_urls, validate_urls,
};
use webrain_core::vision::{ask_viewport, index_current_page, retrieve as vision_retrieve};

/// Render the AX tree as compact `role "name"` lines for LLM reading.
/// ponytail: flat walk, capped 200 nodes, no hierarchy (a11y gives JSON for depth).
fn semantic_tree_text(nodes: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(arr) = nodes.as_array() {
        for n in arr.iter().take(200) {
            let role = n
                .get("role")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let name = n
                .get("name")
                .and_then(|r| r.get("value"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !role.is_empty() || !name.is_empty() {
                lines.push(format!("{role} \"{name}\""));
            }
        }
    }
    lines.join("\n")
}

/// All MCP tools exposed by webrain.
/// Portable agent guide — ships inside the binary so ANY LLM connected via MCP
/// can fetch it (webrain_guide). Mirrors docs/AGENT_DECISION_GUIDE.md.
pub const AGENT_GUIDE: &str = r#"webrain — agent decision guide (call this first when unsure how to proceed)

20 TOOLS — match the request to a boundary:
  webrain_navigate   go to a URL (THE entry point; read `challenge` every time)
  webrain_observe    read the CURRENT page (what: state|a11y|semantic|html|images|
                     assets|console|flatten|fit|clean|screenshot|pixel|page_info|
                     annotate|media) — assets = every link internal+external+
                     non-http scheme, images incl. CSS background-image, video
                     sources+embeds, in one pass
  webrain_network    the page's OWN HTTP traffic (op: log|requests|request|clear) —
                     log captures every request the app makes; request <id> returns
                     that response body, which is how a JS app with no visible data
                     gets scraped
  webrain_interact   drive the page (action: click|click_coords|type|press|scroll|
                     nav|tab|select|hover|check|dialog|wait|upload|dismiss_overlays|
                     add_init_script)
  webrain_extract    structured data from the page (mode: schema|regex|jsonld|table|
                     autoschema|bm25)
  webrain_scrape     fast no-browser GET of one URL (static pages only)
  webrain_batch      same op across many URLs in parallel tabs (op: fetch|markdown|
                     extract|scrape|read|interact|eval|assets|screenshot) + cdp_urls
                     per-proxy fan-out. scrape = the CSS schema over plain HTTP,
                     no tab; read = `webrain read` for N URLs; extract = same
                     schema in a browser tab
  webrain_crawl      site traversal (mode: spider|sitemap|scan|validate).
                     spider: the frontier dedupes on path + sorted query (anchor
                     ignored, `#/` route kept) so facet permutations are ONE
                     page; `delay_ms` is per HOST and is a gap between starts
                     (500 = 2 req/s each); `dedupe_pages` is OFF by default —
                     set true to collapse pages with identical visible text and
                     read `duplicates`, but leave it false when every facet or
                     pagination row must come back.
  webrain_search     web search (duckduckgo|google|bing|brave)
  webrain_serp       STRUCTURED SERP JSON (position/title/url/domain/snippet) — any
                     engine; duckduckgo|bing|auto are pure HTTP (no browser),
                     google|brave render in the connected CDP engine (auto-launched
                     guest Chrome); relevance-filtered + deduped + paginated + falls back
  webrain_pdf        pdf work (op: page|extract|render|images) — extract takes a
                     path or http(s) URL (paths[] batches); render = PNG tiles and
                     needs a --features pdfium build + pdfium.dll/libpdfium.so on PATH
  webrain_download   files/media (engine: http|ytdlp)
  webrain_watch      video → transcript + frames
  webrain_session    browser/auth/session state (op: open|close|list|cookies|
                     setcookies|save_state|restore_state|profiles|login|close_launch)
  webrain_vision     screenshot-tile vision index (op: index|retrieve)
  webrain_eval       arbitrary JS (the escape hatch)
  webrain_eval_in_frame  run JS inside a cross-origin iframe (isolated world) →
                     exact reCAPTCHA/hCaptcha grid + verify geometry
  webrain_stats      host health (load, memory, browser process count) — separates
                     "the page is slow" from "the machine is out of memory"
  webrain_guide      this guide
  webrain_domain_memory  persisted domain index (op: search|coverage|architecture|status)

BROWSER / PROFILE / SESSION / CHALLENGE MODEL
- Browser identity, profile, and session are EXECUTION STATE. Never treat a
  browser as disposable. Protected navigation starts with a persistent profile +
  real Chrome + a session — never an anonymous first attempt.
- webrain_navigate returns a `challenge` field — read it on EVERY navigate.
- challenge == null -> page OK, go extract.
- challenge != null (cloudflare_challenge | blocked | captcha) -> page is gated:
    * obscura / lightpanda CANNOT pass interactive challenges (the challenge JS
      crashes).
    * FIX — start real Chrome with a persistent profile + session (native):
        webrain_session(op=profiles)                        # list vault profiles
        webrain_session(op=open, cdp_url="http://127.0.0.1:9222")
        webrain_session(op=login, service, profile, url)    # vault + TOTP login
      or from the CLI: `webrain launch <service> <profile> <url>` then
      `webrain login <service> <profile>`. The profile persists — RE-USE it for
      subsequent navigations so cookies/session survive. Never discard a working
      profile/session after a challenge.
    * OR re-attach an already-authenticated Chrome: webrain_session(op=open,
      cdp_url=...) at its port, then re-navigate — the session is shared.
    * Interactive Turnstile/hCaptcha the native path can't claim need a human in
      the headed browser (2FA/approval gates return waiting_for_human:true).
    * Turnstile on an ordinary site: the old "obscura solves it" note is STALE.
      It claimed `scrapingcourse.com/login/cf-turnstile` solved (2026-08-07).
      RE-MEASURED 2026-11-01 — it does not: the page and the turnstile api.js
      load, `turnstile.render()` returns a widget id, but the placeholder div
      stays EMPTY (no iframe is ever created), the token stays "" at 9.8s / 19s /
      75s, `getResponse()` is undefined and observe(console) logs nothing — a
      silent no-op, not a slow solver. Real Chrome, same URL and same flags,
      returns a 730+ char token. Do not wait longer; see the obscura bullet below.
- COOKIE TRANSFER MOVES LOGIN STATE, NOT BOT CLEARANCE. cf_clearance / DataDome /
  _abck are bound to the minting browser's TLS fingerprint, so importing them into
  another engine fails even with a byte-identical UA. VERIFIED 2026-11-01
  (skroutz.gr): lightpanda (--cookie preloaded, --user-agent = Chrome's) and
  obscura (--user-agent, 4/4 cookies, 46s wait) both stayed on "Just a moment...",
  while the Chrome that minted the cookie returned 200. Batch gated pages on THAT
  Chrome (batch with cdp_urls=["http://127.0.0.1:9222"]); do not call it a cookie bug.
- A COOKIE BRIDGE IS PER-CONNECTION. `setcookies` fills the jar of the connection
  that wrote it: a CLI `webrain setcookies --port N` and the MCP session are two
  different jars, so the page bounces to /login while the shell HTML still answers
  200. Import on the session that will navigate (`webrain_session(op=setcookies,
  session_id=X)` then navigate with session_id=X) and VERIFY with an authenticated
  endpoint (200 vs 403) — `readback: N` only describes the writer's jar. MEASURED
  2026-11-21, serper.dev on obscura :9224: 8 cookies set on one connection, a fresh
  connection read 0 and /auth/me/ answered 403; the same cookies imported through
  the navigating session answered 200 with the dashboard.
- WEDGED ENGINE IS NOT A BLOCKED SITE. If /json/version and /json/list answer but
  Target.getTargets is [] and Page.navigate / Runtime.evaluate hang, the engine is
  wedged: restart it with the same flags and re-import cookies in-session. Do not
  re-run the same call.
- ENGINE IDENTITY (profiles exist beyond Chrome):
    obscura     serve --host 127.0.0.1 --port 9224 --stealth [--user-agent UA]
                [--storage-dir DIR]     # DIR persists the jar ACROSS RESTARTS only
                                        # (load at boot / save); it does NOT lift per-conn
                                        # jar isolation while the server is running
    lightpanda  serve [--user-agent UA] [--cookie in.json] [--cookie-jar out.json]
  `webrain obscura` passes ONLY --stealth (no --storage-dir/--user-agent) — launch
  the binary directly from WEBRAIN_BROWSERS_DIR for those. obscura refuses a
  0.0.0.0 bind without OBSCURA_CDP_TOKEN (>=32 bytes). Its /json/version keeps
  advertising the built-in UA — check navigator.userAgent after navigate instead.
- THE EPHEMERAL ("GUEST") CHROME IS A PURPOSE TOOL WITH TWO CALLERS — this SERP
  fallback and the interactive CAPTCHA/Turnstile solve. Its profile
  (`serp/<profile>`, persisted unless `--fresh`) is what makes the clearance stick
  for the NEXT call. It is launched with `--incognito`, not `--guest`: measured
  2026-11-07 on Chrome 151 and 156, `--guest` answers `Target.createTarget` with
  `-32000 "Failed to open new tab - no browser is open"` (Browse-as-Guest has no
  Browser in Chrome's BrowserList), while `--incognito` accepts both
  `createTarget` and `createBrowserContext`, so the old serial-tab caveat is gone.
  It is still not an engine: bulk `webrain_batch` / `webrain_crawl` belong on real
  Chrome, obscura or lightpanda — a queue in the clearance profile is how the
  clearance gets burned.
- obscura will NEVER pass a Turnstile/Cloudflare challenge — upstream lists
  "Cloudflare interactive challenges" under "what stealth does not handle". Its
  stealth TLS is a fixed wreq Profile::Chrome145 while navigator reports Chrome/143,
  so do NOT point --user-agent at another version (Chrome/156 to match a Chrome
  clearance only widens the TLS-vs-JS mismatch that is checked as a bot signal);
  canvas.getContext('webgl') is null; and Turnstile's own JS hangs there (issue #389).
  `--storage-dir` persists cookies, it does not make the browser convincingly Chrome.
  Batch gated pages in real Chrome; use obscura for unchallenged pages only.
- Never report success on a challenge/login/consent page — verify the target
  content before returning results.
- Need screenshots/rendering? Real Chrome. Fast no-challenge scraping? obscura.
  Static HTML, no JS/auth? webrain_scrape.

RENDERED BUT NOT INTERACTIVE (hydration — check BEFORE calling a page frozen)
A page can return status ok + challenge null + the right title and still never
mount: submit stays disabled, lists stay empty, spinners spin forever. That is
not a challenge and not a broken site.
- navigate returns at readyState complete, BEFORE an SPA hydrates. Re-probe ~1s
  later before declaring anything dead.
- Test hydration on a real control (0 = never mounted):
    const el = document.querySelector('input');
    Object.keys(el).filter(k => k.startsWith('__react')).length
- PROVEN CAUSE (ab.gr/storelocator, real Chrome, 2026-10-24): the injected stealth
  shim aborts framework bootstrap. A fake navigator.plugins threw
  TypeError: Illegal invocation inside page code, and the NOISE block (it replaces
  Object.defineProperty globally and hooks canvas/audio/canPlayType) made Next.js
  die with: Cannot read properties of undefined (reading getInitialProps).
- FIX: WEBRAIN_STEALTH_NOISE=0 for the process owning the CDP connection (restart
  an MCP server to apply — it is read at attach), then open a FRESH tab: init
  scripts live for that target lifetime, so re-navigating the same tab keeps the
  old shim and the env change looks ignored.
- If a hand-opened tab of the same URL works while yours does not, the difference
  is OUR injection, not the site. Never report the site as frozen.

WAIT ON THE LIST, NOT ON THE FIRST CARD
`wait_selector` is satisfied by ANY match reaching the state (fixed in this build) — but
the state is still worth choosing:
- SPA list pages (masoutis.gr, most Greek e-shops) render the SAME cards twice:
  a hidden mobile container first, the visible desktop one second. Waiting for
  `visible` used to lock onto the hidden copy; on 2026-11-06 that cost 30 s per
  URL (159 s for 27 URLs, `ms_total` 858 s of which 810 s was the timeout). Now
  the waiter scans every match; the same batch runs in ~10 s.
- Prefer `wait_selector_state:"attached"` with `wait_timeout_secs:6` when the
  page may legitimately have zero results (`div.product` never appears on a "no
  results" search): that row pays the whole timeout, so a big timeout only buys
  latency. `attached` + 6 s gave byte-identical results to `visible` + 30 s.
- Do not anchor on a shell element that exists before the cards (`#ProductsClass`
  returned in 0.2-2.4 s with found=0 everywhere). Anchor on the repeated card.

EXTRACTION MATRIX
- structured list, schema known    -> extract(mode=schema, base_selector, fields)
- schema unknown                   -> extract(mode=autoschema) + eval, then mode=schema
- paginated pages 1..N / many URLs -> webrain_batch(op=extract, urls, base_selector, fields)
- URLs unknown (discover pages)    -> eval: links whose path = current + '/<N>'
                                      + next/prev labels (NO hardcoded classes), derive
                                      range, then webrain_batch
- whole site                       -> crawl(mode=spider, seed_url)
- whole site, one template repeated -> crawl(mode=spider, dedupe_pages=true)
                                      → one page per template + `duplicates`
- emails/phones/prices/patterns    -> extract(mode=regex)
- JSON-LD / microdata              -> extract(mode=jsonld)
- tables                           -> extract(mode=table)
- infinite scroll / load-more      -> crawl(mode=scan) then extract; or interact(scroll)
- search                           -> webrain_search (navigate to results page)
- typed SERP results                -> webrain_serp (structured JSON; engine:
                                      duckduckgo|bing|google|brave|yahoo|baidu|yandex|auto.
                                      duckduckgo/bing/yahoo/baidu/yandex answer over plain
                                      HTTP with no browser; yahoo needs the client cookie
                                      jar, and baidu/yandex report a challenge page as
                                      `skipped`, never as an empty result set. auto merges
                                      ddg+bing then tops up from baidu/yandex/yahoo —
                                      yahoo last, it is the one that pages deep)
- relevance filter                 -> extract(mode=bm25)
- PDF (path or URL)                -> webrain_pdf(op=extract, path|paths[]) — text + pipe
                                      tables + layout flags, no browser, ~5 s for 9 pages.
                                      Text first, always: it is ~50x faster than pixels.
- PDF figures, as files            -> webrain_pdf(op=images, path, dir) — native-size PNGs,
                                      nested Form XObjects included, paths in the reply
- PDF pages as pixels             -> webrain_pdf(op=render, path, pages[], dpi:150, dir) —
                                      needs pdfium: `webrain install pdfium` once
- read the pixels                 -> vision(op=ask, images:[<paths from the two calls
                                      above>], prompt) — files are read from disk, no tab
- which PDF pages need pixels     -> the `layout` block of op=extract
                                      (pages_needing_vision / _with_raster_images /
                                      _with_captions); ask vision to DESCRIBE a figure or
                                      read a table, never to transcribe a whole page
- watch a video (URL or local file)-> webrain_watch(url) — timestamped transcript
                                      (yt-dlp captions -> local whisper-cli -> Whisper
                                      STT API) + frame file paths; the LLM reads frames
                                      + transcript to summarize. First run:
                                      `webrain install watch` bundles ffmpeg/yt-dlp/
                                      whisper-cli + a GGUF model (no PATH installs).

FROM-SCRATCH DISCOVERY (schema + urls unknown)
  1. webrain_navigate(seed) — read `links` (same-origin) + `challenge`
  2. derive urls from `links` (pagination/next via eval) -> max page -> urls
  3. extract(mode=autoschema) -> container selector
  4. eval -> descendant tags/classes + samples -> fields
  5. webrain_batch(op=extract, urls, base_selector, fields, concurrency=8) -> aggregate
     (read the parsed `data` array — no need to parse `text`)
  6. done(summary="Extracted N items across M pages")

LOAD-MORE / INFINITE-SCROLL SHORTCUT (fastest path)
  These pages almost always back the button/observer with a plain JSON/HTML
  endpoint (scrapingcourse uses /ajax/products?offset=N). Find it in the page's
  own script via eval (grep '/ajax/' in script tags), then
  webrain_batch(op=extract, urls=[...offset=0,10,20...], base_selector, fields)
  directly — one call, no interaction, no scroll. Dedupe overlapping offset
  windows by url/name if the endpoint returns a sliding window.

RULES
- NEVER return raw HTML. observe(what=state|fit|clean) + extract give page
  text/structure far cheaper. observe(what=html) is LAST RESORT — only when the
  task explicitly asks for HTML markup, and if you use it, say why.
- Media URLs (images/videos): extract meta[property='og:image'|'og:video'] as
  attr fields — NEVER `video.src`: for streamed media (reels/DASH) it's a blob:
  URL that can't be downloaded; og:video/og:image carry the real CDN file.
- Never guess selectors/browsers from memory — discover via autoschema/eval and
  read the `challenge` field on every navigate.
- webrain_batch(op=extract) returns each result's products as a parsed `data`
  array (single-page extract_json shape) — read `data`, don't parse `text`.
- eval does NOT reliably await async JS on obscura (returns null). For async
  work use webrain_batch(op=interact, ...) — its interaction runs in a session
  where awaits resolve.
- interact(action=tab): new(url) | switch(id) | close(id) | list. Use tabs to
  isolate parallel scrapes or pre-load login sessions in one tab while scraping
  in another.

MULTI-AGENT DELEGATION (orchestrator pattern — when to spawn subagents)
- webrain's MCP server makes a per-connection session per client. You can spawn
  subagents, give each its own browser via CDP_URL, and they run in parallel:
    Subagent A → CDP_URL=http://127.0.0.1:9222  (real Chrome — CF/SPA/interactive)
    Subagent B → CDP_URL=http://127.0.0.1:9224  (obscura — fast bulk batch)
    Subagent C → (no CDP)                        (webrain_scrape — static)
- DELEGATE when you hit ANY of these; otherwise stay single-threaded:
    1. Many independent URLs/pages/sites → shard the URL list across subagents.
       One browser already parallelizes in-process (webrain_batch concurrency=N);
       delegate ACROSS browsers for more throughput or different engines.
    2. Different engines/roles → challenges/SPAs to Chrome, bulk to obscura,
       static to webrain_scrape.
    3. Per-proxy / per-IP isolation → one CDP_URL per subagent = own
       proxy/cookies/fingerprint = N exit IPs at once.
    4. Huge site-wide crawl → shard by subdomain/section, one crawl(spider)
       per subagent (own crawldir for checkpoint/resume).
    5. Discovery overlaps extraction → one subagent finds schema/URLs on a new
       site while others extract from known sites.
- LAST PARALLEL LEVER: a single browser ALREADY parallelizes (webrain_batch
  concurrency=N, cdp_urls round-robin). Delegate only when in-browser
  parallelism is exhausted or you need DIFFERENT browsers/proxies.
- ENGINE PARALLELISM IS NOT UNIFORM. Real Chrome and obscura open N parallel
  tabs on one connection, so `concurrency` is the lever. **lightpanda is
  single-target per CONNECTION**: Target.createTarget answers
  TargetAlreadyLoaded once the context has a target, and a 2nd
  Target.createBrowserContext is refused ("Cannot have more than one browser
  context at a time"). A single endpoint therefore serializes and says so —
  `single_target: true, effective_concurrency: 1` — and no `concurrency` value
  changes that. BUT the process serves many connections: `serve` runs a worker
  thread (own context + page) per CDP WebSocket, capped by
  `--cdp-max-connections` (default 16). So one cheap lever is to **repeat the
  same endpoint in `cdp_urls`** (measured, one container: 1 conn 8 URLs 6.15s,
  4 conns 1.17s, 8 conns 16 URLs 4.19s). Workers in ONE process share its HTTP
  pool (--http-max-concurrent 40, --http-max-host-open 6 per host), so when many
  URLs hit the same host, separate processes win instead (25 URLs: 8 conns to 1
  container 9.6s vs 10 containers 3.5s). Pick connections for many hosts,
  processes for one host or per-IP isolation; either way CPU is the ceiling
  (~4 productive workers per 4 cores).
- DELEGATE BY PATTERN (shard the TASK, not the steps):
    catalog / "find all" / many urls -> links -> validate -> batch; >~100 urls
      or mixed engines -> shard urls across subagents
    specific pages (3,4,7)          -> single agent, NO delegation
    infinite scroll / load more     -> webrain_batch(op=interact) per site-group
    whole site / huge               -> crawl(spider); shard by subdomain, one
      spider subagent each (own crawldir)
- SUBAGENT SELF-HEAL (fallback chains so it returns data, not failure):
    extraction: observe(fit|flatten) -> extract -> eval -> observe(annotate)
    pagination: construct /page/N -> interact(click Next) -> scroll -> crawl(scan)
    anti-bot:   on challenge, STOP + report — you re-route to the Chrome agent
- SUBAGENT CONTRACT (give every subagent exact scope):
    * ONE browser (CDP_URL), ONE task, explicit URL list OR seed+budget.
    * Return COMPACT JSON only: {status, count, data[] | summary}. No raw HTML.
    * On challenge/block: REPORT it (challenge field), don't fight it.
- AGGREGATE yourself: dedupe by url/name (sliding windows), extract(mode=bm25)
  for relevance, merge into one answer with a count.
- Subagents are just other LLMs with webrain MCP — they can nest or use
  webrain_batch concurrency inside their own shard.

ENVIRONMENT — read at PROCESS START, a tool call cannot change them
  Read once at server start / CDP attach. webrain cannot set one for you.
  Tell the human the exact VAR=value and that the MCP server must restart; then
  open a FRESH tab, because injected scripts outlive a navigate. This list ships
  inside the binary so it works with no repo checkout (docs/reference/
  environment.mdx adds the platform defaults).
  A relaunch is THREE things at once, not one: CDP_URL pointing at the browser
  you validated, WEBRAIN_STEALTH_NOISE=0 if that page needed it, and
  WEBRAIN_NO_STEALTH DELETED — `webrain serp` sets WEBRAIN_NO_STEALTH=1 on
  itself, so an inherited value silently boots the server with ALL stealth off.
  - CDP_URL — CDP endpoint attached at startup (default http://127.0.0.1:9222).
    This IS the browser choice: real Chrome, obscura, lightpanda, or a remote
    Chrome. "Which engine" everywhere in this guide is a CDP_URL decision.
    A raw ws:// or wss:// URL is accepted as-is (no /json/version probe), which
    is how hosted endpoints like Lightpanda Cloud are reached; cdp_urls takes
    the same forms.
  - WEBRAIN_STEALTH_NOISE=0 — drop the canvas/audio/Object.defineProperty
    fingerprint NOISE block. THE hydration fix (see RENDERED BUT NOT
    INTERACTIVE). Without a server restart + fresh tab it looks ignored.
  - WEBRAIN_NO_STEALTH=1 — drop the whole injected stealth JS + UA override.
    Diagnostic for "is OUR shim breaking this page?"; the rest still works.
    Leave stealth on for hostile sites — this is not a workaround for them.
  - WEBRAIN_NO_STATE=1 — shared-instance mode: refuses vault/launch/login/
    session and clamps tab fan-out. "disabled on this instance" is this var,
    not a bug — report it instead of retrying.
  - WEBRAIN_HTTP_TOKEN / WEBRAIN_ALLOW_UNAUTHENTICATED — bearer auth for the
    HTTP transport; the second permits a non-loopback bind without a token. An
    auth rejection is a deployment fact, never a page problem.
  - WEBRAIN_PROXIES — proxy rotation pool (comma/newline list) for the HTTP
    engines and the google/brave guest Chrome. A per-call `proxy` overrides it
    and never rotates.
  - WEBRAIN_SKILLS_DIR — operator .md skills, read here via topic="skills" or
    topic="skill:<name>". Unset means the feature is off.
  - WEBRAIN_MCP_TOOLS — advertise only the named tools (`serp,scrape`). The
    full schema set is ~8k tokens before the prompt; on a small-per-minute tier
    that alone is a rejection, so trimming is the fix, not a workaround.
  - WEBRAIN_BROWSERS_DIR / WEBRAIN_CHROME / WEBRAIN_LIGHTPANDA / WEBRAIN_OBSCURA
    — engine discovery. "engine not found" means one of these is wrong or
    nothing is installed (`webrain install --engine obscura|lightpanda`).
  - WEBRAIN_PROFILES_DIR / WEBRAIN_VAULT_DIR — persistent browser profiles and
    the encrypted vault. Profiles are what keeps a challenge solved across
    calls; a moved profile root looks exactly like "logged out".
  - WEBRAIN_USER / WEBRAIN_PASS — login env fallback (the vault is tried first).
  - BRAVE_API_KEY / SERPAPI_API_KEY — paid SERP tiers; Brave also makes
    engine=brave work with no browser. Unset only lowers the provider tier.
  - Vision `ask` chain, in order: OPENROUTER_API_KEY → OPENAI_API_KEY →
    ANTHROPIC_API_KEY (or CLAUDE_API_KEY) → GEMINI_API_KEY (or GOOGLE_API_KEY)
    → FIREWORKS_API_KEY → DASHSCOPE_API_KEY (or QWEN_API_KEY) → GROQ_API_KEY →
    bundled local Qwen3-VL-2B. Every hop is the same OpenAI-compatible
    chat/completions call, so one key is enough; unset keys merely drop a hop.
    Cloud transcript (STT) is a different list: GROQ → OPENAI → FIREWORKS.
  - EMBED_URL / EMBED_MODEL / EMBED_API_KEY (and EMBED_FALLBACK_*) — embeddings
    for webrain_vision index/retrieve.
  - WEBRAIN_HTTP_CACHE=0, WEBRAIN_HTTP_CACHE_DIR, WEBRAIN_DOWNLOAD_MAX_BYTES,
    WEBRAIN_FETCH_TIMEOUT_SECS, WEBRAIN_HTTP_ADDR — transport and cache limits.
  - WEBRAIN_STT_MODEL, WEBRAIN_WHISPER_BIN, WEBRAIN_WHISPER_MODEL — transcript
    model/binary overrides.
  - RUST_LOG — log verbosity (webrain=info,tungstenite=warn).
"#;

/// Is this line a guide section header?
///
/// Headers are the unindented lines that OPEN with an all-caps word ("RULES",
/// "EXTRACTION MATRIX", "19 TOOLS — …", "FROM-SCRATCH DISCOVERY (…)"); body lines
/// are indented or start with a bullet, so the shape is enough and the const needs
/// no markers. Testing the whole line would miss the three headers that carry a
/// lowercase tail after their title.
fn is_guide_header(t: &str) -> bool {
    if t.is_empty() || !t.starts_with(|c: char| c.is_ascii_alphanumeric()) {
        return false;
    }
    let word = t
        .split_whitespace()
        .find(|w| w.chars().any(|c| c.is_alphabetic()))
        .unwrap_or("");
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphabetic()).collect();
    letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase())
}

/// Split AGENT_GUIDE on its header lines. Encoding the headers in the text's own
/// shape (not markers) keeps the const readable and means adding a section needs
/// no parser change.
fn guide_sections() -> (Option<&'static str>, Vec<(&'static str, &'static str)>) {
    let mut marks: Vec<(usize, usize)> = Vec::new(); // (title start, body start)
    let mut pos = 0usize;
    for line in AGENT_GUIDE.split_inclusive('\n') {
        if is_guide_header(line.trim_end_matches(['\r', '\n'])) {
            marks.push((pos, pos + line.len()));
        }
        pos += line.len();
    }
    let Some(first) = marks.first() else {
        return (Some(AGENT_GUIDE), Vec::new());
    };
    let intro = AGENT_GUIDE[..first.0].trim_end();
    let mut sections = Vec::with_capacity(marks.len());
    for (i, &(title_start, body_start)) in marks.iter().enumerate() {
        let end = marks.get(i + 1).map(|m| m.0).unwrap_or(AGENT_GUIDE.len());
        sections.push((
            AGENT_GUIDE[title_start..body_start].trim_end(),
            &AGENT_GUIDE[body_start..end],
        ));
    }
    (Some(intro), sections)
}

/// Directory of operator-supplied skill files: `WEBRAIN_SKILLS_DIR` when set,
/// otherwise the directory `webrain install skills` populates. An empty
/// directory serves nothing, so a shared instance stays silent until its
/// operator installs something.
fn skills_dir() -> std::path::PathBuf {
    std::env::var_os("WEBRAIN_SKILLS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(webrain_core::skills::dir)
}

/// Resolve `skill:<name>` to a file under `dir`. Naming, recursion and the
/// containment check all live in `webrain_core::skills`, shared with the
/// installer so what it writes is exactly what this can find.
fn skill_path(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    webrain_core::skills::resolve(dir, name)
}

fn list_skills(dir: &std::path::Path) -> Vec<String> {
    webrain_core::skills::list(dir)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// Read a skill file, capped. shortcut: 64 KiB, raise it if a real skill outgrows it.
fn read_skill(p: &std::path::Path) -> Option<String> {
    use std::io::Read;
    let mut buf = String::new();
    std::fs::File::open(p)
        .ok()?
        .take(64 * 1024)
        .read_to_string(&mut buf)
        .ok()?;
    Some(buf)
}

/// The text of one installed skill. Shared by `webrain_guide(topic="skill:<n>")`
/// and by the CLI's `@name` mention expansion, so a mention and a guide read are
/// guaranteed to be the same bytes.
pub fn skill_text(name: &str) -> Option<String> {
    skill_path(&skills_dir(), name.trim()).and_then(|p| read_skill(&p))
}

/// `webrain_guide` body. Default is an INDEX plus the tool list; `topic` reads one
/// section; `topic="all"` returns the whole guide; `skills` / `skill:<name>` read
/// operator files.
///
/// The index exists because the guide is what an agent reads when it is LEAST
/// sure what to do — paying ~3K tokens of doctrine at that moment is the expensive
/// default. Progressive disclosure makes the first call cheap and the second
/// precise. Selection is the model's job on purpose: matching a user prompt to a
/// topic in Rust would be the intent-detection this server exists to avoid.
pub fn guide_from_args(args: &Value) -> Value {
    let topic = args
        .get("topic")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    let (intro, sections) = guide_sections();
    let titles: Vec<String> = sections.iter().map(|(t, _)| t.to_string()).collect();

    if topic.is_empty() {
        let mut text = String::from(intro.unwrap_or(""));
        text.push_str(
            "\n\nTOPICS — call webrain_guide again with topic=<name> (or topic=\"all\" for everything):\n",
        );
        for (title, body) in &sections {
            // The tool list is used on nearly every call, so it stays inline.
            if title.starts_with("19 TOOLS") {
                text.push_str(&format!("\n{title}\n{body}"));
                continue;
            }
            let first: String = body
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim()
                .chars()
                .take(120)
                .collect();
            text.push_str(&format!("  - {title}: {first}\n"));
        }
        return json!({"status": "ok", "guide": text, "topics": titles});
    }
    if topic == "all" {
        return json!({"status": "ok", "topic": "all", "guide": AGENT_GUIDE});
    }
    if topic == "skills" {
        let dir = skills_dir();
        let skills = list_skills(&dir);
        let guide = if skills.is_empty() {
            format!(
                "no skills installed in {} — run `webrain install skills` for the bundled set, \
                 or `webrain skills add <url|file.md>` for one skill",
                dir.display()
            )
        } else {
            format!(
                "skills in {}: read one with webrain_guide(topic=\"skill:<name>\")",
                dir.display()
            )
        };
        return json!({"status": "ok", "guide": guide, "skills": skills});
    }
    if let Some(name) = topic.strip_prefix("skill:") {
        let name = name.trim();
        let dir = skills_dir();
        return match skill_text(name) {
            Some(t) => json!({"status": "ok", "skill": name, "guide": t}),
            None => json!({
                "status": "error",
                "message": format!("no skill named {name:?} (names are a-z A-Z 0-9 _ -, max 64; see topic=\"skills\")"),
                "skills": list_skills(&dir)
            }),
        };
    }
    for (title, body) in &sections {
        if title.to_ascii_lowercase().contains(&topic) {
            return json!({"status": "ok", "topic": title, "guide": format!("{title}\n{body}")});
        }
    }
    json!({
        "status": "ok",
        "guide": format!("no topic matching {topic:?} — pick one of: {}", titles.join(", ")),
        "topics": titles
    })
}

/// Consolidated MCP surface — 17 intent-based tools (firecrawl-style), each
/// with a `what`/`action`/`op`/`mode` selector + when-to-use guidance so the
/// LLM picks the right boundary. `call_tool` routes each call to the legacy
/// per-primitive executor via `map_surface()` (legacy names still dispatch).
/// The three load averages from `/proc/loadavg`.
fn parse_loadavg(text: &str) -> Option<(f64, f64, f64)> {
    let mut fields = text.split_whitespace();
    let one = fields.next()?.parse().ok()?;
    let five = fields.next()?.parse().ok()?;
    let fifteen = fields.next()?.parse().ok()?;
    Some((one, five, fifteen))
}

/// `(total_kb, available_kb)` from `/proc/meminfo`.
fn parse_meminfo(text: &str) -> Option<(u64, u64)> {
    let pick = |key: &str| -> Option<u64> {
        text.lines()
            .find(|line| line.starts_with(key))?
            .split_whitespace()
            .nth(1)?
            .parse()
            .ok()
    };
    Some((pick("MemTotal:")?, pick("MemAvailable:")?))
}

/// Host stats for the machine this process runs in.
///
/// Read from `/proc` directly: this answers "is this machine loaded, and is a
/// browser actually running inside it" without a sandbox sidecar, an FFI
/// dependency or a shell. `available: false` means the platform has no `/proc`
/// (macOS, Windows) — a fact to report rather than a number to guess.
///
/// `browser_processes` counts processes whose `comm` is a known engine, so one
/// Chrome shows up as many (zygote, gpu, renderers). It is a liveness signal, not
/// a tab count.
///
/// shortcut: linux `/proc` only — the session image is debian:12. Disk usage is
/// deliberately absent: every way to obtain it costs either a dependency
/// (`sysinfo`) or a process (a shell), and the second is the thing this exists to
/// avoid.
pub fn host_stats() -> Value {
    let load = std::fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|text| parse_loadavg(&text));
    let memory = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| parse_meminfo(&text))
        // No /proc (Windows, macOS): the same sysinfo read the tab budget uses,
        // so `memory` is no longer three nulls off Linux. Kept as a fallback so
        // each platform reports from one source.
        .or_else(|| {
            let total_mb = webrain_core::backends::cdp::memory_total_mb();
            (total_mb > 0).then(|| {
                (
                    total_mb * 1024,
                    webrain_core::backends::cdp::memory_available_mb() * 1024,
                )
            })
        });

    let mut total = 0_u32;
    let mut browsers = 0_u32;
    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            total += 1;
            let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
            if matches!(
                comm.trim(),
                "chrome"
                    | "chromium"
                    | "headless_shell"
                    | "obscura"
                    | "obscura-worker"
                    | "lightpanda"
            ) {
                browsers += 1;
            }
        }
    }

    let uptime_s = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|text| {
            text.split_whitespace()
                .next()
                .and_then(|value| value.parse::<f64>().ok())
        })
        .map(|value| value as u64);

    json!({
        "available": load.is_some(),
        // Say WHY instead of shipping nulls: load average and process counts come
        // from /proc (Linux). Memory now falls back to sysinfo, so only those two
        // are missing off Linux, and the reason names them.
        "reason": if load.is_some() {
            Value::Null
        } else {
            json!(format!(
                "{} has no /proc: load average and process counts are Linux-only. Memory comes from sysinfo; use the OS task manager for process counts.",
                std::env::consts::OS
            ))
        },
        "platform": std::env::consts::OS,
        "load": load.map(|(m1, m5, m15)| json!({"m1": m1, "m5": m5, "m15": m15})),
        "memory": memory.map(|(total_kb, available_kb)| json!({
            "total_mb": total_kb / 1024,
            "available_mb": available_kb / 1024,
        })),
        "processes": {"total": total, "browser_processes": browsers},
        "mcp": {
            "calls": crate::CALLS.load(std::sync::atomic::Ordering::Relaxed),
            "errors": crate::CALL_ERRORS.load(std::sync::atomic::Ordering::Relaxed),
            "retries": crate::RETRIES.load(std::sync::atomic::Ordering::Relaxed),
        },
        "uptime_s": uptime_s,
    })
}

/// Ceiling on the rows a batch reply may carry, in bytes of text/data.
///
/// MCP has no streaming response in this build, so for 1,000 URLs the reply
/// body is the memory hog, not the tabs (the driver sits at 11-13 MB across 1
/// to 32 tabs, PERF_AUDIT.md phase 1). This is the backpressure the transport
/// can actually deliver: a bounded body with a named count of dropped rows.
fn response_budget() -> usize {
    std::env::var("WEBRAIN_RESPONSE_BUDGET")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(8 * 1024 * 1024)
}

/// Blank `text`/`data` on rows past the budget, keeping url/title/error so the
/// caller still sees that the row ran. Returns (bytes kept, rows truncated).
fn apply_response_budget(payload: &mut Value) -> (usize, usize) {
    let budget = response_budget();
    let Some(rows) = payload.get_mut("results").and_then(|v| v.as_array_mut()) else {
        return (0, 0);
    };
    let (mut bytes, mut truncated) = (0usize, 0usize);
    for row in rows.iter_mut() {
        let len = row
            .get("text")
            .and_then(|v| v.as_str())
            .map(str::len)
            .unwrap_or(0)
            + row.get("data").map(|v| v.to_string().len()).unwrap_or(0);
        if bytes + len > budget {
            if row.get("text").is_some() {
                row["text"] = json!("");
            }
            if let Some(obj) = row.as_object_mut() {
                obj.remove("data");
            }
            truncated += 1;
        } else {
            bytes += len;
        }
    }
    (bytes, truncated)
}

/// Concurrency a batch really had: the tabs it measured, never the number it
/// asked for. 0 when every row failed — `effective_concurrency: 4` next to
/// `ok: 0` was the contradiction the guest-mode Chrome exposed.
fn effective_tabs(tabs_seen: usize, errs: usize, single: Option<bool>, asked: usize) -> usize {
    if tabs_seen > 0 {
        tabs_seen
    } else if errs > 0 {
        0
    } else if single == Some(true) {
        1
    } else {
        asked
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;

    #[test]
    fn effective_tabs_never_claims_a_tab_that_failed() {
        // Measured beats asked.
        assert_eq!(effective_tabs(3, 0, Some(false), 4), 3);
        // Nothing ran: 0, even though 4 were requested.
        assert_eq!(effective_tabs(0, 3, Some(true), 4), 0);
        // Single-target engine that did run: one tab.
        assert_eq!(effective_tabs(0, 0, Some(true), 4), 1);
        // No measurement available: the request is the best guess.
        assert_eq!(effective_tabs(0, 0, None, 4), 4);
    }

    #[test]
    fn parses_the_two_proc_files() {
        assert_eq!(
            parse_loadavg("0.52 0.41 0.30 1/234 5678\n"),
            Some((0.52, 0.41, 0.30))
        );
        assert_eq!(parse_loadavg("garbage"), None);

        let meminfo = "MemTotal:        2048000 kB\nMemFree:   100 kB\nMemAvailable:   512000 kB\n";
        assert_eq!(parse_meminfo(meminfo), Some((2_048_000, 512_000)));
        // MemAvailable is what a container can actually still use; without it the
        // number would be `MemFree`, which counts page cache as gone.
        assert_eq!(parse_meminfo("MemTotal: 1024 kB\n"), None);
    }
}

/// Tools that act on the attached page/browser and so honour the router's
/// per-call `session_id` (see `route_key` in lib.rs). Declared on every one of
/// them: the router has always accepted it, but an UNDECLARED parameter is one
/// no client will ever send, which made `webrain_session{op:open, cdp_url}`
/// write-only for everything except batch and session itself.
const SESSION_ROUTED_TOOLS: [&str; 12] = [
    "webrain_navigate",
    "webrain_observe",
    "webrain_network",
    "webrain_interact",
    "webrain_eval",
    "webrain_eval_in_frame",
    "webrain_extract",
    "webrain_vision",
    "webrain_pdf",
    "webrain_crawl",
    "webrain_search",
    "webrain_serp",
];

pub fn list_tools() -> Vec<Value> {
    let mut tools = vec![
        json!({
            "name": "webrain_guide",
            "description": "Agent decision guide: browser selection (real Chrome vs obscura vs lightpanda vs webrain_scrape), challenge bypass, hydration failures, extraction matrix, delegation doctrine, and the environment variables that change all of it. Call FIRST when unsure which webrain tool to use. Returns an index (plus the tool list) by default; pass `topic` to read one section.",
            "inputSchema": {"type": "object", "properties": {
                "topic": {"type": "string", "description": "One section by name — `tools`, `browser`, `extraction`, `delegation`, `rules`, `env`, … (case-insensitive substring) — or `all` for the whole guide, or `skills` / `skill:<name>` to read an operator skill from WEBRAIN_SKILLS_DIR"}
            }}
        }),
        json!({
            "name": "webrain_stats",
            "description": "Host stats for the machine webrain runs in: load average, memory total/available, process counts (including how many browser processes are alive), and uptime. Read-only, takes no arguments. `available: false` means this platform has no /proc. Use it to tell 'the page is slow' from 'the machine is out of memory'.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "webrain_domain_memory",
            "description": "Query the persistent web-memory SQLite store built by domain crawls: FTS5 BM25 search over indexed page text (op=search), coverage SLO counts discovered/fetched/indexed (op=coverage), host/URL/page/edge architecture (op=architecture), or global store counts (op=status). Read-only and browserless. Store path: `db` or WEBRAIN_MEMORY_DB (default ./web-memory.sqlite).",
            "inputSchema": {"type": "object", "properties": {
                "op": {"type": "string", "enum": ["search","coverage","architecture","status"], "description": "search | coverage | architecture | status"},
                "query": {"type": "string", "description": "search: keyword query (FTS5 BM25)"},
                "domain": {"type": "string", "description": "coverage/architecture/search filter; includes subdomains"},
                "top_k": {"type": "integer", "description": "search: max hits (default 10, max 100)"},
                "db": {"type": "string", "description": "SQLite store path (default WEBRAIN_MEMORY_DB or ./web-memory.sqlite)"}
            }, "required": ["op"]}
        }),
        json!({
            "name": "webrain_eval",
            "description": "Run arbitrary JavaScript in the current page and return the JSON result. The escape hatch for anything a tool doesn't cover (probe DOM, read scripts, call APIs).",
            "inputSchema": {"type": "object", "properties": {
                "js": {"type": "string", "description": "JS expression. Return a JSON-serializable value (JSON.stringify(...)). Or `@path/to/file.js` to read the script from disk, which spares you escaping quotes and backslashes into JSON twice."},
            }, "required": ["js"]}
        }),
        json!({
            "name": "webrain_eval_in_frame",
            "description": "Run JS inside a cross-origin iframe (isolated world). url_contains selects the iframe by src substring; js runs in that frame and returns the JSON result.",
            "inputSchema": {"type": "object", "properties": {
                "url_contains": {"type": "string", "description": "Substring of the target iframe's src to select it"},
                "js": {"type": "string", "description": "JS expression to run inside the iframe, or `@path/to/file.js` to read it from disk."}
            }, "required": ["url_contains", "js"]}
        }),
        json!({
            "name": "webrain_navigate",
            "description": "Navigate to a URL and return page state (title, visible text, interactive elements, same-origin links) plus `challenge` (cloudflare_challenge|blocked|captcha) when gated. THE entry point. Optional request-quality params: disable_resources, block_trackers, network_idle, wait_selector(+state), wait_timeout_secs, css_selector. Use `links` for one-call crawl discovery.",
            "inputSchema": {"type": "object", "properties": {
                "url": {"type": "string"},
                "disable_resources": {"type": "boolean", "default": false},
                "block_trackers": {"type": "boolean", "default": false},
                "network_idle": {"type": "boolean", "default": false},
                "wait_selector": {"type": "string"},
                "wait_selector_state": {"type": "string", "enum": ["attached","visible","hidden","detached"], "default": "visible"},
                "wait_timeout_secs": {"type": "integer", "default": 20, "description": "Max seconds to wait for page load + conditions"},
                "css_selector": {"type": "string", "description": "Narrow returned text to this element"}
            }, "required": ["url"]}
        }),
        json!({
            "name": "webrain_observe",
            "description": "Read the CURRENT page without navigating. Pick `what` (required): state (page state), a11y (accessibility tree, role/filter/max_nodes), semantic (text tree), html (raw HTML — LAST RESORT), images (img/srcset/lazy/icon + CSS background-image), assets (EVERY link internal+external+non-http scheme, images incl. backgrounds, video sources+embeds, one pass; optional only=links|images|videos and max per array), console (page errors), flatten (Shadow-DOM text), fit (dense content), clean (boiled text), screenshot (base64+file, full_page/dir), pixel (vision tiles), page_info (scroll/viewport), annotate (numbered-box overlay), media (network media URLs, url/wait_ms).",
            "inputSchema": {"type": "object", "properties": {
                "what": {"type": "string", "enum": ["state","a11y","semantic","html","images","assets","console","flatten","fit","clean","screenshot","pixel","page_info","annotate","media"]},
                "only": {"type": "string", "enum": ["links","images","videos"], "description": "assets: return just one group"},
                "max": {"type": "integer", "description": "assets: per-array cap (default 300)"},
                "role": {"type": "string", "description": "a11y: ARIA role filter"},
                "filter": {"type": "string", "description": "a11y: name/value/css substring filter"},
                "max_nodes": {"type": "integer", "description": "a11y: node cap"},
                "selector": {"type": "string", "description": "html: CSS selector to narrow"},
                "full_page": {"type": "boolean", "default": false, "description": "screenshot: full scrollable page"},
                "dir": {"type": "string", "default": "screenshots", "description": "screenshot: output dir"},
                "b64": {"type": "boolean", "default": true, "description": "screenshot: include base64 (that is the image block). false returns only the file path — use it for tall pages and long screenshot runs"},
                "tile_width": {"type": "number", "default": 800, "description": "pixel: tile width px"},
                "tile_height": {"type": "number", "default": 800, "description": "pixel: tile height px"},
                "max_tiles": {"type": "integer", "default": 16, "description": "pixel: max tiles"},
                "tiles_dir": {"type": "string", "description": "pixel: write the tiles here and return paths instead of base64 (a 660 KB PNG page is ~10 MB across 16 tiles — use this, then feed the paths to webrain_vision{op:'ask', images:[...]})"},
                "word_threshold": {"type": "integer", "default": 2, "description": "clean: min word length"},
                "format": {"type": "string", "enum": ["png", "jpeg", "webp", "avif"], "default": "png", "description": "screenshot: image encoding — webp/jpeg are ~10x smaller than png; avif is smallest but needs a CPU re-encode (CDP has no avif encoder)"},
                "quality": {"type": "integer", "default": 80, "description": "screenshot: jpeg/webp quality 1-100 (ignored for png)"},
                "exclude_social": {"type": "boolean", "default": true, "description": "clean: drop social links"},
                "url": {"type": "string", "description": "media: URL to load and capture network requests"},
                "wait_ms": {"type": "integer", "default": 0, "description": "media: keep capturing after load"}
            }, "required": ["what"]}
        }),
        json!({
            "name": "webrain_network",
            "description": "The page's OWN HTTP traffic — every request the app makes, not just what it paints. op=log arms CDP Network capture (optionally navigating first with url, then capturing for wait_ms) and returns entries: id, url, method, type, status, mime, bytes, at (epoch ms), ms, failed. The arm stays on afterwards, so log → interact (a click) → requests reads one buffer. op=request returns ONE response body by that id — this is how a JS app with no visible data gets scraped; ask while the renderer still holds it, and note an entry with bytes:0 has no body to give. op=requests re-reads the buffer; op=har exports the same rows as HAR 1.2 (headers/cookies are not captured, so they are HAR's -1/[] unknowns); op=clear empties it. Narrow with filter (URL substring), type, status, limit.",
            "inputSchema": {"type": "object", "properties": {
                "op": {"type": "string", "enum": ["log","requests","request","har","clear"]},
                "url": {"type": "string", "description": "log: navigate first and capture the whole load"},
                "wait_ms": {"type": "integer", "default": 0, "description": "log: keep capturing this long after load, for late XHR"},
                "id": {"type": "string", "description": "request: the id from op=log"},
                "filter": {"type": "string", "description": "only entries whose URL contains this"},
                "type": {"type": "string", "description": "only this resource type: XHR, Fetch, Document, Script, Image…"},
                "status": {"type": "integer", "description": "only this status code"},
                "limit": {"type": "integer", "default": 200, "description": "max entries returned"}
            }, "required": ["op"]}
        }),
        json!({
            "name": "webrain_interact",
            "description": "Drive the page. Pick `action` (required): click, click_coords (x,y), hold (x,y,ms: trusted press-and-hold for \"Press & Hold to confirm you are human\" / PerimeterX widgets, crosses cross-origin iframes), drag (x1,y1 → x2,y2 + optional ms hold: trusted slider/drag CAPTCHAs, eased smooth path, crosses cross-origin iframes), type (index,text + optional ms for real per-key typing), press (key or a shortcut like ctrl+a / shift+Tab), scroll (direction, or x,y,delta to wheel the element under a point), nav (back|forward|reload), tab (new|switch|close|list + url/id), select (index,value), hover (index), check (index,checked), dialog (accept|dismiss + prompt_text), wait (ms|selector|text + timeout_ms), upload (index,files[]), dismiss_overlays, add_init_script (js). Element indices map to the elements[] from navigate/observe. `click` also takes `selector` — the only path that reaches a widget inside a CLOSED shadow root (pass the root-scoped css_path that observe what=a11y prints).",
            "inputSchema": {"type": "object", "properties": {
                "action": {"type": "string", "enum": ["click","click_coords","hold","drag","type","press","scroll","nav","tab","select","hover","check","dialog","wait","upload","dismiss_overlays","add_init_script"]},
                "index": {"type": "integer", "description": "Element index from navigate/observe"},
                "selector": {"type": "string", "description": "click: CSS selector — the one path that reaches inside a CLOSED shadow root (use the root-scoped css_path from observe what=a11y); wait: CSS selector to poll for"},
                "text": {"type": "string", "description": "type: text to type; wait: text substring to wait for"},
                "key": {"type": "string", "description": "press: a key or a shortcut — Enter|Tab|Escape|Backspace|Delete|ArrowUp|ArrowDown|ArrowLeft|ArrowRight|Home|End|PageUp|PageDown|Space, or a combo like ctrl+a / shift+Tab / meta+v"},
                "direction": {"type": "string", "enum": ["up","down"], "default": "down", "description": "scroll"},
                "x": {"type": "number", "description": "click_coords/hold: viewport x; scroll: wheel point x"},
                "y": {"type": "number", "description": "click_coords/hold: viewport y; scroll: wheel point y"},
                "delta": {"type": "number", "description": "scroll: signed wheel delta at (x,y) — negative scrolls up, default ±500"},
                "x1": {"type": "number", "description": "drag: start x"},
                "y1": {"type": "number", "description": "drag: start y"},
                "x2": {"type": "number", "description": "drag: end x"},
                "y2": {"type": "number", "description": "drag: end y"},
                "nav": {"type": "string", "enum": ["back","forward","reload"], "default": "back"},
                "tab": {"type": "string", "enum": ["new","switch","close","list"], "default": "list"},
                "url": {"type": "string", "description": "tab new: URL"},
                "id": {"type": "string", "description": "tab switch/close: tab id"},
                "value": {"type": "string", "description": "select: option value or visible text"},
                "checked": {"type": "boolean", "default": true, "description": "check: desired state"},
                "dialog": {"type": "string", "enum": ["accept","dismiss"], "default": "accept"},
                "prompt_text": {"type": "string", "description": "dialog prompt(): text to type"},
                "ms": {"type": "integer", "description": "wait: fixed delay ms; type: per-key delay ms (real keydown/keyup typing); hold: how long to keep the button down (default 6000); drag: how long to hold the handle before moving (default 150)"},
                "timeout_ms": {"type": "integer", "default": 15000, "description": "wait: max ms"},
                "files": {"type": "array", "items": {"type": "string"}, "description": "upload: absolute paths"},
                "js": {"type": "string", "description": "add_init_script: JS to run on every new document"}
            }, "required": ["action"]}
        }),
        json!({
            "name": "webrain_extract",
            "description": "Structured extraction from the CURRENT page. Pick `mode` (required): schema (CSS-schema → JSON rows: base_selector+fields, optional adaptive), regex (built-in email/url/phone/... + custom patterns), jsonld (schema.org blocks), table (HTML tables → JSON), autoschema (discover repeated containers), bm25 (relevance-filter texts: query+items+top_k). Zero-LLM.",
            "inputSchema": {"type": "object", "properties": {
                "mode": {"type": "string", "enum": ["schema","regex","jsonld","table","autoschema","bm25"]},
                "base_selector": {"type": "string", "description": "schema: CSS selector per repeated item"},
                "fields": {"type": "array", "items": {"type": "object"}, "description": "schema: [{name, selector, type: text|attr|html|xpath, attr?}]"},
                "base_fields": {"type": "array", "items": {"type": "object"}, "description": "schema: container attrs [{name, attribute}]"},
                "adaptive": {"type": "boolean", "default": false, "description": "schema: relocate container if 0 matches (site redesign)"},
                "patterns": {"type": "array", "items": {"type": "object"}, "description": "regex: custom [{label, re}]"},
                "min_occurrences": {"type": "integer", "default": 3, "description": "autoschema: min repeats"},
                "query": {"type": "string", "description": "bm25: what the kept items should be about"},
                "items": {"type": "array", "items": {"type": "string"}, "description": "bm25: texts to rank"},
                "top_k": {"type": "integer", "default": 10, "description": "bm25: max results"}
            }, "required": ["mode"]}
        }),
        json!({
            "name": "webrain_scrape",
            "description": "Fetch ONE URL's content over plain HTTP — no browser, 10-100x faster, zero memory. Returns {url, status, text, needs_js}. For static pages / quick probes / pagination probing. For JS/SPA pages use webrain_navigate.",
            "inputSchema": {"type": "object", "properties": {
                "url": {"type": "string"},
                "base_selector": {"type": "string", "description": "optional CSS schema: selector per repeated item. With it the FULL page is fetched (no 3000-char probe cap) and parsed with the same field vocabulary as webrain_extract mode=schema -- no browser, so it runs on the shared pool."},
                "fields": {"type": "array", "items": {"type": "object"}, "description": "optional CSS schema fields: [{name, selector, type: text|attr|html|list|nested_list, attr?}]"},
                "autoschema": {"type": "boolean", "default": false, "description": "propose candidate CSS base selectors for this page instead of returning its text -- the browserless form of webrain_autoschema, so it runs on the shared pool. Answers [{selector, count}] to build a schema from."},
                "min_occurrences": {"type": "integer", "default": 3, "description": "autoschema: min repeats for a container to count as a pattern"},
                "max_chars": {"type": "integer", "description": "text cap in characters (default 3000). Raise it to PARSE the page rather than probe it; the reply reports truncated:true when the body was cut."},
                "full": {"type": "boolean", "default": false, "description": "return the whole body (no 3000-char cap). Use instead of upgrading to a browser when the page is static."},
                "proxy": {"type": "string", "description": "HTTP(S)/SOCKS proxy URL for this fetch, including the base_selector and autoschema forms (they fetch the full body over the same agent). Omit to use the WEBRAIN_PROXIES rotation pool."}
            }, "required": ["url"]}
        }),
        json!({
            "name": "webrain_batch",
            "description": "Run one op across MANY URLs in parallel tabs. op: fetch (visible text, 3000-char cap) | markdown (FULL page as Markdown, no cap; query+top_k bm25-prune it) | extract (CSS schema: base_selector+fields, in a browser tab) | scrape (the SAME schema over plain HTTP — no tab, no render, the batch twin of webrain_scrape) | read (the batch twin of `webrain read`: HTTP markdown negotiation, `.md` retry, nearest llms.txt — no tab) | interact (async JS per tab) | eval (custom JS extractor) | assets (every link internal+external, images incl. CSS backgrounds, video sources+embeds — counts plus a `max` sample per URL) | screenshot. concurrency bounds in-flight tabs; cdp_urls round-robins across N browsers and runs their batches CONCURRENTLY (per-proxy isolation). For a single-target engine — lightpanda, one page per CONNECTION — repeat the same endpoint in cdp_urls to get parallel pages (up to --cdp-max-connections, default 16); on one connection it reports single_target:true, effective_concurrency:1. The workhorse for at-scale scraping. PDFs are webrain_pdf, not an op here. Every row is {url, title, text, data, error, ms}: extract/scrape/read/eval/interact put the payload in `data` (text stays empty), fetch/markdown put it in `text`; `title` is the document that was actually read, so a challenge/consent/404 title means you did NOT read the page you asked for.",
            "inputSchema": {"type": "object", "properties": {
                "op": {"type": "string", "enum": ["fetch","markdown","extract","scrape","read","interact","eval","screenshot","autoschema","assets","network"], "description": "pdf moved to its own tool (webrain_pdf) — it was advertised here but never dispatched, so `op=pdf` silently returned an empty fetch row. scrape: the CSS schema over plain HTTP, no tab (the batch twin of webrain_scrape); extract: the same schema in a browser tab; read: the batch twin of `webrain read` — HTTP markdown negotiation + `.md` retry + nearest llms.txt, no tab, payload in `data` with `source`. assets: internal+external links, images (incl. CSS backgrounds), videos, documents and network URLs, with `max` per array (default 200). network: every URL the page requested (Resource Timing, all initiator types)."},
                "min_occurrences": {"type": "integer", "default": 3, "description": "autoschema: min repeats for a container to count as a pattern"},
                "max": {"type": "integer", "description": "assets/network: per-array cap per URL (assets default 200, max 2000; network default 1000, max 5000)"},
                "urls": {"type": "array", "items": {"type": "string"}},
                "query": {"type": "string", "description": "markdown/read: optional focus — bm25 keeps only the top_k chunks relevant to this query (crawl4ai fit_markdown style, no LLM). For read it prunes the markdown the row produced, so N pages can be read for ONE question"},
                "proxy": {"type": "string", "description": "scrape/read ONLY (the browserless ops): HTTP(S)/SOCKS exit for the fetches. Omit to rotate the WEBRAIN_PROXIES pool per request; an explicit URL is honored as-is and never rotates. Browser ops (fetch/markdown/extract/interact/eval/screenshot) ignore it — their exit is the browser's, set at launch or via cdp_urls."},
                "top_k": {"type": "integer", "description": "markdown/read: keep this many query-relevant chunks (requires query; 0 = whole document)"},
                "filter": {"type": "string", "description": "read ONLY: return just the section whose heading contains this text — the verb's `--filter`, applied per row"},
                "outline": {"type": "boolean", "default": false, "description": "read ONLY: answer the heading tree instead of the body — the verb's `--outline`, applied per row"},
                "max_chars": {"type": "integer", "description": "read ONLY: cap the returned text after selection — the verb's `--max`"},
                "cdp_urls": {"type": "array", "items": {"type": "string"}, "description": "Round-robin URLs across these CDP backends (per-proxy isolation)"},
                "js": {"type": "string", "description": "eval: JS extractor to run in each tab (required when op=eval); `@path/to/file.js` reads it from disk instead"},
                "interaction": {"type": "string", "description": "interact: async JS to run in each tab; `@path/to/file.js` reads it from disk instead"},
                "base_selector": {"type": "string", "description": "extract/interact: repeated-item selector"},
                "fields": {"type": "array", "items": {"type": "object"}, "description": "extract/interact: schema"},
                "concurrency": {"type": "integer", "default": 4},
                "per_backend_concurrency": {"type": "integer", "default": 4, "description": "tabs per backend when cdp_urls set"},
                "dir": {"type": "string", "default": "screenshots", "description": "screenshot: output dir"},
                "output": {"type": "string", "description": "Persist the full payload to this file path"},
                "disable_resources": {"type": "boolean", "default": false},
                "block_trackers": {"type": "boolean", "default": false},
                "network_idle": {"type": "boolean", "default": false},
                "wait_selector": {"type": "string", "description": "Poll this CSS selector after navigate; `visible` is satisfied by ANY match, so a hidden mobile copy of a list (common in SPAs) does not stall the wait"},
                "wait_selectors": {"type": "array", "items": {"type": "string"}, "description": "per-URL wait selector, index-parallel to urls (empty string = no selector). Use when one batch spans HOSTS that render differently; overrides wait_selector per URL."},
                "wait_selector_state": {"type": "string", "enum": ["attached","visible","hidden","detached"], "default": "visible"},
                "wait_timeout_secs": {"type": "integer", "default": 20, "description": "Max seconds to wait for page load + conditions per URL. A page with no match at all pays this in full — set 6-8s on a list that may legitimately be empty"}
            }, "required": ["op", "urls"]}
        }),
        json!({
            "name": "webrain_crawl",
            "description": "Site traversal. Pick `mode` (required): spider (BFS/DFS/best-first whole-site crawl: seed_url + depth/pages/allow/deny/autothrottle/checkpoint), sitemap (discover URLs from a site's sitemap: url), scan (auto-scroll infinite-scroll: max_scrolls), validate (alive/dead URL probe: urls[]). Spider dedupes the frontier on path + sorted query (anchor fragments ignored, `#/` hash routes kept), so `?a=1&b=2` and `?b=2&a=1` are crawled once.",
            "inputSchema": {"type": "object", "properties": {
                "mode": {"type": "string", "enum": ["spider","sitemap","scan","validate"]},
                "proxy": {"type": "string", "description": "sitemap/validate ONLY (they are pure HTTP): proxy URL for the fetch. Omit to use the WEBRAIN_PROXIES rotation pool. spider/scan drive a browser, so their exit is set at launch or by the session context instead."},
                "seed_url": {"type": "string", "description": "spider: starting URL"},
                "max_depth": {"type": "integer", "default": 2},
                "max_pages": {"type": "integer", "default": 20},
                "strategy": {"type": "string", "enum": ["bfs","dfs","bestfirst"], "default": "bfs"},
                "same_domain": {"type": "boolean", "default": true},
                "allowed_domains": {"type": "array", "items": {"type": "string"}},
                "no_content": {"type": "boolean", "default": false, "description": "spider: link-only fast path"},
                "http_only": {"type": "boolean", "default": false, "description": "spider: fetch each page over the pooled HTTP agent instead of a browser tab (no JS/no tab; ideal for link discovery on static sites)"},
                "seed_from_sitemap": {"type": "boolean", "default": false, "description": "spider: seed the frontier from robots.txt/sitemap.xml before crawling; falls back to the seed URL when no sitemap exists"},
                "respect_robots": {"type": "boolean", "default": false},
                "keywords": {"type": "array", "items": {"type": "string"}, "description": "spider bestfirst: relevance keywords"},
                "allow": {"type": "array", "items": {"type": "string"}, "description": "spider: only follow URLs matching these regexes"},
                "deny": {"type": "array", "items": {"type": "string"}, "description": "spider: skip URLs matching these regexes"},
                "retry": {"type": "integer", "default": 0},
                "delay_ms": {"type": "integer", "default": 0},
                "crawl_timeout_secs": {"type": "integer", "default": 0, "description": "0 = no cap"},
                "autothrottle": {"type": "boolean", "default": false},
                "autothrottle_max_ms": {"type": "integer", "default": 30000},
                "crawldir": {"type": "string", "description": "spider: checkpoint/resume dir. Persists {queue, seen, results} atomically; a resume (same crawldir) returns prior + new results and never re-fetches. max_pages counts the whole crawldir — raise it on resume to fetch more"},
                "checkpoint_every": {"type": "integer", "default": 10},
                "dedupe_pages": {"type": "boolean", "default": false, "description": "spider: drop pages whose visible text repeats an earlier page and report `duplicates`. Off by default — it removes rows, so a caller wanting every facet/pagination row must leave it false"},
                "concurrency": {"type": "integer", "default": 4, "description": "spider: concurrent page fetches across tabs (real Chrome/obscura multi-target); single-target/lightpanda auto-serializes"},
                "disable_resources": {"type": "boolean", "default": false, "description": "spider: block fonts/images/media on every page fetch"},
                "block_trackers": {"type": "boolean", "default": false, "description": "spider: also block the 3500-domain tracker list"},
                "network_idle": {"type": "boolean", "default": false, "description": "spider: wait for network idle on every page"},
                "wait_selector": {"type": "string", "description": "spider: wait for this CSS selector on every page"},
                "wait_selector_state": {"type": "string", "enum": ["attached","visible","hidden","detached"], "default": "visible"},
                "wait_timeout_secs": {"type": "integer", "default": 20, "description": "spider: max seconds per page load"},
                "url": {"type": "string", "description": "sitemap: site root or sitemap URL"},
                "max_scrolls": {"type": "integer", "default": 15, "description": "scan: max scroll steps"},
                "urls": {"type": "array", "items": {"type": "string"}, "description": "validate: URLs to probe"}
            }, "required": ["mode"]}
        }),
        json!({
            "name": "webrain_search",
            "description": "Search the web and navigate to the results page. engine: duckduckgo (default, scrape-friendly) | google | bing | brave (needs a browser — SPA). Returns the results page state.",
            "inputSchema": {"type": "object", "properties": {
                "q": {"type": "string"},
                "engine": {"type": "string", "enum": ["duckduckgo","google","bing","brave"], "default": "duckduckgo"},
                "proxy": {"type": "string", "description": "HTTP(S)/SOCKS proxy URL for the fetch. Omit to use the WEBRAIN_PROXIES rotation pool. This tool fetches over HTTP with no browser, so the exit applies per call -- a launched browser's exit is set at launch (`--proxy-server`) instead."}
            }, "required": ["q"]}
        }),
        json!({
            "name": "webrain_serp",
            "description": "STRUCTURED search results as typed JSON (position/title/url/domain/snippet) — a SERP API for any LLM. engine: duckduckgo (default) | bing | google | brave | yahoo | baidu | yandex | auto. duckduckgo|bing|yahoo|baidu|yandex|auto are pure HTTP (no browser; yahoo needs the client cookie jar, baidu/yandex report a challenge page as skipped rather than empty). google|brave are JS-gated and auto-launch a guest Chrome (or attach to the connected CDP engine), then paginate the results page for more than 10. auto fetches duckduckgo+bing concurrently, then tops up from baidu, yandex and yahoo (yahoo last — it is the one that pages to a deep limit) until `limit` is met; google/brave join via the browser when one is attached, sequential. Relevance-filters junk pages, merges + dedupes, limit 1..=100. Features: provider fallback (fallback, default true), pagination (page), safe search (safe), region/locale (region, e.g. us-en/gb-en/gr-el), request_id + ms + per_engine breakdown + source (actual provider after a fallback) in the reply, retry with backoff. Returns {status, query, engine, source, results[], per_engine[], request_id, ms, skipped[]}.",
            "inputSchema": {"type": "object", "properties": {
                "q": {"type": "string"},
                "engine": {"type": "string", "enum": ["duckduckgo","bing","google","brave","yahoo","baidu","yandex","auto"], "default": "duckduckgo", "description": "duckduckgo|bing|yahoo|baidu|yandex|auto = HTTP (no browser); yahoo pages on b=11,21,...; baidu serves ~7 organic rows then a security check; yandex serves SmartCaptcha from datacenter IPs. brave = keyed API when BRAVE_API_KEY is set, else guest-browser flow; google = guest-browser flow"},
                "limit": {"type": "integer", "default": 10, "description": "max results, 1..=100"},
                "page": {"type": "integer", "default": 0, "description": "0-based results page"},
                "safe": {"type": "boolean", "default": false},
                "region": {"type": "string", "description": "locale/region, e.g. us-en, gb-en, gr-el, de-de"},
                "browser": {"type": "boolean", "default": false, "description": "bing only: true attaches/launches a guest Chrome and paginates its results page for more than ~10 results. Default false keeps the pure-HTTP path (measured 2026-10-10: 11 results for duckduckgo, 10 for bing at limit=50). google always needs a browser; brave needs one only when BRAVE_API_KEY is unset. Both ignore this flag."},
                "fallback": {"type": "boolean", "default": true, "description": "fall back to another provider when the requested engine errors or returns zero"},
                "retries": {"type": "integer", "default": 2, "description": "transient-failure retries with backoff, 0..=5"},
                "proxy": {"type": "string", "description": "HTTP(S)/SOCKS proxy URL (e.g. http://user:pass@host:port) for the HTTP engines (duckduckgo/bing/auto) and baked into the guest Chrome launch for google/brave. Omit to use the WEBRAIN_PROXIES rotation pool instead (round-robin across exits, auto-rotating off a 403/429/503 wall); an explicit URL here is honored as-is and never rotates."}
            }, "required": ["q"]}
        }),
        json!({
            "name": "webrain_pdf",
            "description": "PDF work. Pick `op` (required): page (save current page as PDF) | extract (PDF → markdown/JSON: path or paths[], each a file OR an http(s) URL) | render (PDF pages → PNG tiles for vision: path + optional pages/dpi/tile_size) — needs a --features pdfium build | images (extract embedded images: path + optional pages).",
            "inputSchema": {"type": "object", "properties": {
                "op": {"type": "string", "enum": ["page","extract","render","images"]},
                "path": {"type": "string", "description": "extract/render/images: PDF file path OR http(s) URL (fetched to a temp .pdf first — no browser needed)"},
                "paths": {"type": "array", "items": {"type": "string"}, "description": "extract: batch paths"},
                "pages": {"type": "array", "items": {"type": "integer"}, "description": "render/images: page numbers (1-based), omit = all"},
                "dpi": {"type": "number", "default": 150, "description": "render: DPI"},
                "tile_size": {"type": "integer", "description": "render: split pages into square tiles of this px"},
                "dir": {"type": "string", "description": "render/images: write the PNGs here and return paths instead of base64. render writes p001.png (or p001_t0x0.png with tile_size); images writes img_p001_000.png at the embedded image's native size. A 9-page 150-dpi render is ~15 MB of base64 and 29 extracted figures can be ~20 MB — paths keep both out of the context window."}
            }, "required": ["op"]}
        }),
        json!({
            "name": "webrain_download",
            "description": "Download files/media. engine: http (default — stream plain URLs, optional filter_extension narrows to one type) | ytdlp (video/audio via yt-dlp: HLS/DASH/playlists, audio_only, format, args[]). Batch via urls[].",
            "inputSchema": {"type": "object", "properties": {
                "urls": {"type": "array", "items": {"type": "string"}},
                "dir": {"type": "string", "default": "downloads"},
                "proxy": {"type": "string", "description": "HTTP(S)/SOCKS proxy URL for BOTH engines (http via the pooled agent, ytdlp via --proxy). Omit to use the WEBRAIN_PROXIES rotation pool."},
                "engine": {"type": "string", "enum": ["http","ytdlp"], "default": "http"},
                "filter_extension": {"type": "string", "description": "http: only download URLs ending in this extension"},
                "audio_only": {"type": "boolean", "default": false, "description": "ytdlp: extract mp3"},
                "format": {"type": "string", "description": "ytdlp: -f value"},
                "args": {"type": "array", "items": {"type": "string"}, "description": "ytdlp: extra CLI flags"}
            }, "required": ["urls"]}
        }),
        json!({
            "name": "webrain_watch",
            "description": "Watch a video (URL or local path): download with yt-dlp, extract a timestamped transcript + scene frames, hand them to the LLM to summarize or answer a question about the video. No browser needed.",
            "inputSchema": {"type": "object", "properties": {
                "url": {"type": "string", "description": "Video URL or local path"},
                "prompt": {"type": "string", "description": "Optional question to answer from the transcript"}
            }, "required": ["url"]}
        }),
        json!({
            "name": "webrain_session",
            "description": "Browser / auth / session state. Pick `op` (required): open (create named session: session_id+cdp_url; add `proxy` to route it through a browser context -- re-open the same session_id with a different proxy to switch a LIVE Chrome's egress -- and `keep_identity` to carry the previous session's cookies across that switch), close (destroy session), list (active sessions), cookies (export), setcookies (import cookies[]), save_state / restore_state (auth state ↔ state.json: service+profile, plus port or cdp_url), profiles (vault names), login (auto-login from vault: service+profile+url, plus port or cdp_url), captcha (buy a reCAPTCHA-v2 token from the configured paid solver: sitekey+page_url, optional data_s+proxy; returns token+provider -- YOU still inject it into the page and call the callback), close_launch (stop a launched Chrome: service+profile).",
            "inputSchema": {"type": "object", "properties": {
                "op": {"type": "string", "enum": ["open","close","list","cookies","setcookies","save_state","restore_state","profiles","login","captcha","close_launch"]},
                "session_id": {"type": "string", "description": "open/close: session name"},
                "proxy": {"type": "string", "description": "open: route THIS session through this proxy by creating a browser context with `proxyServer` (live switch -- re-open the same session_id with a different proxy to move it). Needs Chrome; lightpanda/obscura have no per-context proxy."},
                "keep_identity": {"type": "boolean", "description": "open (with proxy): copy the previous session's COOKIES into the new context so the login survives the switch. Cookies only -- localStorage/IndexedDB/service workers stay behind. Best-effort: for sites that treat an IP change as fraud, pin one exit per profile instead of switching."},
                "cdp_url": {"type": "string", "description": "open: CDP URL for this session; save_state/restore_state/login: the caller's own browser, used INSTEAD of `port`"},
                "cookies": {"type": "array", "items": {"type": "object"}, "description": "setcookies: cookie objects (name, value, domain, path, ...)"},
                "service": {"type": "string", "description": "save/restore_state, login, close_launch: service"},
                "profile": {"type": "string", "description": "save/restore_state, login, close_launch: profile"},
                "port": {"type": "integer", "default": 9222, "description": "state/login: CDP port"},
                "url": {"type": "string", "description": "login: login page to navigate first"},
                "sitekey": {"type": "string", "description": "captcha: reCAPTCHA v2 sitekey — read it from the page, e.g. document.querySelector('[data-sitekey]').dataset.sitekey (or the iframe src's `k=`)"},
                "page_url": {"type": "string", "description": "captcha: the page URL the token must be valid for"},
                "data_s": {"type": "string", "description": "captcha: Google /sorry's `data-s` challenge secret, when the page has one"},
                "inline": {"type": "boolean", "description": "save_state: return the state payload instead of writing state.json — for a caller that keeps the durable copy itself, so no credential is left on this worker"},
                "payload": {"type": "object", "description": "restore_state: the state object to apply (what `save_state inline` returned); used INSTEAD of state.json, which may not exist on a machine that never logged in"}
            }, "required": ["op"]}
        }),
        json!({
            "name": "webrain_vision",
            "description": "Vision over screenshot pixels with the bundled local Qwen3-VL. op: index (embed current-page tiles into a named index: tag + tile params) | retrieve (cosine top-k tile ids for a text query: tag+query+k) | ask (screenshot the viewport or a clip region and ask the local vision model a prompt — THE captcha/visual-QA tool; returns the model's answer, no cloud key needed).",
            "inputSchema": {"type": "object", "properties": {
                "op": {"type": "string", "enum": ["index","retrieve","ask"]},
                "tag": {"type": "string", "default": "default", "description": "Index name"},
                "max_tiles": {"type": "integer", "default": 8, "description": "index: max tiles to embed"},
                "tile_width": {"type": "number", "default": 800},
                "tile_height": {"type": "number", "default": 800},
                "query": {"type": "string", "description": "retrieve: text query"},
                "k": {"type": "integer", "default": 5, "description": "retrieve: top-k"},
                "prompt": {"type": "string", "description": "ask: question for the local vision model"},
                "scale": {"type": "number", "default": 1, "description": "ask: upscale factor for each clip (>1 = higher-res capture — use 3 for small captcha tiles so the 2B model reads them accurately)"},
                "tiles": {"type": "array", "items": {"type": "object", "properties": {"x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"}}}, "description": "ask: BATCH mode — array of clip regions; each is captured at `scale` and ALL sent in ONE llama request as numbered images (watch-frames batching). Use for per-tile captcha classification; the prompt must state the numbering (1..N)."},
                "images": {"type": "array", "items": {"type": "string"}, "description": "ask: file paths of tiles already saved by observe{what:pixel, tiles_dir}. Read from disk — no second capture, and paths keep the tool call small instead of megabytes of base64."},
                "x": {"type": "number", "description": "ask: clip region x (default full viewport)"},
                "y": {"type": "number", "description": "ask: clip region y"},
                "w": {"type": "number", "description": "ask: clip region width"},
                "h": {"type": "number", "description": "ask: clip region height"}
            }, "required": ["op"]}
        }),
    ];
    for t in tools.iter_mut() {
        if SESSION_ROUTED_TOOLS.contains(&t["name"].as_str().unwrap_or_default()) {
            t["inputSchema"]["properties"]["session_id"] = json!({
                "type": "string",
                "description": "Route this call to a browser opened with webrain_session{op:'open', session_id, cdp_url}. Omit to use the connection's browser (CDP_URL, default 9222)."
            });
        }
    }
    keep_tools(tools)
}

/// `WEBRAIN_MCP_TOOLS=serp,scrape` advertises only those. The full set is ~8k
/// tokens of schema on its own, which is over Groq's free 7k tokens/minute cap,
/// so trimming the surface is the only way that tier runs an agent at all.
/// Names are accepted with or without the `webrain_` prefix.
fn keep_tools(tools: Vec<Value>) -> Vec<Value> {
    match std::env::var("WEBRAIN_MCP_TOOLS") {
        Ok(want) => keep_tools_for(tools, &want),
        Err(_) => tools,
    }
}

fn keep_tools_for(tools: Vec<Value>, want: &str) -> Vec<Value> {
    let keep: Vec<String> = want
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if s.starts_with("webrain_") {
                s.to_string()
            } else {
                format!("webrain_{s}")
            }
        })
        .collect();
    if keep.is_empty() {
        return tools;
    }
    let kept: Vec<Value> = tools
        .iter()
        .filter(|t| {
            t["name"]
                .as_str()
                .is_some_and(|n| keep.iter().any(|k| k == n))
        })
        .cloned()
        .collect();
    // A typo must not silently leave the agent with no tools at all: an
    // unmatched list falls back to the full surface, which still works.
    if kept.is_empty() { tools } else { kept }
}

#[cfg(test)]
mod session_arg_tests {
    use super::*;

    /// `WEBRAIN_MCP_TOOLS` is what makes a 7k-tokens/minute tier usable, so a
    /// typo must degrade to the full surface, not to an agent with no tools.
    #[test]
    fn tool_filter_narrows_and_fails_open() {
        let all = list_tools();
        assert!(all.len() > 5);
        let two = keep_tools_for(all.clone(), "serp, webrain_scrape");
        assert_eq!(two.len(), 2, "{:?}", two);
        assert!(two.iter().any(|t| t["name"] == "webrain_serp"));
        assert_eq!(keep_tools_for(all.clone(), "nope").len(), all.len());
        assert_eq!(keep_tools_for(all.clone(), "").len(), all.len());
    }

    /// Every page tool advertises `session_id`, and the HTTP-only tools do not:
    /// the router keys on the argument, so a tool missing from this list would
    /// silently ignore a caller who sent one.
    #[test]
    fn page_tools_declare_session_id() {
        let tools = list_tools();
        for name in SESSION_ROUTED_TOOLS {
            let t = tools
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("{name} missing from list_tools"));
            assert!(
                t["inputSchema"]["properties"]["session_id"].is_object(),
                "{name} does not declare session_id"
            );
        }
        for name in ["webrain_scrape", "webrain_guide", "webrain_stats"] {
            let t = tools.iter().find(|t| t["name"] == name).unwrap();
            assert!(
                t["inputSchema"]["properties"]["session_id"].is_null(),
                "{name} has no browser and must not advertise session_id"
            );
        }
    }
}

/// Page-agent style page info + scroll hints (borrowed from alibaba/page-agent
/// getPageInfo): viewport/page size, scroll position, how much is above/below.
/// Just-in-time: tells the LLM when to scroll before interacting — no DOM dump.
const PAGE_INFO_JS: &str = r#"(() => {
  const vw = window.innerWidth, vh = window.innerHeight;
  const pw = Math.max(document.documentElement.scrollWidth, document.body.scrollWidth || 0);
  const ph = Math.max(document.documentElement.scrollHeight, document.body.scrollHeight || 0);
  const sx = window.scrollX || document.documentElement.scrollLeft || 0;
  const sy = window.scrollY || document.documentElement.scrollTop || 0;
  const below = Math.max(0, ph - (vh + sy));
  return { url: location.href, title: document.title, viewport: [vw, vh], page: [pw, ph],
    scroll: [sx, sy], pixels_above: sy, pixels_below: below,
    pages_above: vh > 0 ? +(sy / vh).toFixed(1) : 0, pages_below: vh > 0 ? +(below / vh).toFixed(1) : 0,
    total_pages: vh > 0 ? +(ph / vh).toFixed(1) : 0,
    position_pct: ph > vh ? Math.round((sy / (ph - vh)) * 100) : 0 };
})()"#;

/// One-pass page inventory: EVERY link kind (internal / external / non-http
/// scheme), images including CSS `background-image` (the ones `document.images`
/// cannot see), and video sources + embeds. `__LIM__` caps each array, so the
/// single-page read and the batch read share this one expression instead of two
/// near-copies that drift.
///
/// Why not extend `LINKS_JS` (the `navigate` field): that one is same-origin and
/// capped at 200 for context economy — every navigate pays for it. This is opt-in
/// (`observe what=assets`), so it can afford the wider net. Backgrounds need
/// `getComputedStyle` per element, hence the element cap: 2500 elements is ~0.3 s.
const ASSET_SCAN_JS_TMPL: &str = r#"(() => {
  try {
    const LIM = __LIM__;
    const host = location.hostname;
    const abs = (u) => { if (!u) return ''; try { return new URL(u, location.href).href; } catch (e) { return ''; } };
    const strip = (u) => u.split('#')[0];
    const kindOf = (u) => {
      const seg = (u.split('?')[0].split('/').pop() || '').toLowerCase();
      const d = seg.lastIndexOf('.');
      const e = d > -1 ? seg.slice(d + 1) : '';
      if (!e) return 'page';
      if (e === 'pdf') return 'pdf';
      if (['jpg','jpeg','png','gif','webp','avif','svg','ico','bmp','tif','tiff'].indexOf(e) > -1) return 'image';
      if (['mp4','webm','mov','m4v','avi','mkv','flv'].indexOf(e) > -1) return 'video';
      if (['m3u8','mpd'].indexOf(e) > -1) return 'stream';
      if (['mp3','m4a','ogg','wav','flac','opus'].indexOf(e) > -1) return 'audio';
      if (['doc','docx','xls','xlsx','ppt','pptx','csv','txt','md','json','xml','rss','atom'].indexOf(e) > -1) return 'doc';
      if (['zip','rar','7z','tar','gz','xz','bz2','apk','dmg','iso','msi','exe','deb','rpm'].indexOf(e) > -1) return 'archive';
      return 'page';
    };

    // ---- links: every kind. Relative and protocol-relative resolve to http(s);
    // anything else (mailto, tel, javascript, data, ftp) is counted by scheme.
    const inAll = new Set(), exAll = new Set(), other = {};
    const internal = [], external = [];
    const pushLink = (raw, text) => {
      const low = raw.toLowerCase();
      const hasScheme = /^[a-z][a-z0-9+.-]*:/.test(low);
      const isHttp = low.startsWith('http://') || low.startsWith('https://') || low.startsWith('//');
      if (hasScheme && !isHttp) {
        const sc = low.split(':')[0];
        if (sc && sc.length < 16 && sc !== 'javascript') other[sc] = (other[sc] || 0) + 1;
        return;
      }
      const u = strip(abs(raw));
      if (!u || !/^https?:/i.test(u)) return;
      let h = '';
      try { h = new URL(u).hostname; } catch (e) { return; }
      const sameSite = h === host || h.endsWith('.' + host);
      // Dedupe and count BEFORE the cap. Adding to the seen-set only inside the
      // cap guard made `counts.internal` report the cap (6) instead of the real
      // total — measured on 14 live sites, every one read exactly the limit.
      const seen = sameSite ? inAll : exAll;
      if (seen.has(u)) return;
      seen.add(u);
      const item = { url: u, kind: kindOf(u), text: (text || '').replace(/\s+/g, ' ').trim().slice(0, 80) };
      if (sameSite) {
        if (internal.length < LIM) internal.push(item);
      } else if (external.length < LIM) {
        external.push(item);
      }
    };
    for (const el of document.querySelectorAll('a[href], area[href]')) {
      pushLink(el.getAttribute('href') || '', el.textContent || '');
    }

    // ---- images: <img> (+ lazy attrs/srcset), <picture>, icon/og links, and CSS
    // background-image. `data:` URIs carry no URL an agent can fetch — skipped.
    const imgSeen = new Set(), images = [];
    const pushImg = (u, kind, alt) => {
      if (!u || !/^https?:/i.test(u) || imgSeen.has(u)) return;
      imgSeen.add(u);
      if (images.length < LIM) images.push({ src: u, kind: kind, alt: (alt || '').slice(0, 120) });
    };
    const srcset = (ss, kind) => {
      if (!ss) return;
      for (const part of ss.split(',')) {
        const u = part.trim().split(/\s+/)[0];
        if (u) pushImg(abs(u), kind, '');
      }
    };
    for (const img of document.querySelectorAll('img, input[type=image]')) {
      const alt = img.getAttribute('alt') || '';
      pushImg(img.currentSrc || img.src || '', 'img', alt);
      for (const a of ['data-src', 'data-original', 'data-lazy-src', 'data-lazy', 'data-url']) {
        const v = img.getAttribute(a);
        if (v) pushImg(abs(v), 'lazy', alt);
      }
      srcset(img.getAttribute('srcset'), 'imgset');
      srcset(img.getAttribute('data-srcset'), 'lazyset');
    }
    for (const s of document.querySelectorAll('picture source, video source[type^="image"]')) {
      srcset(s.getAttribute('srcset'), 'picture');
      const sc = s.getAttribute('src');
      if (sc) pushImg(abs(sc), 'picture', '');
    }
    for (const l of document.querySelectorAll('link[rel~="icon"], link[rel="image_src"], link[rel="apple-touch-icon"], meta[property="og:image"], meta[name="twitter:image"]')) {
      pushImg(abs(l.getAttribute('href') || l.getAttribute('content') || ''), 'icon', '');
    }
    const bgCandidates = document.querySelectorAll('body,main,section,article,header,footer,aside,nav,div,span,a,li,button,figure,figcaption,picture,td,th');
    const n = Math.min(bgCandidates.length, 1600);
    for (let i = 0; i < n; i++) {
      const el = bgCandidates[i];
      // Hidden nodes do not paint a background an agent can fetch.
      if (!el.offsetParent && el !== document.body) continue;
      let bi = '';
      try { bi = getComputedStyle(el).backgroundImage; } catch (e) { continue; }
      if (!bi || bi === 'none') continue;
      const found = bi.match(/url\((['"]?)([^'")]+)\1\)/g) || [];
      for (const raw of found) {
        const u = raw.replace(/^url\((['"]?)/, '').replace(/(['"]?)\)$/, '');
        if (u && !u.startsWith('data:')) pushImg(abs(u), 'background', '');
      }
    }

    // ---- videos: elements, posters, <source>, known embeds, direct file links,
    // and HLS/DASH the page actually requested.
    const vidSeen = new Set(), videos = [];
    const pushVid = (u, kind) => {
      if (!u || !/^https?:/i.test(u) || vidSeen.has(u)) return;
      vidSeen.add(u);
      if (videos.length < LIM) videos.push({ src: u, kind: kind });
    };
    for (const v of document.querySelectorAll('video, audio')) {
      pushVid(abs(v.getAttribute('src') || v.currentSrc || ''), v.tagName.toLowerCase());
      pushVid(abs(v.getAttribute('poster') || ''), 'poster');
      for (const s of v.querySelectorAll('source')) pushVid(abs(s.getAttribute('src') || ''), 'source');
    }
    const embeds = ['youtube.com','youtu.be','youtube-nocookie.com','vimeo.com','dailymotion.com','twitch.tv','tiktok.com','brightcove.net','brightcove.com','jwplayer.com','wistia.com','wistia.net','streamable.com','rumble.com','loom.com','vidyard.com','bilibili.com','rutube.ru','kick.com'];
    for (const f of document.querySelectorAll('iframe[src], embed[src], object[data]')) {
      const u = abs(f.getAttribute('src') || f.getAttribute('data') || '');
      if (!u) continue;
      let hh = '';
      try { hh = new URL(u).hostname.toLowerCase(); } catch (e) { continue; }
      // Host suffix, not substring: Google's signin URL carries
      // `continue=...youtube.com...` and matched a substring test, so a logged-out
      // YouTube watch page reported an "embed" that was a login redirect.
      for (const h of embeds) {
        if (hh === h || hh.endsWith('.' + h)) { pushVid(u, 'embed'); break; }
      }
    }
    for (const a of document.querySelectorAll('a[href]')) {
      const u = strip(abs(a.getAttribute('href') || ''));
      if (!u) continue;
      const k = kindOf(u);
      if (k === 'video' || k === 'stream' || k === 'audio') pushVid(u, 'link');
    }
    try {
      for (const e of performance.getEntriesByType('resource')) {
        const k = kindOf(e.name);
        if (k === 'video' || k === 'stream') pushVid(e.name, 'network');
      }
    } catch (e) {}

    // ---- documents: stylesheets, scripts, preloads, iframes, forms, manifests
    // and meta-refresh targets. These are links an agent can fetch but that the
    // anchor/media scans above do not see.
    const docSeen = new Set();
    const documents = {stylesheets: [], scripts: [], preloads: [], iframes: [], forms: [], meta_refresh: [], manifest: []};
    const pushDoc = (arr, u, kind) => {
      if (!u || !/^https?:/i.test(u)) return;
      const key = kind + '|' + u;
      if (docSeen.has(key)) return;
      docSeen.add(key);
      if (arr.length < LIM) arr.push({url: u, kind: kind});
    };
    for (const l of document.querySelectorAll('link[href]')) {
      const rel = (l.getAttribute('rel') || '').toLowerCase().split(/\s+/);
      const u = abs(l.getAttribute('href') || '');
      if (rel.indexOf('stylesheet') > -1) pushDoc(documents.stylesheets, u, 'stylesheet');
      else if (rel.indexOf('manifest') > -1) pushDoc(documents.manifest, u, 'manifest');
      else if (rel.some(r => ['preload','modulepreload','prefetch','dns-prefetch','preconnect'].indexOf(r) > -1)) pushDoc(documents.preloads, u, rel.join(' '));
    }
    for (const s of document.querySelectorAll('script[src]')) pushDoc(documents.scripts, abs(s.getAttribute('src') || ''), 'script');
    for (const f of document.querySelectorAll('iframe[src]')) pushDoc(documents.iframes, abs(f.getAttribute('src') || ''), 'iframe');
    for (const f of document.querySelectorAll('form[action]')) pushDoc(documents.forms, abs(f.getAttribute('action') || ''), 'form');
    for (const m of document.querySelectorAll('meta[http-equiv]')) {
      if ((m.getAttribute('http-equiv') || '').toLowerCase() !== 'refresh') continue;
      const mm = (m.getAttribute('content') || '').match(/url\s*=\s*([^;]+)/i);
      if (mm) pushDoc(documents.meta_refresh, abs(mm[1].trim().replace(/^['"]|['"]$/g, '')), 'refresh');
    }

    // ---- network: every URL the page actually requested (XHR/fetch/API images
    // included), from Resource Timing. One pass, no CDP arming.
    const netSeen = new Set(), network = [];
    try {
      for (const e of performance.getEntriesByType('resource')) {
        const u = e.name;
        if (!/^https?:/i.test(u) || netSeen.has(u)) continue;
        netSeen.add(u);
        if (network.length < LIM) network.push({url: u, type: e.initiatorType || '', transferSize: e.transferSize || 0, duration: Math.round(e.duration || 0)});
      }
    } catch (e) {}

    return JSON.stringify({
      url: location.href,
      host: host,
      counts: { internal: inAll.size, external: exAll.size, images: imgSeen.size, videos: vidSeen.size, documents: docSeen.size, network: netSeen.size, scanned: n },
      links: { internal: internal, external: external, other_schemes: other },
      images: images,
      videos: videos,
      documents: documents,
      network: network
    });
  } catch (e) { return JSON.stringify({ error: String(e) }); }
})()"#;

/// `__LIM__` → the per-array cap. Single-page reads want a full list; a 20-URL
/// batch wants counts plus a sample, or the reply is megabytes of JSON.
fn asset_scan_js(limit: usize) -> String {
    ASSET_SCAN_JS_TMPL.replace("__LIM__", &limit.to_string())
}

/// Every URL the page actually requested, via Resource Timing. This is the
/// cheap, batchable sibling of `webrain_network op=log` (which needs CDP armed
/// before navigation): no bodies, but all initiator types and transfer sizes.
const NETWORK_SCAN_JS_TMPL: &str = r#"(() => {
  try {
    const LIM = __LIM__;
    const seen = new Set(), network = [];
    for (const e of performance.getEntriesByType('resource')) {
      const u = e.name;
      if (!/^https?:/i.test(u) || seen.has(u)) continue;
      seen.add(u);
      if (network.length < LIM) network.push({url: u, type: e.initiatorType || '', transferSize: e.transferSize || 0, duration: Math.round(e.duration || 0)});
    }
    return JSON.stringify({url: location.href, count: seen.size, network: network});
  } catch (e) { return JSON.stringify({error: String(e)}); }
})()"#;

fn network_scan_js(limit: usize) -> String {
    NETWORK_SCAN_JS_TMPL.replace("__LIM__", &limit.to_string())
}

/// crawl4ai `PruningContentFilter` borrow: no-query "fit" content extractor.
/// Walks the DOM scoring blocks by text-vs-link density + tag importance, drops
/// nav/footer/aside/form/header boilerplate, and returns the dense text — the
/// meat of the page for the LLM, instead of raw innerText full of chrome.
/// ponytail: block tags + score thresholds are a heuristic; tune if a page type
/// over/under-prunes.
/// The in-page selector-discovery expression, shared by `webrain_autoschema`
/// and `webrain_batch{op:"autoschema"}`. One copy on purpose: two copies of a
/// heuristic drift, and then the batch answer disagrees with the single-page one.
fn autoschema_js(min_occurrences: usize) -> String {
    format!(
        r#"(()=>{{ const c={{}}; document.querySelectorAll('div[class],li,tr,article,section').forEach(el=>{{ const cls = el.className ? '.'+String(el.className).trim().split(/\s+/).slice(0,2).join('.') : ''; const key=el.tagName.toLowerCase()+cls; c[key]=(c[key]||0)+1; }}); return JSON.stringify(Object.entries(c).filter(([k,n])=>n>={min_occurrences}).sort((a,b)=>b[1]-a[1]).slice(0,8).map(([sel,count])=>({{selector:sel, count}}))); }})()"#
    )
}

const FIT_JS: &str = r#"(() => {
  // Only leaf blocks emit. MAIN/ARTICLE/SECTION are containers — they wrap the
  // whole page (e.g. Wikipedia <main class="mw-body">) and must always descend
  // so a container can't dump the entire page + toolbar as one block.
  const EMIT = new Set(['P','H1','H2','H3','H4','H5','H6','LI','TD','BLOCKQUOTE','PRE']);
  const BONUS = { H1:60, H2:50, H3:40, P:20, LI:15, TD:15, PRE:10, BLOCKQUOTE:10 };
  const skip = el => el.closest('nav,footer,aside,header,form,script,style,button,select,input');
  const out = [];
  let total = 0;
  const CAP = 60000;
  function walk(el) {
    if (total >= CAP) return;
    if (!el || el.nodeType !== 1) return;
    if (skip(el)) return;
    const text = (el.innerText || '').trim();
    const wc = text ? text.split(/\s+/).length : 0;
    if (EMIT.has(el.tagName) && wc >= 4) {
      let linkChars = 0;
      for (const a of el.querySelectorAll('a')) linkChars += (a.innerText || '').length;
      const density = text.length > 0 ? 1 - (linkChars / text.length) : 1;
      const score = wc * density + (BONUS[el.tagName] || 0);
      if (score >= 12 && density >= 0.5) { out.push(text); total += text.length; return; }
    }
    for (const c of el.children) walk(c);
  }
  for (const c of document.body.children) walk(c);
  return out.join('\n\n');
})()"#;

/// crawl4ai `flatten_shadow_dom` borrow, text-focused: walk light DOM + open
/// shadow roots recursively (resolving <slot> projections) and return the full
/// composed page text. Web-Component sites (Lit/Stencil/Shoelace) render content
/// in shadow roots that querySelectorAll/innerText miss entirely.
/// ponytail: open shadow roots only — closed roots need an attachShadow patch
/// injected before component creation; add if a target site uses closed roots.
const FLATTEN_JS: &str = r#"(() => {
  function textOf(node) {
    if (node.nodeType === 3) return node.textContent || '';
    if (node.nodeType !== 1) return '';
    const t = (node.tagName || '').toLowerCase();
    if (t === 'script' || t === 'style' || t === 'noscript') return '';
    if (t === 'slot') {
      const assigned = node.assignedNodes({ flatten: true });
      let s = '';
      for (const a of assigned) s += textOf(a);
      if (s) return s;
      let fb = '';
      for (const c of node.childNodes) fb += textOf(c);
      return fb;
    }
    let s = '';
    if (node.shadowRoot) for (const c of node.shadowRoot.childNodes) s += textOf(c);
    for (const c of node.childNodes) s += textOf(c);
    return s;
  }
  return (textOf(document.body) || '').replace(/\n{3,}/g, '\n\n').trim();
})()"#;

/// agent-browser `screenshot --annotate` borrow: overlay numbered red boxes on
/// interactive elements at VIEWPORT coords (position:fixed), matching webrain's
/// click indices. Returns the legend [{n, index, tag, text}] so the vision LLM
/// can read the labels and click by index. Viewport screenshot only.
const ANNOTATE_JS: &str = r#"(() => {
  const els = Array.from(document.querySelectorAll('a, button, input, select, textarea, [role="button"]'));
  const existing = document.getElementById('__webrain_annotate__');
  if (existing) existing.remove();
  const c = document.createElement('div');
  c.id = '__webrain_annotate__';
  c.style.cssText = 'position:fixed;top:0;left:0;width:0;height:0;pointer-events:none;z-index:2147483647;';
  const items = [];
  for (let i = 0; i < els.length && items.length < 50; i++) {
    const el = els[i];
    const r = el.getBoundingClientRect();
    if (r.width === 0 || r.height === 0) continue;
    const n = items.length + 1;
    const b = document.createElement('div');
    b.style.cssText = 'position:fixed;left:' + r.left + 'px;top:' + r.top + 'px;width:' + r.width + 'px;height:' + r.height + 'px;border:2px solid rgba(255,0,0,0.85);box-sizing:border-box;';
    const l = document.createElement('div');
    l.textContent = String(n);
    l.style.cssText = 'position:fixed;top:' + (r.top < 14 ? '0px' : (r.top - 15) + 'px') + ';left:' + r.left + 'px;background:rgba(255,0,0,0.9);color:#fff;font:bold 11px/14px monospace;padding:0 4px;border-radius:2px;white-space:nowrap;';
    b.appendChild(l);
    c.appendChild(b);
    items.push({ n: n, index: i, tag: el.tagName, text: (el.innerText || el.value || '').trim().slice(0, 50) });
  }
  document.documentElement.appendChild(c);
  return items;
})()"#;

/// Crude visible-text length of raw HTML (tag-strip; no regex dep in webrain-mcp).
pub(crate) fn visible_text_len(html: &str) -> usize {
    let mut out = 0usize;
    let mut in_tag = false;
    // Skip the BODIES of <script>/<style>/<noscript>: the old stripper counted
    // them as "visible text", so a JS shell with a long inline <script> looked
    // like it had content and was classified as needing no browser.
    let mut skip = 0usize;
    let mut tag = String::new();
    for ch in html.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' => {
                in_tag = false;
                let raw = tag.trim();
                let closing = raw.starts_with('/');
                let name = raw
                    .trim_start_matches('/')
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if matches!(name.as_str(), "script" | "style" | "noscript") {
                    if closing {
                        skip = skip.saturating_sub(1);
                    } else {
                        skip += 1;
                    }
                }
            }
            _ if in_tag => tag.push(ch),
            _ if skip == 0 => out += 1,
            _ => {}
        }
    }
    out
}

/// spider-rs `smart` borrow (lazy slice): is this raw HTML a JS shell the
/// browser is needed for? True when the page says JS is required, or when
/// there's almost no visible text AND script tags are present.
pub(crate) fn probe_needs_js(html: &str) -> bool {
    let l = html.to_ascii_lowercase();
    const JS_MARKERS: &[&str] = &[
        "enable javascript",
        "javascript is required",
        "javascript is disabled",
        "enable js",
    ];
    if JS_MARKERS.iter().any(|m| l.contains(m)) {
        return true;
    }
    // An HTML shell with almost no visible text is almost certainly JS-rendered.
    // (script tags often sit past the 3000-char text cap, so don't require them.)
    visible_text_len(html) < 100 && l.contains("<html")
}

/// Filter + cap an accessibility tree to what the LLM needs right now
/// (just-in-time: don't dump 30KB of nodes when only buttons/links matter).
fn filter_ax(nodes: &Value, role: Option<&str>, filter: Option<&str>, max: Option<usize>) -> Value {
    let arr = nodes.as_array().cloned().unwrap_or_default();
    let mut out: Vec<Value> = Vec::new();
    for n in arr {
        let nr = n
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_lowercase();
        if let Some(r) = role {
            let r = r.to_lowercase();
            // Exact or substring (e.g. "button" -> pushbutton/radiobutton). Forgiving
            // on purpose: an exact-role miss silently returns [] and sends the LLM
            // guessing (Google Material dropdowns are combobox/option, not button).
            if nr != r && !nr.contains(&r) {
                continue;
            }
        }
        if let Some(f) = filter {
            let f = f.to_lowercase();
            let name = n.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let value = n.get("value").and_then(|v| v.as_str()).unwrap_or("");
            let css = n.get("css_path").and_then(|v| v.as_str()).unwrap_or("");
            // Name is not enough: Google/Material controls often put their label in a
            // descendant (exposed as `value` or only reachable via css_path).
            if !name.to_lowercase().contains(&f)
                && !value.to_lowercase().contains(&f)
                && !css.to_lowercase().contains(&f)
            {
                continue;
            }
        }
        out.push(n);
        if let Some(m) = max {
            if out.len() >= m {
                break;
            }
        }
    }
    json!(out)
}

/// Parse a JS `JSON.stringify(...)` result string into a Value (Null on bad JSON).
fn parse_json_str(v: &Value) -> Value {
    v.as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null)
}

/// Length of a JSON array (0 for non-array).
fn arr_len(v: &Value) -> usize {
    v.as_array().map(|a| a.len()).unwrap_or(0)
}

/// Map the consolidated 17-tool surface to the legacy per-primitive executor.
/// Each tool's `what`/`action`/`op`/`mode` selects the legacy arm; the rest of
/// the args pass through unchanged (legacy arms read the same param names).
/// Legacy tool names map to None → handled by their own arm (backward compat).
pub fn map_surface(name: &str, args: &Value) -> Option<(&'static str, Value)> {
    let s = |k: &str| args.get(k).and_then(|v| v.as_str());
    let mut a = args.clone();
    let dropk = |a: &mut Value, k: &str| {
        if let Some(o) = a.as_object_mut() {
            o.remove(k);
        }
    };
    let route = |a: &mut Value, k: &str, v: &str| {
        if let Some(o) = a.as_object_mut() {
            o.insert(k.to_string(), json!(v));
        }
    };
    match name {
        "webrain_observe" => {
            let what = s("what")?;
            dropk(&mut a, "what");
            Some((
                match what {
                    "state" => "webrain_snapshot",
                    "a11y" => "webrain_a11y",
                    "semantic" => "webrain_semantic_tree",
                    "html" => "webrain_get_html",
                    "images" => "webrain_get_images",
                    "console" => "webrain_console",
                    "flatten" => "webrain_flatten",
                    "fit" => "webrain_fit",
                    "clean" => "webrain_clean",
                    "screenshot" => "webrain_screenshot",
                    "pixel" => "webrain_pixel",
                    "page_info" => "webrain_page_info",
                    "annotate" => "webrain_annotate",
                    "media" => "webrain_media",
                    "assets" => "webrain_assets",
                    _ => return None,
                },
                a,
            ))
        }
        "webrain_interact" => {
            let action = s("action")?;
            dropk(&mut a, "action");
            let legacy = match action {
                "click" => "webrain_click",
                "click_coords" => "webrain_click_coords",
                "hold" => "webrain_hold",
                "drag" => "webrain_drag",
                "type" => "webrain_type",
                "press" => "webrain_press",
                "scroll" => "webrain_scroll",
                "select" => "webrain_select",
                "hover" => "webrain_hover",
                "check" => "webrain_check",
                "wait" => "webrain_wait",
                "upload" => "webrain_upload",
                "dismiss_overlays" => "webrain_dismiss_overlays",
                "add_init_script" => "webrain_add_init_script",
                // legacy arms that read their own action/op key — rename the param
                "nav" => {
                    let v = s("nav").unwrap_or("back");
                    dropk(&mut a, "nav");
                    route(&mut a, "op", v);
                    "webrain_nav"
                }
                "tab" => {
                    let v = s("tab").unwrap_or("list");
                    dropk(&mut a, "tab");
                    route(&mut a, "action", v);
                    "webrain_tab"
                }
                "dialog" => {
                    let v = s("dialog").unwrap_or("accept");
                    dropk(&mut a, "dialog");
                    route(&mut a, "action", v);
                    "webrain_dialog"
                }
                _ => return None,
            };
            Some((legacy, a))
        }
        "webrain_extract" => {
            let mode = s("mode")?;
            dropk(&mut a, "mode");
            Some((
                match mode {
                    "schema" => "webrain_extract_json",
                    "regex" => "webrain_extract_regex",
                    "jsonld" => "webrain_get_jsonld",
                    "table" => "webrain_table",
                    "autoschema" => "webrain_autoschema",
                    "bm25" => "webrain_bm25",
                    _ => return None,
                },
                a,
            ))
        }
        "webrain_crawl" => {
            let mode = s("mode")?;
            dropk(&mut a, "mode");
            // ONE normalisation, at the only place crawl picks a mode.
            //
            // The schema advertises three seed spellings flat (`url`, `urls`,
            // `seed_url`) while each mode reads exactly one of them, so a caller
            // -- a model reading the schema, or the platform gateway in front of
            // it -- picks the wrong spelling and is answered "<x> required" for a
            // request that named a perfectly good URL. Measured against this
            // binary: spider wants `seed_url`, validate wants `urls`,
            // sitemap/scan want `url`.
            //
            // Filling in all three from whichever was supplied makes the mode
            // irrelevant to the caller, and doing it HERE rather than in each arm
            // means the next mode inherits it instead of re-learning it.
            let scalar = a
                .get("seed_url")
                .or_else(|| a.get("url"))
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(|| {
                    a.get("urls")
                        .and_then(|v| v.as_array())
                        .and_then(|list| list.first())
                        .and_then(|v| v.as_str())
                        .map(String::from)
                });
            if let (Some(one), Some(obj)) = (scalar, a.as_object_mut()) {
                if !obj.contains_key("url") {
                    obj.insert("url".into(), json!(one.clone()));
                }
                if !obj.contains_key("seed_url") {
                    obj.insert("seed_url".into(), json!(one.clone()));
                }
                if !obj.contains_key("urls") {
                    obj.insert("urls".into(), json!([one]));
                }
            }
            Some((
                match mode {
                    "spider" => "webrain_spider",
                    "sitemap" => "webrain_sitemap",
                    "scan" => "webrain_scan",
                    "validate" => "webrain_validate_urls",
                    _ => return None,
                },
                a,
            ))
        }
        "webrain_pdf" => {
            let op = s("op")?;
            dropk(&mut a, "op");
            Some((
                match op {
                    "page" => "webrain_pdf",
                    "extract" => "webrain_pdf_extract",
                    "render" => "webrain_pdf_render",
                    "images" => "webrain_pdf_images",
                    _ => return None,
                },
                a,
            ))
        }
        "webrain_session" => {
            let op = s("op")?;
            dropk(&mut a, "op");
            Some((
                match op {
                    "open" => "webrain_open_session",
                    "close" => "webrain_close_session",
                    "list" => "webrain_list_sessions",
                    "cookies" => "webrain_cookies",
                    "setcookies" => "webrain_setcookies",
                    "save_state" => "webrain_save_state",
                    "restore_state" => "webrain_restore_state",
                    "profiles" => "webrain_profiles",
                    "login" => "webrain_login",
                    "captcha" => "webrain_captcha",
                    "close_launch" => "webrain_close_launch",
                    _ => return None,
                },
                a,
            ))
        }
        "webrain_vision" => {
            let op = s("op")?;
            dropk(&mut a, "op");
            Some((
                match op {
                    "index" => "webrain_vision_index",
                    "retrieve" => "webrain_vision_retrieve",
                    "ask" => "webrain_vision_ask",
                    _ => return None,
                },
                a,
            ))
        }
        "webrain_scrape" => Some(("webrain_fetch_http", a)),
        // single-tool surface (navigate/eval/batch/search/download/watch/guide)
        _ => None,
    }
}

/// Dispatch tool calls to the shared CDP backend.
/// ponytail: match arm, no registry pattern.
/// Dispatch one tool call. The span carries the FOLDED tool name, so a
/// `webrain_observe{what:screenshot}` shows up as `webrain_screenshot` in
/// `RUST_LOG=debug` — the name that actually did the work.
// Parse Scrapling-style request-quality params from tool args (shared by
// navigate + batch). All optional; defaults keep legacy behavior.
fn nav_opts(args: &Value) -> webrain_core::backends::cdp::NavOpts {
    use webrain_core::backends::cdp::NavOpts;
    NavOpts {
        disable_resources: args
            .get("disable_resources")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        block_trackers: args
            .get("block_trackers")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        network_idle: args
            .get("network_idle")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        wait_selector: args
            .get("wait_selector")
            .and_then(|v| v.as_str())
            .map(String::from),
        wait_selector_state: args
            .get("wait_selector_state")
            .and_then(|v| v.as_str())
            .unwrap_or("visible")
            .to_string(),
        css_selector: args
            .get("css_selector")
            .and_then(|v| v.as_str())
            .map(String::from),
        wait_timeout_secs: args.get("wait_timeout_secs").and_then(|v| v.as_u64()),
    }
}
/// `js` / `interaction` may be a literal expression or `@path` — the convention
/// `webrain tool` already uses for its args, so a script full of quotes and regexes
/// never has to survive a second round of JSON escaping. A leading `@` that names no
/// file stays literal, so a typo runs (and fails loudly) instead of silently emptying.
fn js_arg(args: &Value, key: &str) -> Result<String, String> {
    let s = args.get(key).and_then(|v| v.as_str()).unwrap_or("");
    match s
        .strip_prefix('@')
        .filter(|p| std::path::Path::new(p).is_file())
    {
        Some(path) => std::fs::read_to_string(path).map_err(|e| format!("{s}: {e}")),
        None => Ok(s.to_string()),
    }
}

/// A bad `js`/`interaction` argument, as one row per URL — the same shape batch uses
/// for "js required", so a failed `@path` never reads as a successful empty batch.
fn batch_js_err(urls: &[String], msg: String) -> Vec<BatchResult> {
    urls.iter()
        .map(|u| BatchResult {
            url: u.clone(),
            title: String::new(),
            text: String::new(),
            data: None,
            error: Some(msg.clone()),
            ms: 0,
        })
        .collect()
}

#[tracing::instrument(skip_all, fields(tool = %name))]
pub async fn call_tool(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});

    // Consolidated 17-tool surface → legacy executor. Legacy tool names pass
    // through (their own arms still match) — old agents keep working.
    let mapped = map_surface(name, args);
    let name: &str = mapped.as_ref().map(|(n, _)| *n).unwrap_or(name);
    let args: &Value = mapped.as_ref().map(|(_, a)| a).unwrap_or(args);

    match name {
        "webrain_guide" => guide_from_args(args),
        "webrain_domain_memory" => {
            let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("status");
            let db = args
                .get("db")
                .and_then(|v| v.as_str())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(webrain_core::webmemory::WebMemory::default_path);
            let mem = match webrain_core::webmemory::WebMemory::open(&db) {
                Ok(m) => m,
                Err(e) => return err(e),
            };
            let domain = args.get("domain").and_then(|v| v.as_str());
            match op {
                "status" => match mem.status() {
                    Ok(v) => {
                        json!({"status": "ok", "op": "status", "db": db.display().to_string(), "counts": v})
                    }
                    Err(e) => err(e),
                },
                "coverage" => match domain {
                    Some(d) => match mem.coverage(d) {
                        Ok(v) => json!({"status": "ok", "op": "coverage", "report": v}),
                        Err(e) => err(e),
                    },
                    None => {
                        json!({"status": "error", "message": "domain required for op=coverage"})
                    }
                },
                "architecture" => match domain {
                    Some(d) => match mem.architecture(d) {
                        Ok(v) => json!({"status": "ok", "op": "architecture", "report": v}),
                        Err(e) => err(e),
                    },
                    None => {
                        json!({"status": "error", "message": "domain required for op=architecture"})
                    }
                },
                "search" => {
                    let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                    let top_k = args
                        .get("top_k")
                        .and_then(|v| v.as_i64())
                        .map(|n| n.clamp(1, 100) as usize)
                        .unwrap_or(10);
                    match mem.search(query, domain, top_k) {
                        Ok(hits) => {
                            json!({"status": "ok", "op": "search", "query": query, "count": hits.len(), "hits": hits})
                        }
                        Err(e) => err(e),
                    }
                }
                other => {
                    json!({"status": "error", "message": format!("unknown op {other:?}; use search|coverage|architecture|status")})
                }
            }
        }
        "webrain_add_init_script" => {
            let js = match js_arg(args, "js") {
                Ok(js) => js,
                Err(m) => return json!({"status": "error", "message": m}),
            };
            if js.is_empty() {
                return json!({"status": "error", "message": "js required"});
            }
            match backend.add_init_script(&js).await {
                Ok(_) => json!({"status": "ok"}),
                Err(e) => err(e),
            }
        }
        other => match family_of(other) {
            Some("page") => call_page(backend, name, args).await,
            Some("session") => call_session(backend, name, args).await,
            Some("crawl") => call_crawl(backend, name, args).await,
            Some("extract") => call_extract(backend, name, args).await,
            Some("interact") => call_interact(backend, name, args).await,
            Some("read") => call_read(backend, name, args).await,
            Some("batch") => call_batch(backend, name, args).await,
            Some("media") => call_media(backend, name, args).await,
            Some("toolbox") => call_toolbox(backend, name, args).await,
            _ => json!({"error": format!("Unknown tool: {name}")}),
        },
    }
}

/// Which family function owns a tool name.
///
/// Kept as data rather than as literal patterns inside `call_tool`, because a
/// 61-arm dispatcher has one silent failure mode: a tool that is advertised
/// and dispatched nowhere. `every_advertised_tool_has_a_family` holds that.
fn family_of(name: &str) -> Option<&'static str> {
    Some(match name {
        "webrain_navigate"
        | "webrain_eval"
        | "webrain_screenshot"
        | "webrain_click"
        | "webrain_type"
        | "webrain_click_coords"
        | "webrain_hold"
        | "webrain_drag"
        | "webrain_eval_in_frame"
        | "webrain_scroll"
        | "webrain_get_html" => "page",
        "webrain_cookies" | "webrain_setcookies" => "session",
        "webrain_spider" | "webrain_sitemap" | "webrain_snapshot" => "crawl",
        "webrain_pixel"
        | "webrain_extract_json"
        | "webrain_extract_regex"
        | "webrain_get_jsonld"
        | "webrain_table"
        | "webrain_scan"
        | "webrain_autoschema"
        | "webrain_fetch_http"
        | "webrain_bm25"
        | "webrain_fit"
        | "webrain_flatten"
        | "webrain_annotate" => "extract",
        "webrain_select" | "webrain_hover" | "webrain_check" | "webrain_dialog"
        | "webrain_wait" | "webrain_upload" => "interact",
        "webrain_validate_urls"
        | "webrain_pdf"
        | "webrain_tab"
        | "webrain_page_info"
        | "webrain_a11y"
        | "webrain_semantic_tree" => "read",
        "webrain_batch" => "batch",
        "webrain_download" | "webrain_watch" | "webrain_search" | "webrain_serp"
        | "webrain_nav" | "webrain_press" | "webrain_get_images" | "webrain_assets" => "media",
        "webrain_media"
        | "webrain_network"
        | "webrain_console"
        | "webrain_dismiss_overlays"
        | "webrain_vision_index"
        | "webrain_vision_retrieve"
        | "webrain_vision_ask"
        | "webrain_clean"
        | "webrain_profiles"
        | "webrain_captcha"
        | "webrain_login"
        | "webrain_domain_memory" => "toolbox",
        _ => return None,
    })
}

async fn call_page(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});
    match name {
        "webrain_navigate" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                return json!({"status": "error", "message": "url required"});
            }
            let opts = nav_opts(args);
            match backend.navigate_opts(url, &opts).await {
                Ok(s) => {
                    json!({"status": "ok", "url": s.url, "title": s.title, "text": s.text, "elements": s.elements, "links": s.links, "challenge": s.challenge, "crippled": s.crippled, "chrome_error": s.chrome_error, "capabilities": backend.caps()})
                }
                Err(e) => err(e),
            }
        }
        "webrain_eval" => {
            let js = match js_arg(args, "js") {
                Ok(js) => js,
                Err(m) => return json!({"status": "error", "message": m}),
            };
            match backend.evaluate(&js).await {
                Ok(v) => json!({"status": "ok", "result": v}),
                Err(e) => err(e),
            }
        }
        "webrain_screenshot" => {
            let full = args
                .get("full_page")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let dir = args
                .get("dir")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .unwrap_or("screenshots");
            let format = args
                .get("format")
                .and_then(|v| v.as_str())
                .unwrap_or("png")
                .to_string();
            let quality = args.get("quality").and_then(|v| v.as_u64()).unwrap_or(80) as u8;
            match backend.screenshot_fmt(full, &format, quality).await {
                Ok(img) => {
                    use base64::Engine;
                    // Extension must follow the bytes actually returned, or a client
                    // opening `shot.png` gets a jpeg. An unsupported request (avif)
                    // lands on png — report the format that was really used.
                    let (ext, mime) = match format.to_ascii_lowercase().as_str() {
                        "jpg" | "jpeg" => ("jpg", "image/jpeg"),
                        "webp" => ("webp", "image/webp"),
                        "avif" => ("avif", "image/avif"),
                        _ => ("png", "image/png"),
                    };
                    let mut out = json!({
                        "status": "ok",
                        "mime": mime,
                        "format": if mime == "image/jpeg" { "jpeg" } else { ext },
                        "requested_format": format,
                        "bytes": img.len(),
                    });
                    // also write to disk so any client can VIEW the file, not just
                    // round-trip the base64 blob (chat sessions decode awkwardly)
                    if let Err(e) = std::fs::create_dir_all(dir) {
                        out["path_error"] = json!(e.to_string());
                    } else {
                        let ts = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_millis())
                            .unwrap_or(0);
                        let path = format!("{dir}/shot_{ts}.{ext}");
                        match std::fs::write(&path, &img) {
                            Ok(()) => out["path"] = json!(path),
                            Err(e) => out["path_error"] = json!(e.to_string()),
                        }
                    }
                    // The base64 IS the MCP image block's payload (hoist_screenshot
                    // moves it into one), so it ships by default. `b64:false` trades
                    // the picture for the file on disk — the caller wants the file.
                    if args.get("b64").and_then(|v| v.as_bool()).unwrap_or(true) {
                        out["screenshot_b64"] =
                            json!(base64::engine::general_purpose::STANDARD.encode(&img));
                    }
                    out
                }
                Err(e) => err(e),
            }
        }
        "webrain_click" => {
            // A selector path exists because the index path cannot reach a widget
            // inside a *closed* shadow root: ELEMENTS_JS is page JS and page JS
            // cannot see one. `observe what=a11y` prints those nodes with a
            // root-scoped css_path; clicking that path goes through the pierced
            // CDP resolver + a trusted click at the element's centre.
            if let Some(sel) = args.get("selector").and_then(|v| v.as_str()) {
                if !sel.is_empty() {
                    return match backend.click_css(sel).await {
                        Ok(_) => json!({"status": "ok", "clicked_selector": sel}),
                        Err(e) => err(e),
                    };
                }
            }
            let idx = args.get("index").and_then(|v| v.as_i64()).unwrap_or(-1);
            if idx < 0 {
                return json!({"status": "error", "message": "index required"});
            }
            match backend.click(idx as usize).await {
                Ok(_) => json!({"status": "ok", "clicked": idx}),
                Err(e) => err(e),
            }
        }
        "webrain_type" => {
            let idx = args.get("index").and_then(|v| v.as_i64()).unwrap_or(-1);
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if idx < 0 {
                return json!({"status": "error", "message": "index required"});
            }
            // ms>0 → real per-key keydown/keyup at that cadence. Text captchas
            // and the reCAPTCHA/hCaptcha audio challenge rate-limit or flag an
            // instant paste (Input.insertText has no key events at all).
            let delay = args.get("ms").and_then(|v| v.as_u64()).unwrap_or(0);
            let r = if delay > 0 {
                backend.type_text_delayed(idx as usize, text, delay).await
            } else {
                backend.type_text(idx as usize, text).await
            };
            match r {
                Ok(_) => json!({"status": "ok", "typed": idx, "delay_ms": delay}),
                Err(e) => err(e),
            }
        }
        "webrain_click_coords" => {
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(-1);
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(-1);
            if x < 0 || y < 0 {
                return json!({"status": "error", "message": "x and y required"});
            }
            match backend.click_coords(x, y).await {
                Ok(_) => json!({"status": "ok", "x": x, "y": y}),
                Err(e) => err(e),
            }
        }
        "webrain_hold" => {
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(-1);
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(-1);
            if x < 0 || y < 0 {
                return json!({"status": "error", "message": "x and y required"});
            }
            let ms = args.get("ms").and_then(|v| v.as_u64()).unwrap_or(6000);
            match backend.hold_at(x, y, ms).await {
                Ok(_) => json!({"status": "ok", "hold": [x, y, ms]}),
                Err(e) => err(e),
            }
        }
        "webrain_drag" => {
            let x1 = args.get("x1").and_then(|v| v.as_i64()).unwrap_or(-1);
            let y1 = args.get("y1").and_then(|v| v.as_i64()).unwrap_or(-1);
            let x2 = args.get("x2").and_then(|v| v.as_i64()).unwrap_or(-1);
            let y2 = args.get("y2").and_then(|v| v.as_i64()).unwrap_or(-1);
            if x1 < 0 || y1 < 0 || x2 < 0 || y2 < 0 {
                return json!({"status": "error", "message": "x1,y1,x2,y2 required"});
            }
            let ms = args.get("ms").and_then(|v| v.as_u64()).unwrap_or(150);
            match backend.drag(x1, y1, x2, y2, ms).await {
                Ok(_) => json!({"status": "ok", "drag": [x1, y1, x2, y2], "hold_ms": ms}),
                Err(e) => err(e),
            }
        }
        "webrain_eval_in_frame" => {
            let url_contains = args
                .get("url_contains")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let js = match js_arg(args, "js") {
                Ok(js) => js,
                Err(m) => return json!({"status": "error", "message": m}),
            };
            if url_contains.is_empty() || js.is_empty() {
                return json!({"status": "error", "message": "url_contains and js required"});
            }
            match backend.eval_in_frame(&url_contains, &js).await {
                Ok(v) => json!({"status": "ok", "result": v}),
                Err(e) => err(e),
            }
        }
        "webrain_scroll" => {
            let dir = args
                .get("direction")
                .and_then(|v| v.as_str())
                .unwrap_or("down");
            let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(-1);
            let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(-1);
            let delta = args.get("delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
            // A point scrolls the element UNDER it — the only way to move a
            // captcha modal or an inner panel. No point → window scroll.
            let r = if x >= 0 && y >= 0 {
                let d = if delta != 0.0 {
                    delta
                } else if dir == "up" {
                    -500.0
                } else {
                    500.0
                };
                backend.scroll_at(dir, x, y, d).await
            } else {
                backend.scroll(dir).await
            };
            match r {
                Ok(_) => json!({"status": "ok", "direction": dir, "x": x, "y": y, "delta": delta}),
                Err(e) => err(e),
            }
        }
        "webrain_get_html" => match backend.get_html().await {
            Ok(html) => json!({"status": "ok", "html": html}),
            Err(e) => err(e),
        },
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}

async fn call_session(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});
    match name {
        // Cross-browser session migration: read/write cookies ON THE SESSION
        // BACKEND. Set + batch on the same connection so per-connection
        // isolated browsers (obscura stealth) share the imported session.
        // Chrome login -> webrain_cookies -> webrain_setcookies -> webrain_batch
        // (no cdp_urls) keeps cookies on one connection end-to-end.
        //
        // A caller-supplied `cdp_url` overrides that and uses THEIR browser. It
        // is resolved HERE rather than left to the session, and the reason is not
        // symmetry with the vault ops: these ops act on a CONNECTION. Reading the
        // endpoint from the argument is what lets the deepscrape gateway treat "a
        // caller brought their own browser" as verified. If this arm kept using
        // the session backend while the caller passed `cdp_url`, the gateway
        // would clear its own cross-tenant guard on the strength of an argument
        // the worker ignores -- and then read whatever Chrome the session held.
        "webrain_cookies" => match crate::caller_cdp(args) {
            Some(target) => match CdpBackend::connect_with_url(&target).await {
                Ok(b) => match b.cookies().await {
                    Ok(c) => json!({"status": "ok", "count": c.len(), "cookies": c}),
                    Err(e) => err(e),
                },
                Err(e) => json!({
                    "status": "error",
                    "message": format!("cannot connect to {target}: {e}")
                }),
            },
            None => match backend.cookies().await {
                Ok(c) => json!({"status": "ok", "count": c.len(), "cookies": c}),
                Err(e) => err(e),
            },
        },
        "webrain_setcookies" => {
            let cookies = args
                .get("cookies")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if cookies.is_empty() {
                return json!({"status": "error", "message": "cookies array required"});
            }
            match crate::caller_cdp(args) {
                Some(target) => match CdpBackend::connect_with_url(&target).await {
                    Ok(b) => match b.set_cookies(&cookies).await {
                        Ok(_) => {
                            let back = match b.cookies().await {
                                Ok(c) => c.len(),
                                Err(_) => 0,
                            };
                            json!({"status": "ok", "set": cookies.len(), "readback": back})
                        }
                        Err(e) => err(e),
                    },
                    Err(e) => json!({
                        "status": "error",
                        "message": format!("cannot connect to {target}: {e}")
                    }),
                },
                None => match backend.set_cookies(&cookies).await {
                    Ok(_) => {
                        let back = match backend.cookies().await {
                            Ok(c) => c.len(),
                            Err(_) => 0,
                        };
                        json!({"status": "ok", "set": cookies.len(), "readback": back})
                    }
                    Err(e) => err(e),
                },
            }
        }
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}

async fn call_crawl(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});
    match name {
        "webrain_spider" => {
            let seed = args.get("seed_url").and_then(|v| v.as_str()).unwrap_or("");
            if seed.is_empty() {
                return json!({"status": "error", "message": "seed_url required"});
            }
            let depth = args.get("max_depth").and_then(|v| v.as_i64()).unwrap_or(2) as usize;
            let pages = args.get("max_pages").and_then(|v| v.as_i64()).unwrap_or(20) as usize;
            let strategy = match args
                .get("strategy")
                .and_then(|v| v.as_str())
                .unwrap_or("bfs")
            {
                "dfs" => CrawlStrategy::Dfs,
                "bestfirst" => CrawlStrategy::BestFirst,
                _ => CrawlStrategy::Bfs,
            };
            let keywords: Vec<String> = args
                .get("keywords")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let same_domain = args
                .get("same_domain")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let discover_only = args
                .get("no_content")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let respect_robots = args
                .get("respect_robots")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let http_only = args
                .get("http_only")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let seed_from_sitemap = args
                .get("seed_from_sitemap")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let allowed: Vec<String> = args
                .get("allowed_domains")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let allow: Vec<String> = args
                .get("allow")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let deny: Vec<String> = args
                .get("deny")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let retry = args.get("retry").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
            let delay_ms = args
                .get("delay_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0) as u64;
            let crawl_timeout = args
                .get("crawl_timeout_secs")
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                .max(0) as u64;
            let autothrottle = args
                .get("autothrottle")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let autothrottle_max = args
                .get("autothrottle_max_ms")
                .and_then(|v| v.as_i64())
                .unwrap_or(30_000)
                .max(0) as u64;
            let crawldir = args
                .get("crawldir")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let checkpoint_every = args
                .get("checkpoint_every")
                .and_then(|v| v.as_i64())
                .unwrap_or(10)
                .max(1) as usize;
            let concurrency = crate::clamp_concurrency(
                args.get("concurrency")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(4)
                    .max(1) as usize,
            );
            let spider = SpiderEngine::new(depth, pages)
                .with_strategy(strategy)
                .with_same_domain(same_domain)
                .with_allowed_domains(allowed)
                .with_discover_only(discover_only)
                .with_respect_robots(respect_robots)
                .with_http_only(http_only)
                .with_sitemap_seed(seed_from_sitemap)
                .with_keywords(keywords)
                .with_filters(allow, deny)
                .with_retry(retry)
                .with_delay_ms(delay_ms)
                .with_crawl_timeout(crawl_timeout)
                .with_autothrottle(autothrottle, 200, autothrottle_max)
                .with_checkpoint(crawldir, checkpoint_every)
                .with_concurrency(concurrency)
                .with_nav_opts(nav_opts(args));
            let t0 = std::time::Instant::now();
            let mut results = spider.crawl(backend, seed).await;
            // Opt-in: a paginated or faceted site returns the same template page N
            // times and the caller pays a token for every copy. Keep the first page
            // per visible-text hash. An error page is never dropped — its text is
            // empty, so they would all collide.
            let mut duplicates = 0usize;
            if args
                .get("dedupe_pages")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                let mut seen = std::collections::HashSet::new();
                let before = results.len();
                results.retain(|r| {
                    let text = r.page.content.as_deref().unwrap_or("");
                    if r.page.error.is_some() || text.trim().is_empty() {
                        return true;
                    }
                    let mut h = std::collections::hash_map::DefaultHasher::new();
                    std::hash::Hash::hash(&text, &mut h);
                    seen.insert(std::hash::Hasher::finish(&h))
                });
                duplicates = before - results.len();
            }
            // ponytail: spider stats — consistent with the batch stats block; the
            // LLM sees elapsed + ok/err at a glance for a long crawl.
            let total_ms = t0.elapsed().as_millis() as u64;
            let ok = results.iter().filter(|r| r.page.error.is_none()).count();
            let errs = results.len() - ok;
            let page_ms: u64 = results.iter().map(|r| r.page.duration_ms).sum();
            json!({
                "status": "ok", "pages": results.len(), "results": results,
                "duplicates": duplicates,
                "stats": {"elapsed_ms": total_ms, "pages_ok": ok, "pages_err": errs, "page_ms_total": page_ms}
            })
        }
        "webrain_sitemap" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                return json!({"status": "error", "message": "url required"});
            }
            let proxy = args
                .get("proxy")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(webrain_core::proxy::next_proxy);
            match sitemap_urls(url, proxy.as_deref()) {
                Ok(v) => json!({"status": "ok", "result": v}),
                Err(e) => json!({"status": "error", "message": e.to_string()}),
            }
        }
        "webrain_snapshot" => match backend.snapshot().await {
            Ok(s) => {
                json!({"status": "ok", "url": s.url, "title": s.title, "text": s.text, "elements": s.elements, "links": s.links, "challenge": s.challenge, "crippled": s.crippled, "chrome_error": s.chrome_error})
            }
            Err(e) => err(e),
        },
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}

async fn call_extract(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});
    match name {
        "webrain_pixel" => {
            let tw = args
                .get("tile_width")
                .and_then(|v| v.as_f64())
                .unwrap_or(800.0);
            let th = args
                .get("tile_height")
                .and_then(|v| v.as_f64())
                .unwrap_or(800.0);
            let mt = args.get("max_tiles").and_then(|v| v.as_i64()).unwrap_or(16) as usize;
            let engine = TileEngine::new(tw, th, mt);
            let dir = args.get("tiles_dir").and_then(|v| v.as_str());
            match engine.tile(backend, dir).await {
                Ok(tiles) => json!({"status": "ok", "count": tiles.len(), "tiles": tiles}),
                Err(e) => err(e),
            }
        }
        "webrain_extract_json" => {
            let base = args
                .get("base_selector")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if base.is_empty() {
                return json!({"status": "error", "message": "base_selector required"});
            }
            let base_fields = args
                .get("base_fields")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let fields = args
                .get("fields")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let adaptive = args
                .get("adaptive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let js = if adaptive {
                build_adaptive_extract_js(base, &base_fields, &fields)
            } else {
                build_extract_js(base, &base_fields, &fields)
            };
            match backend.evaluate(&js).await {
                Ok(v) => {
                    let arr = parse_json_str(&v);
                    json!({"status": "ok", "count": arr_len(&arr), "data": arr})
                }
                Err(e) => err(e),
            }
        }
        "webrain_extract_regex" => {
            let custom = args
                .get("patterns")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            match regex_extract(backend, &custom).await {
                Ok(matches) => json!({"status": "ok", "matches": matches}),
                Err(e) => err(e),
            }
        }
        "webrain_get_jsonld" => {
            // browsemind extract_identity: JSON-LD / microdata, zero LLM.
            let js = r#"JSON.stringify(Array.from(document.querySelectorAll('script[type="application/ld+json"]')).map(s => { try { return JSON.parse(s.textContent); } catch(e) { return null; } }).filter(Boolean))"#;
            match backend.evaluate(js).await {
                Ok(v) => {
                    let arr = parse_json_str(&v);
                    json!({"status": "ok", "count": arr_len(&arr), "jsonld": arr})
                }
                Err(e) => err(e),
            }
        }
        "webrain_table" => {
            // browsemind extract_table: HTML tables -> JSON rows, zero LLM.
            // Header cells are object KEYS, so their whitespace is normalised:
            // HTML pads headers with `&nbsp;` (U+00A0) for layout, and
            // `trim()` only strips the ends, which left `row["First\u00a0appeared"]`
            // — a key that looks right and is not. Verified on a Wikipedia
            // infobox: the key is now `First appeared` (U+0020). Cell values are
            // left verbatim, because a <br> in a cell is real structure.
            let js = r#"JSON.stringify(Array.from(document.querySelectorAll('table')).slice(0,20).map(t => { const headers = Array.from(t.querySelectorAll('th')).map(th => th.textContent.replace(/\s+/g, ' ').trim()); const rows = Array.from(t.querySelectorAll('tr')).slice(headers.length ? 1 : 0).map(tr => Array.from(tr.querySelectorAll('td,th')).map(td => td.textContent.trim())); return headers.length ? rows.map(r => Object.fromEntries(headers.map((h,i) => [h, r[i] ?? null]))) : rows; }))"#;
            match backend.evaluate(js).await {
                Ok(v) => {
                    let arr = parse_json_str(&v);
                    json!({"status": "ok", "count": arr_len(&arr), "tables": arr})
                }
                Err(e) => err(e),
            }
        }
        "webrain_scan" => {
            // browsemind scan_full_page: auto-scroll to load infinite-scroll content.
            let max_scrolls = args
                .get("max_scrolls")
                .and_then(|v| v.as_i64())
                .unwrap_or(15) as usize;
            let js = format!(
                r#"(async()=>{{ const MAX={max_scrolls}; let last=document.body.scrollHeight; let done=0; for(let i=0;i<MAX;i++){{ window.scrollTo(0, document.body.scrollHeight); await new Promise(r=>setTimeout(r,250)); const h=document.body.scrollHeight; done=i+1; if(h===last && i>2) break; last=h; }} return JSON.stringify({{scrolls: done, height: document.body.scrollHeight}}); }})()"#
            );
            match backend.evaluate(&js).await {
                Ok(v) => {
                    let obj = parse_json_str(&v);
                    json!({"status": "ok", "result": obj})
                }
                Err(e) => err(e),
            }
        }
        "webrain_autoschema" => {
            // browsemind auto-detect CSS schema: find repeated container patterns.
            // Returns top candidate base-selectors for the LLM to build a full schema.
            let min_occurrences = args
                .get("min_occurrences")
                .and_then(|v| v.as_i64())
                .unwrap_or(3) as usize;
            let js = autoschema_js(min_occurrences);
            match backend.evaluate(&js).await {
                Ok(v) => {
                    let arr = parse_json_str(&v);
                    json!({"status": "ok", "candidates": arr})
                }
                Err(e) => err(e),
            }
        }
        "webrain_fetch_http" => {
            // ureq blocks for up to 30s and the DOM parse is pure CPU: both must
            // stay off the executor's threads or one slow URL stalls every other
            // tool call. lib.rs's short-circuit already does this.
            let a = args.clone();
            match tokio::task::spawn_blocking(move || fetch_http_from_args(&a)).await {
                Ok(v) => v,
                Err(e) => json!({"status": "error", "message": e.to_string()}),
            }
        }
        "webrain_bm25" => {
            // browsemind BM25 filter / crawl4ai ContentRelevanceFilter: rank text
            // items by query relevance, keep top_k. Zero LLM.
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let k = args.get("top_k").and_then(|v| v.as_i64()).unwrap_or(10) as usize;
            let items: Vec<String> = args
                .get("items")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let results = bm25_filter(&items, query, k);
            json!({"status": "ok", "count": results.len(), "results": results})
        }
        "webrain_fit" => {
            // crawl4ai PruningContentFilter borrow: dense page text, no query.
            match backend.evaluate(FIT_JS).await {
                Ok(v) => {
                    let text = v.as_str().unwrap_or("");
                    let words = text.split_whitespace().count();
                    json!({"status": "ok", "chars": text.chars().count(), "words": words, "text": text})
                }
                Err(e) => err(e),
            }
        }
        "webrain_flatten" => {
            // crawl4ai flatten_shadow_dom borrow: composed text incl. shadow DOM.
            match backend.evaluate(FLATTEN_JS).await {
                Ok(v) => {
                    let text = v.as_str().unwrap_or("");
                    let words = text.split_whitespace().count();
                    json!({"status": "ok", "chars": text.chars().count(), "words": words, "text": text})
                }
                Err(e) => err(e),
            }
        }
        "webrain_annotate" => {
            // agent-browser screenshot --annotate borrow: numbered overlay + legend.
            match backend.evaluate(ANNOTATE_JS).await {
                Ok(legend) => {
                    let shot = backend.screenshot(false).await;
                    let _ = backend
                        .evaluate("document.getElementById('__webrain_annotate__')?.remove()")
                        .await;
                    match shot {
                        Ok(png) => {
                            use base64::Engine;
                            json!({"status": "ok", "screenshot_b64": base64::engine::general_purpose::STANDARD.encode(png), "legend": legend})
                        }
                        Err(e) => err(e),
                    }
                }
                Err(e) => err(e),
            }
        }
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}

async fn call_interact(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});
    match name {
        "webrain_select" => {
            // agent-browser select_option borrow: value|text match, change event,
            // error lists available options on no-match.
            let idx = args.get("index").and_then(|v| v.as_i64()).unwrap_or(-1);
            let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("");
            if idx < 0 {
                return json!({"status": "error", "message": "index required"});
            }
            match backend.select_option(idx as usize, value).await {
                Ok(v) => json!({"status": "ok", "matched": v["matched"]}),
                Err(e) => err(e),
            }
        }
        "webrain_hover" => {
            let idx = args.get("index").and_then(|v| v.as_i64()).unwrap_or(-1);
            if idx < 0 {
                return json!({"status": "error", "message": "index required"});
            }
            match backend.hover(idx as usize).await {
                Ok(_) => json!({"status": "ok", "hovered": idx}),
                Err(e) => err(e),
            }
        }
        "webrain_check" => {
            // agent-browser check/uncheck borrow: click + verify + label retarget.
            let idx = args.get("index").and_then(|v| v.as_i64()).unwrap_or(-1);
            let want = args
                .get("checked")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            if idx < 0 {
                return json!({"status": "error", "message": "index required"});
            }
            match backend.set_checked(idx as usize, want).await {
                Ok(state) => json!({"status": "ok", "index": idx, "checked": state}),
                Err(e) => err(e),
            }
        }
        "webrain_dialog" => {
            // agent-browser handle_dialog borrow: unblocks a paused renderer.
            let accept = args
                .get("action")
                .and_then(|v| v.as_str())
                .map(|a| a != "dismiss")
                .unwrap_or(true);
            let prompt = args.get("prompt_text").and_then(|v| v.as_str());
            match backend.dialog(accept, prompt).await {
                Ok(_) => json!({"status": "ok"}),
                Err(e) => err(e),
            }
        }
        "webrain_wait" => {
            // agent-browser wait borrow: fixed ms or poll selector/text.
            if let Some(ms) = args.get("ms").and_then(|v| v.as_u64()) {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                return json!({"status": "ok", "satisfied": true});
            }
            let timeout = args
                .get("timeout_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(15000);
            let expr = if let Some(sel) = args.get("selector").and_then(|v| v.as_str()) {
                format!("document.querySelector({sel:?}) !== null", sel = sel)
            } else if let Some(t) = args.get("text").and_then(|v| v.as_str()) {
                format!(
                    "(document.body && document.body.innerText || '').includes({t:?})",
                    t = t
                )
            } else {
                return json!({"status": "error", "message": "one of ms, selector, text required"});
            };
            match backend.wait_for(&expr, timeout).await {
                Ok(satisfied) => json!({"status": "ok", "satisfied": satisfied}),
                Err(e) => err(e),
            }
        }
        "webrain_upload" => {
            // agent-browser upload borrow: DOM.setFileInputFiles via node id.
            let idx = args.get("index").and_then(|v| v.as_i64()).unwrap_or(-1);
            let files: Vec<String> = args
                .get("files")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if idx < 0 {
                return json!({"status": "error", "message": "index required"});
            }
            if files.is_empty() {
                return json!({"status": "error", "message": "files required"});
            }
            match backend.set_file_inputs(idx as usize, &files).await {
                Ok(_) => json!({"status": "ok", "uploaded": files.len()}),
                Err(e) => err(e),
            }
        }
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}

async fn call_read(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});
    match name {
        "webrain_validate_urls" => {
            // browsemind seed(from_links, validate=True): which URLs are alive vs dead.
            let urls: Vec<String> = args
                .get("urls")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if urls.is_empty() {
                return json!({"status": "error", "message": "urls required"});
            }
            let proxy = args
                .get("proxy")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(webrain_core::proxy::next_proxy);
            let results = validate_urls(&urls, proxy.as_deref());
            let alive = results
                .iter()
                .filter(|r| r.get("alive").and_then(|a| a.as_bool()).unwrap_or(false))
                .count();
            json!({"status": "ok", "alive": alive, "dead": results.len() - alive, "results": results})
        }
        "webrain_pdf" => match backend.pdf().await {
            Ok(bytes) => {
                use base64::Engine;
                json!({"status": "ok", "pdf_b64": base64::engine::general_purpose::STANDARD.encode(bytes)})
            }
            Err(e) => err(e),
        },
        "webrain_tab" => {
            let action = args
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("list");
            match action {
                "new" => {
                    let url = args
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("about:blank");
                    // A new tab when the browser can make one, the live tab when it
                    // cannot (a one-window or wedged Chrome answers -32000): asking
                    // for a tab should not fail the whole call, and the reply says
                    // which one it used.
                    match backend.tab_or_existing().await {
                        Ok((id, reused)) => match backend.activate_tab(&id).await {
                            Err(e) => err(e),
                            Ok(_) => match backend.navigate(url).await {
                                Ok(_) => json!({
                                    "status": "ok", "tab": id, "reused": reused,
                                    "tabs": backend.list_tabs().await.unwrap_or(json!([]))
                                }),
                                Err(e) => err(e),
                            },
                        },
                        Err(e) => err(e),
                    }
                }
                "switch" => {
                    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    match backend.activate_tab(id).await {
                        Ok(_) => json!({"status": "ok", "active": id}),
                        Err(e) => err(e),
                    }
                }
                "close" => {
                    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    match backend.close_tab(id).await {
                        Ok(_) => json!({"status": "ok", "closed": id}),
                        Err(e) => err(e),
                    }
                }
                _ => match backend.list_tabs().await {
                    Ok(t) => json!({"status": "ok", "tabs": t}),
                    Err(e) => err(e),
                },
            }
        }
        "webrain_page_info" => match backend.evaluate(PAGE_INFO_JS).await {
            Ok(v) => json!({"status": "ok", "page": v}),
            Err(e) => err(e),
        },
        "webrain_a11y" => match backend.a11y().await {
            Ok(nodes) => {
                let role = args.get("role").and_then(|v| v.as_str());
                let filter = args.get("filter").and_then(|v| v.as_str());
                let max = args
                    .get("max_nodes")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize);
                json!({"status": "ok", "nodes": filter_ax(&nodes, role, filter, max)})
            }
            Err(e) => err(e),
        },
        "webrain_semantic_tree" => match backend.a11y().await {
            // ponytail: the AX tree IS the semantic tree; render a compact text
            // view for the LLM (lightpanda LP.getSemanticTree style).
            Ok(nodes) => {
                let text = semantic_tree_text(&nodes);
                json!({"status": "ok", "text": text, "nodes": nodes})
            }
            Err(e) => err(e),
        },
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}

async fn call_batch(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    match name {
        "webrain_batch" => {
            let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("fetch");
            let urls: Vec<String> = args
                .get("urls")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if urls.is_empty() {
                return json!({"status": "error", "message": "urls required"});
            }
            // `.max(1)` BEFORE the cast: a negative i64 `as usize` wraps to a
            // colossal value, which would ask the worker pool for an unbounded
            // number of tabs (a client could send `concurrency: -1`).
            //
            // Screenshot / tile / pdf rows hold a full-frame bitmap per tab, so
            // the same box buys ~a third of them. The tab budget is what the op
            // actually costs, not what a text row costs.
            let cost_mult = match op {
                "screenshot" | "pixel" | "pdf" => 3,
                _ => 1,
            };
            // Before the work, so the delta answers "did THIS call get throttled".
            let throttle_t0 =
                webrain_core::engines::THROTTLE_EVENTS.load(std::sync::atomic::Ordering::Relaxed);
            let concurrency = crate::clamp_concurrency_for(
                args.get("concurrency")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(4)
                    .max(1) as usize,
                cost_mult,
            );
            let opts = nav_opts(args);
            // Wall clock for the whole call: `ms_total` sums per-URL times, so the
            // ratio ms_total/ms_wall is the only honest measure of how much of the
            // requested concurrency the engine actually delivered.
            let t_batch = std::time::Instant::now();

            // The per-backend op dispatcher — shared by the single-backend path and
            // the multi-backend round-robin so a fix here covers every caller.
            async fn run_batch(
                b: &CdpBackend,
                op: &str,
                urls: &[String],
                args: &Value,
                opts: &webrain_core::backends::cdp::NavOpts,
                concurrency: usize,
            ) -> Vec<BatchResult> {
                let base = args
                    .get("base_selector")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let base_fields = args
                    .get("base_fields")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let fields = args
                    .get("fields")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                // Only the browserless ops can take an exit per call; a browser op's
                // exit is the browser's (set at launch, or via cdp_urls). Omitted →
                // the engine rotates the WEBRAIN_PROXIES pool per request.
                let proxy = args
                    .get("proxy")
                    .and_then(|v| v.as_str())
                    .filter(|p| !p.is_empty())
                    .map(String::from);
                match op {
                    "extract" => {
                        batch_extract(b, urls, &base, &base_fields, &fields, concurrency, opts)
                            .await
                    }
                    // The browserless twin of `extract`: same schema, no tab. N
                    // static pages were N `webrain_scrape` calls before this.
                    "scrape" => {
                        batch_scrape(urls, &base, &fields, concurrency, proxy.as_deref()).await
                    }
                    // The batch twin of `webrain read`: HTTP markdown negotiation per
                    // URL, no tab. N docs pages were N `read` invocations before this.
                    "read" => {
                        let query = args
                            .get("query")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let top_k = args
                            .get("top_k")
                            .and_then(|v| v.as_i64())
                            .map(|n| n.max(0) as usize)
                            .unwrap_or(0);
                        // `--filter`/`--outline`/`--max` of the verb, per row: the
                        // caller should not have to re-implement the selection.
                        let read = webrain_core::read::ReadOpts {
                            outline_only: args
                                .get("outline")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false),
                            filter: args
                                .get("filter")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .map(String::from),
                            max_chars: args
                                .get("max_chars")
                                .and_then(|v| v.as_i64())
                                .map(|n| n.max(0) as usize)
                                .unwrap_or(0),
                            proxy: None,
                        };
                        batch_read(urls, concurrency, proxy.as_deref(), &query, top_k, &read).await
                    }
                    "interact" => {
                        let interaction = match js_arg(args, "interaction") {
                            Ok(js) => js,
                            Err(m) => return batch_js_err(&urls, m),
                        };
                        batch_interact(
                            b,
                            urls,
                            &interaction,
                            &base,
                            &base_fields,
                            &fields,
                            concurrency,
                            opts,
                        )
                        .await
                    }
                    "screenshot" => {
                        let dir = args
                            .get("dir")
                            .and_then(|v| v.as_str())
                            .unwrap_or("screenshots")
                            .to_string();
                        batch_screenshot(b, urls, &dir, concurrency, opts).await
                    }
                    "eval" => {
                        let js = match js_arg(args, "js") {
                            Ok(js) => js,
                            Err(m) => return batch_js_err(&urls, m),
                        };
                        if js.is_empty() {
                            return batch_js_err(&urls, "js required (op=eval)".to_string());
                        }
                        batch_eval(b, urls, &js, concurrency, opts).await
                    }
                    // `batch_markdown` existed in core but was UNREACHABLE from
                    // this tool: the op enum stopped at screenshot and every
                    // unknown op fell through to batch_fetch (3000-char innerText
                    // cap), so "op=markdown" silently returned plain text.
                    "markdown" => {
                        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
                        let top_k = args
                            .get("top_k")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                            .max(0) as usize;
                        batch_markdown(b, urls, query, top_k, concurrency, opts).await
                    }
                    // Selector discovery ACROSS urls: `op=extract` needs a
                    // base_selector, so this is the step that produces one. Same
                    // in-page heuristic as webrain_autoschema, one call per tab.
                    "autoschema" => {
                        let min = args
                            .get("min_occurrences")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(3)
                            .max(1) as usize;
                        batch_eval(b, urls, &autoschema_js(min), concurrency, opts).await
                    }
                    // Link/image/video/document/network inventory across N URLs.
                    // `max` caps each array; counts stay unbounded.
                    "assets" => {
                        let lim = args
                            .get("max")
                            .and_then(|v| v.as_i64())
                            .map(|n| n.clamp(1, 2000) as usize)
                            .unwrap_or(200);
                        batch_eval(b, urls, &asset_scan_js(lim), concurrency, opts).await
                    }
                    // All URLs each page actually requested (Resource Timing), no
                    // CDP arming and no response bodies. Composes across N URLs.
                    "network" => {
                        let lim = args
                            .get("max")
                            .and_then(|v| v.as_i64())
                            .map(|n| n.clamp(1, 5000) as usize)
                            .unwrap_or(1000);
                        batch_eval(b, urls, &network_scan_js(lim), concurrency, opts).await
                    }
                    _ => batch_fetch(b, urls, concurrency, opts).await,
                }
            }

            // Game-changer for per-proxy isolation: optional `cdp_urls` fan the
            // batch out across N CDP backends (each browser = own proxy/cookies/
            // fingerprint), round-robin by URL index. One call, N exit IPs.
            let cdp_urls: Vec<String> = args
                .get("cdp_urls")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            // Per-backend memory cap: N backends × `concurrency` tabs = N× tabs total,
            // which OOMs on huge jobs. `per_backend_concurrency` bounds tabs per browser
            // (default = concurrency, i.e. current behavior). ponytail: total tabs =
            // per_backend_concurrency × backends; raise it only when memory allows.
            let per_backend_concurrency = crate::clamp_concurrency_for(
                args.get("per_backend_concurrency")
                    .and_then(|v| v.as_i64())
                    .map(|n| n.max(1) as usize)
                    .unwrap_or(concurrency),
                cost_mult,
            );
            // Each listed backend opens its OWN pool, so the real tab count is
            // per_backend × backends. Bound that aggregate by the same box budget —
            // 10 backends at the default 4 would otherwise be 40 tabs, silent.
            let per_backend_concurrency = if cdp_urls.is_empty() {
                per_backend_concurrency
            } else {
                per_backend_concurrency
                    .min((crate::tab_budget_for(cost_mult) / cdp_urls.len()).max(1))
            };
            // per-URL wait selectors. NavOpts is per-call, so group the URLs by
            // their effective selector, dispatch once per group, then restore input
            // order. This wraps BOTH dispatch paths on purpose: it used to live
            // inside the no-`cdp_urls` branch, so `wait_selectors` was silently
            // ignored whenever `cdp_urls` was set — i.e. on every lightpanda run
            // (measured: plural 492 ms vs singular 12604 ms for the same URLs).
            let per_url_wait: Vec<Option<String>> = args
                .get("wait_selectors")
                .and_then(|v| v.as_array())
                .map(|a| {
                    (0..urls.len())
                        .map(|i| {
                            a.get(i)
                                .and_then(|x| x.as_str())
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
            if per_url_wait.iter().any(|w| w.is_some()) {
                for (i, w) in per_url_wait.iter().enumerate() {
                    let key = w.clone().unwrap_or_default();
                    match groups.iter_mut().find(|(k, _)| *k == key) {
                        Some((_, idxs)) => idxs.push(i),
                        None => groups.push((key, vec![i])),
                    }
                }
            } else {
                groups.push((String::new(), (0..urls.len()).collect()));
            }

            // One batch per `cdp_urls` backend, all CONCURRENTLY, results restored
            // to `sub` order. Connect failures land in `errs` (per call, not per URL).
            async fn fan_out(
                cdp_urls: &[String],
                op: &str,
                sub: &[String],
                args: &Value,
                opts: &webrain_core::backends::cdp::NavOpts,
                per_backend_concurrency: usize,
                errs: &mut Vec<BatchResult>,
            ) -> (Vec<BatchResult>, usize, Option<bool>) {
                // Connects are independent WebSocket handshakes: run them at the
                // same time. Serially, four backends cost four round-trips of wall
                // time before the first URL is even queued. Spawn order is kept,
                // so the round-robin mapping below is unchanged.
                let mut conns = Vec::with_capacity(cdp_urls.len());
                for u in cdp_urls {
                    let u = u.clone();
                    conns.push(tokio::spawn(async move {
                        let r = CdpBackend::connect_with_url(&u).await;
                        (u, r)
                    }));
                }
                let mut backends = Vec::new();
                for h in conns {
                    match h.await {
                        Ok((_, Ok(b))) => backends.push(b),
                        Ok((u, Err(e))) => errs.push(BatchResult {
                            url: u,
                            title: String::new(),
                            text: String::new(),
                            data: None,
                            error: Some(format!("cdp connect failed: {e}")),
                            ms: 0,
                        }),
                        Err(join) => errs.push(BatchResult {
                            url: String::new(),
                            title: String::new(),
                            text: String::new(),
                            data: None,
                            error: Some(format!("cdp connect task failed: {join}")),
                            ms: 0,
                        }),
                    }
                }
                let n = backends.len().max(1);
                // Round-robin: url[i] -> backend[i % n]. Backends run CONCURRENTLY:
                // the old sequential loop paid N connects for zero extra throughput.
                let mut slots: Vec<Option<Vec<BatchResult>>> =
                    (0..backends.len()).map(|_| None).collect();
                let mut handles = Vec::with_capacity(backends.len());
                for (bi, b) in backends.into_iter().enumerate() {
                    let share: Vec<String> = sub
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i % n == bi)
                        .map(|(_, u)| u.clone())
                        .collect();
                    if share.is_empty() {
                        continue;
                    }
                    let op = op.to_string();
                    let args = args.clone();
                    let opts = opts.clone();
                    handles.push((
                        bi,
                        tokio::spawn(async move {
                            let v =
                                run_batch(&b, &op, &share, &args, &opts, per_backend_concurrency)
                                    .await;
                            (v, b.batch_tabs().unwrap_or(0), b.single_target_known())
                        }),
                    ));
                }
                // Measured facts come back from the clones — the outer backend
                // never sees them, which is why `single_target` used to be null
                // and `effective_concurrency` an assumption on this path.
                // Tabs are SUMMED: `requested_concurrency` here is
                // per_backend_concurrency × backends, so a max would compare two
                // different quantities (measured 4 requested, 2 reported, for
                // two backends that each really held two tabs).
                let mut tabs = 0usize;
                let mut single: Option<bool> = None;
                for (bi, h) in handles {
                    if let Ok((v, t, s)) = h.await {
                        slots[bi] = Some(v);
                        tabs += t;
                        single = single.or(s);
                    }
                }
                // Round-robin decides placement (url[i] -> backend i % n, ascending
                // inside a backend), so the slots alone rebuild `sub` order.
                let mut iters: Vec<std::vec::IntoIter<BatchResult>> = slots
                    .into_iter()
                    .map(|s| s.unwrap_or_default().into_iter())
                    .collect();
                let mut out = Vec::with_capacity(sub.len());
                for (i, url) in sub.iter().enumerate() {
                    out.push(
                        iters
                            .get_mut(i % n)
                            .and_then(|it| it.next())
                            .unwrap_or_else(|| BatchResult {
                                url: url.clone(),
                                title: String::new(),
                                text: String::new(),
                                data: None,
                                error: Some("backend produced no result for this URL".to_string()),
                                ms: 0,
                            }),
                    );
                }
                (out, tabs, single)
            }

            let mut ordered: Vec<Option<BatchResult>> = (0..urls.len()).map(|_| None).collect();
            let mut connect_errs: Vec<BatchResult> = Vec::new();
            // Measured across groups: the widest fan-out this call achieved, and
            // whatever the engines said about their own tab capability.
            let mut tabs_seen = 0usize;
            let mut single_seen: Option<bool> = None;
            for (gi, (sel, idxs)) in groups.into_iter().enumerate() {
                let sub: Vec<String> = idxs.iter().map(|i| urls[*i].clone()).collect();
                let mut o = opts.clone();
                if !sel.is_empty() {
                    o.wait_selector = Some(sel);
                }
                let (got, tabs, single) = if cdp_urls.is_empty() {
                    let v = run_batch(backend, op, &sub, args, &o, concurrency).await;
                    (
                        v,
                        backend.batch_tabs().unwrap_or(0),
                        backend.single_target_known(),
                    )
                } else if gi == 0 {
                    fan_out(
                        &cdp_urls,
                        op,
                        &sub,
                        args,
                        &o,
                        per_backend_concurrency,
                        &mut connect_errs,
                    )
                    .await
                } else {
                    // A dead backend is a per-CALL fact; reporting it once per group
                    // would print the same connect error twice.
                    let mut discarded = Vec::new();
                    fan_out(
                        &cdp_urls,
                        op,
                        &sub,
                        args,
                        &o,
                        per_backend_concurrency,
                        &mut discarded,
                    )
                    .await
                };
                tabs_seen = tabs_seen.max(tabs);
                single_seen = single_seen.or(single);
                for r in got {
                    let slot = idxs
                        .iter()
                        .copied()
                        .find(|&i| ordered[i].is_none() && urls[i] == r.url);
                    if let Some(i) = slot {
                        ordered[i] = Some(r);
                    }
                }
            }
            let mut results: Vec<BatchResult> = connect_errs;
            results.extend(ordered.into_iter().flatten());
            let mut payload = json!({"status": "ok", "count": results.len(), "results": results});
            // ponytail: optional `output` path persists the batch payload to disk so a
            // temp-file GC (or a dropped MCP response) can't lose it between turns.
            // Default off — only write when the LLM asks, avoids surprise file writes.
            if let Some(path) = args.get("output").and_then(|v| v.as_str()) {
                if let Ok(json) = serde_json::to_string_pretty(&payload) {
                    if let Some(parent) = std::path::Path::new(path).parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::write(path, json) {
                        Ok(_) => payload["written_to"] = json!(path),
                        Err(e) => payload["write_error"] = json!(e.to_string()),
                    }
                }
            }
            // ponytail: batch stats — the LLM sees total/ok/errors + total ms at a
            // glance for a heavy run, instead of counting results rows itself.
            if let Some(res) = payload.get("results").and_then(|v| v.as_array()) {
                let ok = res
                    .iter()
                    .filter(|r| r.get("error").is_none() || r["error"].is_null())
                    .count();
                let errs = res
                    .iter()
                    .filter(|r| r.get("error").is_some() && !r["error"].is_null())
                    .count();
                let ms: u64 = res
                    .iter()
                    .filter_map(|r| r.get("ms").and_then(|v| v.as_u64()))
                    .sum();
                // lightpanda serve is single-target: its pool collapses to ONE tab
                // whatever `concurrency` asked for (its 2nd createTarget errors
                // TargetAlreadyLoaded). Report that instead of letting the caller
                // tune concurrency against an engine that ignores it.
                //
                // `effective_concurrency` is the number of tabs the batch REALLY
                // opened (`batch_tabs`), not the number it was asked for: a run whose
                // workers never got a tab used to report 4 while `ms_total` was 0.
                let single = single_seen;
                let asked = if cdp_urls.is_empty() {
                    // The pool never runs more workers than there are URLs.
                    concurrency.min(urls.len()).max(1)
                } else {
                    per_backend_concurrency * cdp_urls.len().max(1)
                };
                let effective = effective_tabs(tabs_seen, errs, single, asked);
                payload["stats"] = json!({
                    "total": res.len(),
                    "ok": ok,
                    "errors": errs,
                    "ms_total": ms,
                    "ms_wall": t_batch.elapsed().as_millis() as u64,
                    "requested_concurrency": concurrency,
                    "effective_concurrency": effective,
                    "single_target": single,
                    // What the machine allowed, which is not what was asked for
                    // when memory or cores are the binding constraint. The cost
                    // multiplier is part of the answer: a screenshot tab is not
                    // a text tab.
                    "tabs_allowed": crate::clamp_concurrency_for(concurrency, cost_mult),
                    "cost_multiplier": cost_mult,
                    "mem_available_mb": webrain_core::backends::cdp::memory_available_mb(),
                    "mem_total_mb": webrain_core::backends::cdp::memory_total_mb(),
                    // One-way valve: true means workers were halved mid-run
                    // because free memory crossed 12%, so a slow batch has a stated
                    // reason instead of looking like a slow site.
                    "throttled_by_memory": webrain_core::engines::THROTTLE_EVENTS
                        .load(std::sync::atomic::Ordering::Relaxed)
                        > throttle_t0,
                    "capabilities": backend.caps(),
                });
            }
            // Applied after the rows exist: a 1,000 row reply is the memory hog,
            // not the tabs. Truncated rows keep url/title/error.
            let (bytes_returned, rows_truncated) = apply_response_budget(&mut payload);
            if let Some(stats) = payload.get_mut("stats") {
                stats["bytes_returned"] = json!(bytes_returned);
                stats["rows_truncated"] = json!(rows_truncated);
                stats["budget_bytes"] = json!(response_budget());
            }
            payload
        }
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}

async fn call_media(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});
    match name {
        "webrain_download" => {
            // Combined download: single or batch. engine 'ytdlp' routes to the
            // installed yt-dlp binary (video/audio/HLS/playlists/features);
            // default 'http' keeps the streaming path (browsemind download_many).
            let mut urls: Vec<String> = args
                .get("urls")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            if urls.is_empty() {
                return json!({"status": "error", "message": "urls required"});
            }
            let dir = args
                .get("dir")
                .and_then(|v| v.as_str())
                .unwrap_or("downloads")
                .to_string();
            if args
                .get("engine")
                .and_then(|v| v.as_str())
                .unwrap_or("http")
                == "ytdlp"
            {
                // ponytail: single implementation lives in webrain_core::engines
                // (same one the no-browser path in lib.rs uses) — no duplication.
                let extra: Vec<String> = args
                    .get("args")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let audio_only = args
                    .get("audio_only")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let format = args
                    .get("format")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let proxy = args
                    .get("proxy")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .or_else(webrain_core::proxy::next_proxy);
                return webrain_core::engines::download_ytdlp(
                    &urls,
                    &dir,
                    audio_only,
                    format.as_deref(),
                    &extra,
                    proxy.as_deref(),
                );
            }
            // browsemind download_many: narrow to one file type (.mp4/.pdf/.js...).
            if let Some(ext) = args
                .get("filter_extension")
                .and_then(|v| v.as_str())
                .map(|s| s.trim_start_matches('.').to_lowercase())
                .filter(|e| !e.is_empty())
            {
                urls.retain(|u| {
                    let path = u.split('?').next().unwrap_or(u);
                    path.to_lowercase().ends_with(&format!(".{ext}"))
                });
                if urls.is_empty() {
                    return json!({"status": "error", "message": format!("no URLs match extension '.{ext}'")});
                }
            }
            let proxy = args
                .get("proxy")
                .and_then(|v| v.as_str())
                .map(String::from)
                .or_else(webrain_core::proxy::next_proxy);
            let results = download_files(&urls, &dir, proxy.as_deref());
            json!({"status": "ok", "count": results.len(), "results": results})
        }
        // Watch any video (URL or local file) — no browser needed, so it lives
        // in the shared helper both this dispatch AND the lib.rs no-browser
        // short-circuit call (one implementation, two callers).
        "webrain_watch" => watch_from_args(args),
        "webrain_search" => {
            let q = args.get("q").and_then(|v| v.as_str()).unwrap_or("");
            if q.is_empty() {
                return json!({"status": "error", "message": "q required"});
            }
            let engine = args
                .get("engine")
                .and_then(|v| v.as_str())
                .unwrap_or("duckduckgo");
            // Percent-encode the query — space→+ lets &/#/% inject params.
            let encoded: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
            let url = match engine {
                "bing" => format!("https://www.bing.com/search?q={encoded}"),
                "brave" => format!("https://search.brave.com/search?q={encoded}"),
                "google" => format!("https://www.google.com/search?q={encoded}"),
                _ => format!("https://html.duckduckgo.com/html/?q={encoded}"),
            };
            match backend.navigate(&url).await {
                Ok(s) => json!({
                    "status": "ok", "engine": engine, "url": s.url, "title": s.title,
                    "text": s.text.chars().take(3000).collect::<String>(), "elements": s.elements,
                    "links": s.links, "challenge": s.challenge, "crippled": s.crippled,
                    "chrome_error": s.chrome_error
                }),
                Err(e) => err(e),
            }
        }
        // Structured SERP API — shared helper used by BOTH this dispatch (browser
        // available for brave) and the lib.rs no-browser short-circuit (HTTP only).
        "webrain_serp" => serp_from_args(args, Some(backend)).await,
        "webrain_nav" => {
            let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("back");
            let js = match op {
                "forward" => "history.forward()",
                "reload" => "location.reload()",
                _ => "history.back()",
            };
            match backend.evaluate(js).await {
                Ok(_) => json!({"status": "ok", "op": op}),
                Err(e) => err(e),
            }
        }
        "webrain_press" => {
            let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("Enter");
            // Trusted CDP Input.dispatchKeyEvent (browsemind cdp_session_press),
            // JS fallback. Enter routes through the JS path to get form.submit().
            match backend.press(key).await {
                Ok(_) => json!({"status": "ok", "pressed": key}),
                Err(e) => err(e),
            }
        }
        "webrain_get_images" => {
            // Same scan the assets op runs, one array out of it: `<img srcset>` was
            // invisible to the old `document.images` version, and CSS
            // `background-image` (hero/banner/card art) was never returned at all —
            // "get images" quietly missed the images a page is mostly made of.
            match backend.evaluate(&asset_scan_js(200)).await {
                Ok(v) => {
                    let obj = parse_json_str(&v);
                    let arr = obj.get("images").cloned().unwrap_or_else(|| json!([]));
                    json!({"status": "ok", "count": arr_len(&arr), "images": arr})
                }
                Err(e) => err(e),
            }
        }
        "webrain_assets" => {
            let lim = args
                .get("max")
                .and_then(|v| v.as_i64())
                .map(|n| n.clamp(1, 2000) as usize)
                .unwrap_or(300);
            let only = args.get("only").and_then(|v| v.as_str()).unwrap_or("");
            match backend.evaluate(&asset_scan_js(lim)).await {
                Ok(v) => {
                    let mut obj = parse_json_str(&v);
                    if let Some(o) = obj.as_object_mut() {
                        match only {
                            "links" => {
                                o.remove("images");
                                o.remove("videos");
                                o.remove("documents");
                                o.remove("network");
                            }
                            "images" => {
                                o.remove("links");
                                o.remove("videos");
                                o.remove("documents");
                                o.remove("network");
                            }
                            "videos" => {
                                o.remove("links");
                                o.remove("images");
                                o.remove("documents");
                                o.remove("network");
                            }
                            "documents" => {
                                o.remove("links");
                                o.remove("images");
                                o.remove("videos");
                                o.remove("network");
                            }
                            "network" => {
                                o.remove("links");
                                o.remove("images");
                                o.remove("videos");
                                o.remove("documents");
                            }
                            _ => {}
                        }
                    }
                    json!({"status": "ok", "assets": obj})
                }
                Err(e) => err(e),
            }
        }
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}

async fn call_toolbox(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});
    match name {
        "webrain_media" => {
            let wait_ms = args.get("wait_ms").and_then(|v| v.as_i64()).unwrap_or(0);
            let url = args.get("url").and_then(|v| v.as_str()).map(String::from);
            if let Some(u) = url {
                if u.is_empty() {
                    return json!({"status": "error", "message": "url empty"});
                }
                // Tier 1 (browsemind): CDP Network capture of the load — works even
                // where the page clears/isolates the performance buffer.
                match backend.capture_media(Some(&u), wait_ms.max(0) as u64).await {
                    Ok(v) => {
                        let media = v.get("media").cloned().unwrap_or(Value::Null);
                        let total = v.get("total").and_then(|t| t.as_u64()).unwrap_or(0);
                        json!({"status": "ok", "mode": "network", "total_requests": total, "count": media.as_array().map(|a| a.len()).unwrap_or(0), "media": media})
                    }
                    Err(e) => err(e),
                }
            } else {
                // Tier 2: Performance API + media elements on the current page.
                let js = format!(
                    r#"(async()=>{{ if({wait_ms}>0) await new Promise(r=>setTimeout(r,{wait_ms})); const res=performance.getEntriesByType('resource').map(e=>e.name); const els=Array.from(document.querySelectorAll('video,audio,source')).map(e=>e.currentSrc||e.src||'').filter(Boolean); const all=[...res,...els].filter(u=>/^https?:/.test(u)); const media=/\.(m3u8|mpd|mp4|webm|mov|m4a|mp3)(\?|$)/i; const hint=/player|video|media|manifest|stream|\.json/i; const hits=[...new Set(all.filter(u=>media.test(u)||hint.test(u)))].slice(0,100); return JSON.stringify({{total:all.length,media:hits}}); }})()"#
                );
                match backend.evaluate(&js).await {
                    Ok(v) => {
                        let obj = parse_json_str(&v);
                        let media = obj.get("media").cloned().unwrap_or(Value::Null);
                        let total = obj.get("total").and_then(|t| t.as_u64()).unwrap_or(0);
                        json!({"status": "ok", "mode": "performance", "total_resources": total, "count": arr_len(&media), "media": media})
                    }
                    Err(e) => err(e),
                }
            }
        }
        "webrain_network" => {
            let op = args.get("op").and_then(|v| v.as_str()).unwrap_or("log");
            let filt = args.get("filter").and_then(|v| v.as_str()).unwrap_or("");
            let want_type = args.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let want_status = args.get("status").and_then(|v| v.as_i64());
            let limit = args
                .get("limit")
                .and_then(|v| v.as_i64())
                .unwrap_or(200)
                .clamp(1, 2048) as usize;
            // One filter for both read ops, so `log` and `requests` answer the
            // same question the same way.
            let keep = |e: &Value| {
                if !filt.is_empty()
                    && !e
                        .get("url")
                        .and_then(|u| u.as_str())
                        .unwrap_or("")
                        .contains(filt)
                {
                    return false;
                }
                if !want_type.is_empty()
                    && !e
                        .get("type")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .eq_ignore_ascii_case(want_type)
                {
                    return false;
                }
                if let Some(s) = want_status {
                    if e.get("status").and_then(|x| x.as_i64()) != Some(s) {
                        return false;
                    }
                }
                true
            };
            let pack = |op: &str, all: Vec<Value>| {
                let total = all.len();
                let rows: Vec<Value> = all.into_iter().filter(|e| keep(e)).take(limit).collect();
                let mut out = json!({"status": "ok", "op": op, "total": total, "count": rows.len(), "requests": rows});
                // A capture that saw nothing is either a page with no traffic or an
                // engine that does not emit Network events (lightpanda/obscura are
                // unverified). Say so — a bare `total: 0` reads as "this page made
                // no requests", which is the wrong conclusion.
                if op == "log" && total == 0 {
                    out["note"] = json!(
                        "no Network events captured — the page made no requests, or this engine does not implement the CDP Network domain (verified on Chrome only)"
                    );
                }
                out
            };
            match op {
                "request" => {
                    let id = args.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    if id.is_empty() {
                        return json!({"status": "error", "message": "id required (from op=log)"});
                    }
                    match backend.network_body(id).await {
                        Ok(mut v) => {
                            // An API-driven page answers in JSON; hand it back
                            // typed instead of making the agent parse the string.
                            if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
                                if let Ok(parsed) = serde_json::from_str::<Value>(text) {
                                    v["json"] = parsed;
                                }
                            }
                            v
                        }
                        Err(e) => err(e),
                    }
                }
                "clear" => {
                    backend.network_clear().await;
                    json!({"status": "ok", "op": "clear", "cleared": true})
                }
                "requests" => pack("requests", backend.network_entries().await),
                // `har` with a url/`wait_ms` captures first: the buffer lives on
                // the connection, so a one-shot CLI call (`log` then `har`) would
                // otherwise export an empty log from a fresh process.
                "har" => {
                    if args.get("url").is_some() || args.get("wait_ms").is_some() {
                        let wait_ms = args
                            .get("wait_ms")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0)
                            .max(0) as u64;
                        let url = args
                            .get("url")
                            .and_then(|v| v.as_str())
                            .filter(|u| !u.is_empty());
                        if let Err(e) = backend.network_log(url, wait_ms).await {
                            return err(e);
                        }
                    }
                    backend.network_har().await
                }
                _ => {
                    let wait_ms = args
                        .get("wait_ms")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0)
                        .max(0) as u64;
                    let url = args
                        .get("url")
                        .and_then(|v| v.as_str())
                        .filter(|u| !u.is_empty());
                    match backend.network_log(url, wait_ms).await {
                        Ok(all) => pack("log", all),
                        Err(e) => err(e),
                    }
                }
            }
        }
        "webrain_console" => {
            // The captor is normally already installed at document start (see
            // CONSOLE_CAPTOR_JS); re-sending it covers a tab attached after load
            // and engines without addScriptToEvaluateOnNewDocument. `splice(0)`
            // empties the SAME array the captor pushes into.
            let js = format!(
                "{};\nJSON.stringify((window.__wr_logs || []).splice(0))",
                webrain_core::browser::CONSOLE_CAPTOR_JS
            );
            match backend.evaluate(&js).await {
                Ok(v) => {
                    let arr = parse_json_str(&v);
                    json!({"status": "ok", "count": arr_len(&arr), "logs": arr})
                }
                Err(e) => err(e),
            }
        }
        "webrain_dismiss_overlays" => {
            let js = r#"
            (() => {
                const els = Array.from(document.querySelectorAll('body *')).filter(el => {
                    const s = getComputedStyle(el);
                    const r = el.getBoundingClientRect();
                    if (s.position !== 'fixed' && s.position !== 'sticky') return false;
                    if (parseInt(s.zIndex) < 50) return false;
                    if (r.width < 100 || r.height < 50) return false;
                    return true;
                });
                let removed = 0;
                for (const el of els) { el.remove(); removed++; }
                return removed;
            })()
            "#;
            match backend.evaluate(js).await {
                Ok(v) => json!({"status": "ok", "removed": v}),
                Err(e) => err(e),
            }
        }
        "webrain_vision_index" => {
            let tag = args
                .get("tag")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let tw = args
                .get("tile_width")
                .and_then(|v| v.as_f64())
                .unwrap_or(800.0);
            let th = args
                .get("tile_height")
                .and_then(|v| v.as_f64())
                .unwrap_or(800.0);
            let mt = args.get("max_tiles").and_then(|v| v.as_i64()).unwrap_or(8) as usize;
            match index_current_page(backend, &tag, tw, th, mt).await {
                Ok(v) => v,
                Err(e) => err(e),
            }
        }
        "webrain_vision_retrieve" => {
            let tag = args
                .get("tag")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_string();
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if query.is_empty() {
                return json!({"status": "error", "message": "query required"});
            }
            let k = args.get("k").and_then(|v| v.as_i64()).unwrap_or(5) as usize;
            match vision_retrieve(&tag, &query, k) {
                Ok(v) => v,
                Err(e) => err(e),
            }
        }
        "webrain_vision_ask" => {
            let prompt = args
                .get("prompt")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if prompt.is_empty() {
                return json!({"status": "error", "message": "prompt required"});
            }
            let clip = match (
                args.get("x").and_then(|v| v.as_f64()),
                args.get("y").and_then(|v| v.as_f64()),
                args.get("w").and_then(|v| v.as_f64()),
                args.get("h").and_then(|v| v.as_f64()),
            ) {
                (Some(x), Some(y), Some(w), Some(h)) if w > 0.0 && h > 0.0 => Some((x, y, w, h)),
                _ => None,
            };
            let scale = args
                .get("scale")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0)
                .max(0.5);
            let mut tiles: Vec<(f64, f64, f64, f64)> = Vec::new();
            if let Some(arr) = args.get("tiles").and_then(|v| v.as_array()) {
                for t in arr {
                    let (x, y, w, h) = (
                        t.get("x").and_then(|v| v.as_f64()),
                        t.get("y").and_then(|v| v.as_f64()),
                        t.get("w").and_then(|v| v.as_f64()),
                        t.get("h").and_then(|v| v.as_f64()),
                    );
                    if let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) {
                        if w > 0.0 && h > 0.0 {
                            tiles.push((x, y, w, h));
                        }
                    }
                }
            }
            let images: Vec<String> = args
                .get("images")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            match ask_viewport(backend, &prompt, clip, &tiles, &images, scale).await {
                Ok(ans) => json!({"status": "ok", "answer": ans}),
                Err(e) => err(e),
            }
        }
        "webrain_clean" => {
            let wt = args
                .get("word_threshold")
                .and_then(|v| v.as_i64())
                .unwrap_or(2) as usize;
            let es = args
                .get("exclude_social")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let js = build_clean_js(wt, es);
            match backend.evaluate(&js).await {
                Ok(v) => json!({"status": "ok", "text": v}),
                Err(e) => err(e),
            }
        }
        "webrain_profiles" => match webrain_core::vault::list() {
            Ok(profiles) => json!({"status": "ok", "count": profiles.len(), "profiles": profiles}),
            Err(e) => err(e),
        },
        "webrain_captcha" => {
            let sitekey = args.get("sitekey").and_then(|v| v.as_str()).unwrap_or("");
            let page_url = args.get("page_url").and_then(|v| v.as_str()).unwrap_or("");
            if sitekey.is_empty() || page_url.is_empty() {
                return json!({"status": "error", "message": "sitekey and page_url required"});
            }
            let Some((name, _)) = webrain_core::captcha::configured_provider() else {
                return json!({"status": "error", "message": "no CAPTCHA solver configured — set CAPSOLVER_API_KEY, WEBRAIN_2CAPTCHA_KEY or TWOCAPTCHA_API_KEY"});
            };
            let sitekey = sitekey.to_string();
            let page_url = page_url.to_string();
            let data_s = args
                .get("data_s")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let proxy = args.get("proxy").and_then(|v| v.as_str()).map(String::from);
            // The solver sleeps between polls (up to ~2.5 min) — never on the executor.
            match tokio::task::spawn_blocking(move || {
                webrain_core::captcha::solve_recaptcha2_auto(
                    &sitekey,
                    &page_url,
                    &data_s,
                    proxy.as_deref(),
                )
            })
            .await
            {
                Ok(Ok(token)) => json!({"status": "ok", "provider": name, "token": token}),
                Ok(Err(e)) => err(e),
                Err(e) => json!({"status": "error", "message": e.to_string()}),
            }
        }
        "webrain_login" => {
            let service = args
                .get("service")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let profile = args
                .get("profile")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if service.is_empty() || profile.is_empty() {
                return json!({"status": "error", "message": "service and profile required"});
            }
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let port = args.get("port").and_then(|v| v.as_u64()).map(|p| p as u16);
            // secret resolves in-process; the value never returns to the caller
            let cred = match webrain_core::vault::get(&service, &profile) {
                Ok(c) => c,
                Err(e) => return err(e),
            };
            // native auto-discovery login (same as `webrain login` CLI);
            // `port` targets a specific Chrome, else the session's shared backend
            let out = match port {
                Some(p) => {
                    match CdpBackend::connect_with_url(&format!("http://127.0.0.1:{p}")).await {
                        Ok(b) => {
                            webrain_core::login::run_login(
                                &b,
                                &cred.username,
                                &cred.password,
                                cred.totp.as_deref(),
                                url.as_deref(),
                            )
                            .await
                        }
                        Err(e) => return err(e),
                    }
                }
                None => {
                    webrain_core::login::run_login(
                        backend,
                        &cred.username,
                        &cred.password,
                        cred.totp.as_deref(),
                        url.as_deref(),
                    )
                    .await
                }
            };
            match out {
                Ok(v) => json!({"status": "ok", "result": v}),
                Err(e) => err(e),
            }
        }
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}

/// Shared `webrain_scrape` / `webrain_fetch_http` implementation.
///
/// Called by the `call_tool` arm AND by lib.rs's no-browser short-circuit. That
/// short-circuit used to call `http_fetch` directly, so the same tool answered
/// differently depending on which layer served it — and silently dropped
/// `base_selector` / `autoschema`, returning 3 KB of truncated HTML instead of
/// extracted rows.
pub fn fetch_http_from_args(args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});
    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
    if url.is_empty() {
        return json!({"status": "error", "message": "url required"});
    }
    // Autoschema first: it PROPOSES a selector, so it is the answer when the
    // caller has none — the case that otherwise sends them to the browser-only
    // `webrain_autoschema`.
    if args
        .get("autoschema")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        let min = args
            .get("min_occurrences")
            .and_then(|v| v.as_i64())
            .unwrap_or(3) as usize;
        let proxy = args
            .get("proxy")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(webrain_core::proxy::next_proxy);
        return match webrain_core::engines::http_fetch_autoschema(url, min, proxy.as_deref()) {
            Ok(v) => json!({"status": "ok", "result": v}),
            Err(e) => err(e),
        };
    }
    // A schema switches to the full-body path: `http_fetch` caps HTML at 3000
    // chars, so parsing its `text` would silently return no rows past the cap.
    let base_selector = args
        .get("base_selector")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if !base_selector.is_empty() {
        let fields = args
            .get("fields")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let proxy = args
            .get("proxy")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(webrain_core::proxy::next_proxy);
        return match webrain_core::engines::http_fetch_extract(
            url,
            base_selector,
            &fields,
            proxy.as_deref(),
        ) {
            Ok(v) => json!({"status": "ok", "result": v}),
            Err(e) => err(e),
        };
    }
    let proxy = args
        .get("proxy")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(webrain_core::proxy::next_proxy);
    match webrain_core::engines::http_fetch_capped_proxied(
        url,
        scrape_max_chars(args),
        proxy.as_deref(),
    ) {
        Ok(mut v) => {
            // spider-rs `smart` borrow: flag a JS shell so the caller upgrades to
            // the browser instead of scraping an empty page.
            let html = v["text"].as_str().unwrap_or("").to_string();
            v["visible_chars"] = json!(visible_text_len(&html));
            v["needs_js"] = json!(probe_needs_js(&html));
            json!({"status": "ok", "result": v})
        }
        Err(e) => err(e),
    }
}

/// Text cap for `webrain_scrape`: `full` = no cap, `max_chars` = explicit, else 3000.
fn scrape_max_chars(args: &Value) -> usize {
    if args.get("full").and_then(|v| v.as_bool()).unwrap_or(false) {
        return 0;
    }
    args.get("max_chars")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(3000)
}

/// Shared webrain_watch implementation (single | batch). Called from the tool
/// dispatch and from lib.rs's no-browser short-circuit, so the MCP server can
/// watch videos before any browser is up.
pub fn watch_from_args(args: &Value) -> Value {
    use webrain_core::video::{Detail, SttBackend, WatchOpts};
    let opts = WatchOpts {
        detail: Detail::parse(
            args.get("detail")
                .and_then(|v| v.as_str())
                .unwrap_or("balanced"),
        ),
        max_frames: args
            .get("max_frames")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize),
        resolution: args
            .get("resolution")
            .and_then(|v| v.as_u64())
            .unwrap_or(512) as u32,
        start: args.get("start").and_then(|v| v.as_f64()),
        end: args.get("end").and_then(|v| v.as_f64()),
        out_dir: args
            .get("out_dir")
            .and_then(|v| v.as_str())
            .map(String::from),
        no_whisper: args
            .get("no_whisper")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        stt_backend: SttBackend::parse(
            args.get("stt_backend")
                .and_then(|v| v.as_str())
                .unwrap_or("whisper"),
        ),
        vision: args
            .get("vision")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    };
    let single = args
        .get("source")
        .or_else(|| args.get("url"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let sources: Vec<String> = args
        .get("sources")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if let Some(src) = single {
        match webrain_core::video::watch(&src, &opts) {
            Ok(v) => json!({"status": "ok", "result": v}),
            Err(e) => json!({"status": "error", "message": e.to_string()}),
        }
    } else if !sources.is_empty() {
        let results = webrain_core::video::watch_batch(&sources, &opts);
        let ok = results.iter().filter(|r| r.get("error").is_none()).count();
        let errs = results.iter().filter(|r| r.get("error").is_some()).count();
        json!({
            "status": "ok",
            "count": results.len(),
            "ok": ok,
            "errors": errs,
            "results": results
        })
    } else {
        json!({"status": "error", "message": "source or sources required"})
    }
}

/// Structured SERP search — shared by the webrain_serp call_tool arm (browser
/// attached, so `brave` works) and the lib.rs no-browser short-circuit (HTTP
/// engines only). One implementation, two callers (watch_from_args pattern).
pub async fn serp_from_args(args: &Value, backend: Option<&CdpBackend>) -> Value {
    let q = args.get("q").and_then(|v| v.as_str()).unwrap_or("");
    if q.trim().is_empty() {
        return json!({"status": "error", "message": "q required"});
    }
    let opts = webrain_core::serp::SerpOpts {
        query: q.trim().to_string(),
        engine: args
            .get("engine")
            .and_then(|v| v.as_str())
            .unwrap_or("duckduckgo")
            .to_string(),
        limit: args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(10)
            .clamp(1, 100),
        page: args
            .get("page")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(0),
        safe: args.get("safe").and_then(|v| v.as_bool()).unwrap_or(false),
        region: args
            .get("region")
            .and_then(|v| v.as_str())
            .map(String::from),
        retries: args
            .get("retries")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(2)
            .min(5),
        fallback: args
            .get("fallback")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        proxy: args.get("proxy").and_then(|v| v.as_str()).map(String::from),
        http_only: args.get("http").and_then(|v| v.as_bool()).unwrap_or(false),
    };
    // google/brave need a browser — the lib.rs short-circuit routes HTTP engines
    // around the backend; here (call_tool) a backend may or may not be attached,
    // and lib.rs auto-launches a guest browser when none is. brave is
    // browser-optional once its keyed API is configured.
    let brave_needs_browser = opts.engine == "brave" && !webrain_core::serp::brave_api_ready();
    if (brave_needs_browser || opts.engine == "google") && backend.is_none() {
        return json!({"status": "error", "message": "engine 'google' (and 'brave' without BRAVE_API_KEY) requires a connected browser (set CDP_URL or start Chrome with --remote-debugging-port=9222; webrain_serp auto-launches a guest browser when none is attached); duckduckgo|bing|yahoo|baidu|yandex|auto work without one"});
    }
    match webrain_core::serp::serp_search(&opts, backend).await {
        Ok(r) => json!({
            "status": "ok",
            "query": r.query,
            "engine": r.engine,
            "source": r.source,
            "results": r.results,
            "per_engine": r.per_engine,
            "request_id": r.request_id,
            "ms": r.ms,
            "skipped": r.skipped,
        }),
        Err(e) => json!({"status": "error", "message": e.to_string()}),
    }
}

#[cfg(test)]
mod guide_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sections_parse_and_topics_resolve() {
        let (intro, sections) = guide_sections();
        assert!(intro.unwrap().contains("agent decision guide"));
        let titles: Vec<&str> = sections.iter().map(|(t, _)| *t).collect();
        assert!(titles.iter().any(|t| t.starts_with("20 TOOLS")));
        assert!(titles.iter().any(|t| t.starts_with("EXTRACTION MATRIX")));
        assert!(titles.iter().any(|t| t.starts_with("MULTI-AGENT")));

        // The env section is what lets a binary on a machine with no repo explain
        // its own knobs, so it must parse as a topic — and must not swallow the
        // section above it.
        assert!(titles.iter().any(|t| t.starts_with("ENVIRONMENT")));
        let env = sections
            .iter()
            .find(|(t, _)| t.starts_with("ENVIRONMENT"))
            .expect("environment section");
        for var in ["CDP_URL", "WEBRAIN_STEALTH_NOISE", "WEBRAIN_NO_STATE"] {
            assert!(env.1.contains(var), "env section names {var}");
        }
        let delegation = sections
            .iter()
            .find(|(t, _)| t.starts_with("MULTI-AGENT"))
            .expect("delegation section");
        assert!(!delegation.1.contains("WEBRAIN_STEALTH_NOISE"));
        assert!(
            guide_from_args(&json!({ "topic": "env" }))["guide"]
                .as_str()
                .unwrap()
                .contains("CDP_URL")
        );

        // A body line that OPENS with an all-caps word is parsed as a header and
        // splits its section in two — it happened with "NOT a challenge and NOT a
        // broken site.", which showed up as a topic of its own. Prose must never
        // start a line that way; indent it or start it with a bullet.
        assert!(
            !titles.iter().any(|t| t.starts_with("NOT ")),
            "prose was read as a guide header: {titles:?}"
        );
        let hydration = sections
            .iter()
            .find(|(t, _)| t.starts_with("RENDERED BUT NOT"))
            .expect("hydration section");
        assert!(
            hydration.1.contains("not a broken site"),
            "hydration section stays whole"
        );

        // A section must end at the next header, not swallow the rest of the file.
        let extraction = sections
            .iter()
            .find(|(t, _)| t.starts_with("EXTRACTION"))
            .expect("extraction section");
        assert!(extraction.1.contains("autoschema"));
        assert!(!extraction.1.contains("SUBAGENT CONTRACT"));

        // The default reply is the tool list + an index, and is cheaper than the file.
        let idx = guide_from_args(&json!({}));
        let text = idx["guide"].as_str().unwrap();
        assert!(text.len() < AGENT_GUIDE.len());
        assert!(text.contains("webrain_navigate"), "tool list stays inline");
        for t in &titles {
            assert!(text.contains(t), "index names every topic: {t}");
        }

        // Topic lookup is a case-insensitive substring, and `all` is the whole guide.
        let one = guide_from_args(&json!({ "topic": "Extract" }));
        assert!(one["guide"].as_str().unwrap().contains("EXTRACTION MATRIX"));
        assert_eq!(
            guide_from_args(&json!({ "topic": "all" }))["guide"],
            AGENT_GUIDE
        );

        // An unknown topic answers with the valid list instead of an empty string.
        let miss = guide_from_args(&json!({ "topic": "nope" }));
        assert!(miss["topics"].as_array().is_some_and(|a| !a.is_empty()));
    }

    /// This reads files on behalf of a remote model, so the name must never carry
    /// a path — `../` and separators have to be rejected before the join.
    #[test]
    fn skill_names_cannot_escape_the_dir() {
        let dir = std::env::temp_dir().join("webrain-skill-path-test");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("demo.md"), "# demo skill").unwrap();

        assert!(skill_path(&dir, "../etc/passwd").is_none());
        assert!(skill_path(&dir, "a/b").is_none());
        assert!(skill_path(&dir, "").is_none());
        assert!(skill_path(&dir, &"x".repeat(65)).is_none());
        assert!(skill_path(&dir, "missing").is_none());

        let p = skill_path(&dir, "demo").expect("demo.md resolves inside the dir");
        assert_eq!(read_skill(&p).unwrap(), "# demo skill");
        assert_eq!(list_skills(&dir), vec!["demo".to_string()]);
    }
}

#[cfg(test)]
mod surface_tests {
    use super::*;
    use serde_json::json;

    fn fold(name: &str, args: Value) -> (&'static str, Value) {
        map_surface(name, &args).expect("should fold")
    }

    #[test]
    fn folds_observe_selectors() {
        for (what, legacy) in [
            ("state", "webrain_snapshot"),
            ("a11y", "webrain_a11y"),
            ("semantic", "webrain_semantic_tree"),
            ("html", "webrain_get_html"),
            ("images", "webrain_get_images"),
            ("console", "webrain_console"),
            ("flatten", "webrain_flatten"),
            ("fit", "webrain_fit"),
            ("clean", "webrain_clean"),
            ("screenshot", "webrain_screenshot"),
            ("pixel", "webrain_pixel"),
            ("page_info", "webrain_page_info"),
            ("annotate", "webrain_annotate"),
            ("media", "webrain_media"),
            ("assets", "webrain_assets"),
        ] {
            let (n, a) = fold("webrain_observe", json!({"what": what}));
            assert_eq!(n, legacy, "observe {what}");
            assert!(a.get("what").is_none(), "selector dropped");
        }
    }

    #[test]
    fn folds_interact_actions() {
        let (n, a) = fold("webrain_interact", json!({"action": "click", "index": 0}));
        assert_eq!(n, "webrain_click");
        assert_eq!(a["index"], 0);
        assert!(a.get("action").is_none());
        // nav renames its param to the legacy arm's `op` key.
        let (n, a) = fold(
            "webrain_interact",
            json!({"action": "nav", "nav": "forward"}),
        );
        assert_eq!(n, "webrain_nav");
        assert_eq!(a["op"], "forward");
    }

    #[test]
    fn folds_session_captcha_op() {
        let (name, args) = fold(
            "webrain_session",
            json!({"op": "captcha", "sitekey": "sk", "page_url": "https://x/sorry"}),
        );
        assert_eq!(
            name, "webrain_captcha",
            "op=captcha must fold to its own executor"
        );
        assert_eq!(args["sitekey"], "sk");
        assert!(
            args.get("op").is_none(),
            "op must be dropped before dispatch"
        );
    }

    #[test]
    fn folds_extract_and_crawl_modes() {
        assert_eq!(
            fold("webrain_extract", json!({"mode": "schema"})).0,
            "webrain_extract_json"
        );
        assert_eq!(
            fold("webrain_extract", json!({"mode": "autoschema"})).0,
            "webrain_autoschema"
        );
        assert_eq!(
            fold("webrain_crawl", json!({"mode": "spider"})).0,
            "webrain_spider"
        );
        assert_eq!(
            fold("webrain_crawl", json!({"mode": "validate"})).0,
            "webrain_validate_urls"
        );
    }

    /// The three seed spellings are advertised flat while each mode reads one of
    /// them, so the fold has to hand every mode the field it wants. Measured
    /// against the shipped binary: `{"mode":"spider","url":...}` was answered
    /// `seed_url required`, and `{"mode":"validate","url":...}` `urls required`.
    #[test]
    fn crawl_seed_spelling_does_not_depend_on_the_mode() {
        let (tool, args) = fold(
            "webrain_crawl",
            json!({"mode": "spider", "url": "https://e.com"}),
        );
        assert_eq!(tool, "webrain_spider");
        assert_eq!(args["seed_url"], json!("https://e.com"));

        let (tool, args) = fold(
            "webrain_crawl",
            json!({"mode": "validate", "url": "https://e.com"}),
        );
        assert_eq!(tool, "webrain_validate_urls");
        assert_eq!(args["urls"], json!(["https://e.com"]));

        let (_, args) = fold(
            "webrain_crawl",
            json!({"mode": "sitemap", "seed_url": "https://e.com"}),
        );
        assert_eq!(args["url"], json!("https://e.com"));

        let (_, args) = fold(
            "webrain_crawl",
            json!({"mode": "scan", "urls": ["https://e.com", "https://f.com"]}),
        );
        assert_eq!(args["url"], json!("https://e.com"));

        // A caller that already names the field correctly is left alone.
        let (_, args) = fold(
            "webrain_crawl",
            json!({"mode": "spider", "seed_url": "https://given.com", "urls": ["https://kept.com"]}),
        );
        assert_eq!(args["seed_url"], json!("https://given.com"));
        assert_eq!(args["urls"], json!(["https://kept.com"]));
    }

    #[test]
    fn folds_session_and_pdf_ops() {
        assert_eq!(
            fold("webrain_session", json!({"op": "open"})).0,
            "webrain_open_session"
        );
        assert_eq!(
            fold("webrain_session", json!({"op": "list"})).0,
            "webrain_list_sessions"
        );
        assert_eq!(
            fold("webrain_session", json!({"op": "cookies"})).0,
            "webrain_cookies"
        );
        assert_eq!(
            fold("webrain_pdf", json!({"op": "extract"})).0,
            "webrain_pdf_extract"
        );
        assert_eq!(
            fold("webrain_pdf", json!({"op": "render"})).0,
            "webrain_pdf_render"
        );
        assert_eq!(
            fold("webrain_vision", json!({"op": "retrieve"})).0,
            "webrain_vision_retrieve"
        );
    }

    #[test]
    fn single_tool_names_pass_through() {
        assert!(map_surface("webrain_scrape", &json!({"url": "x"})).is_some());
        assert!(map_surface("webrain_navigate", &json!({})).is_none());
        assert!(map_surface("webrain_guide", &json!({})).is_none());
        assert!(map_surface("webrain_observe", &json!({})).is_none()); // missing selector
    }

    /// Every tool the server advertises either reaches a family or is answered
    /// earlier by `lib.rs`. The one failure a dispatcher this wide can hide is a
    /// name that is listed, documented and dead — `family_of` is the table that
    /// says which family owns what, so drift shows up here and nowhere else.
    #[test]
    fn every_advertised_tool_is_dispatched_somewhere() {
        // Answered before `call_tool` is reached: the session and pdf ops that
        // `lib.rs` intercepts on their folded names, the two arms still in
        // `call_tool` itself, and the intent umbrellas, which need a selector to
        // fold and are refused by `map_surface` when called bare.
        const HANDLED_EARLIER: &[&str] = &[
            "webrain_guide",
            "webrain_add_init_script",
            "webrain_stats",
            "webrain_observe",
            "webrain_interact",
            "webrain_extract",
            "webrain_crawl",
            "webrain_pdf",
            "webrain_session",
            "webrain_vision",
            "webrain_scrape",
            "webrain_open_session",
            "webrain_close_session",
            "webrain_list_sessions",
            "webrain_pdf_extract",
            "webrain_pdf_render",
            "webrain_pdf_images",
            "webrain_save_state",
            "webrain_restore_state",
            "webrain_close_launch",
        ];
        let mut orphans = Vec::new();
        for t in list_tools() {
            let Some(advertised) = t["name"].as_str() else {
                continue;
            };
            let folded = map_surface(advertised, &json!({}))
                .map(|(n, _)| n)
                .unwrap_or(advertised);
            if HANDLED_EARLIER.contains(&folded) {
                continue;
            }
            if family_of(folded).is_none() {
                orphans.push(advertised.to_string());
            }
        }
        assert!(
            orphans.is_empty(),
            "advertised but dispatched nowhere: {orphans:?}"
        );
    }
}

#[cfg(test)]
mod js_arg_tests {
    use super::js_arg;
    use serde_json::json;

    /// A literal must stay literal (including one that merely LOOKS like a path),
    /// and a real file must be read — the whole point is escaping never happens twice.
    #[test]
    fn js_arg_reads_a_file_only_when_it_exists() {
        assert_eq!(js_arg(&json!({"js": "1+1"}), "js").unwrap(), "1+1");
        assert_eq!(
            js_arg(&json!({"js": "@no/such/file.js"}), "js").unwrap(),
            "@no/such/file.js",
            "a missing @param is JS that fails loudly, not a silent empty script"
        );

        let path = std::env::temp_dir().join("webrain_js_arg_test.js");
        std::fs::write(&path, "document.title").unwrap();
        let arg = json!({ "js": format!("@{}", path.display()) });
        assert_eq!(js_arg(&arg, "js").unwrap(), "document.title");
    }
}
