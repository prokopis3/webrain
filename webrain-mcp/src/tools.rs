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
    batch_interact, batch_screenshot, bm25_filter, build_adaptive_extract_js, build_clean_js,
    build_extract_js, download_files, http_fetch, regex_extract, sitemap_urls, validate_urls,
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

17 TOOLS — match the request to a boundary:
  webrain_navigate   go to a URL (THE entry point; read `challenge` every time)
  webrain_observe    read the CURRENT page (what: state|a11y|semantic|html|images|
                     console|flatten|fit|clean|screenshot|pixel|page_info|annotate|media)
  webrain_interact   drive the page (action: click|click_coords|type|press|scroll|
                     nav|tab|select|hover|check|dialog|wait|upload|dismiss_overlays|
                     add_init_script)
  webrain_extract    structured data from the page (mode: schema|regex|jsonld|table|
                     autoschema|bm25)
  webrain_scrape     fast no-browser GET of one URL (static pages only)
  webrain_batch      same op across many URLs in parallel tabs (op: fetch|extract|
                     interact|eval|screenshot) + cdp_urls per-proxy fan-out
  webrain_crawl      site traversal (mode: spider|sitemap|scan|validate)
  webrain_search     web search (duckduckgo|google|bing|brave)
  webrain_serp       STRUCTURED SERP JSON (position/title/url/domain/snippet) — any
                     engine; duckduckgo|bing|auto are pure HTTP (no browser),
                     google|brave render in the connected CDP engine (auto-launched
                     guest Chrome); relevance-filtered + deduped + paginated + falls back
  webrain_pdf        pdf work (op: page|extract|render|images)
  webrain_download   files/media (engine: http|ytdlp)
  webrain_watch      video → transcript + frames
  webrain_session    browser/auth/session state (op: open|close|list|cookies|
                     setcookies|save_state|restore_state|profiles|login|close_launch)
  webrain_vision     screenshot-tile vision index (op: index|retrieve)
  webrain_eval       arbitrary JS (the escape hatch)
  webrain_eval_in_frame  run JS inside a cross-origin iframe (isolated world) →
                     exact reCAPTCHA/hCaptcha grid + verify geometry
  webrain_guide      this guide

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
    * Non-interactive Turnstile / basic bot detection may pass with obscura --stealth.
      VERIFIED (2026-08-07): `obscura fetch <url> --stealth --wait-until load --wait 30`
      passes scrapingcourse's `/login/cf-turnstile` — the Turnstile widget loads,
      solves (token populates `[name="cf-turnstile-response"]`), and the form is ready.
      From webrain: `webrain_navigate(url, disable_resources=true, block_trackers=true,
      network_idle=true, wait_selector="[name='cf-turnstile-response']", wait_timeout_secs=30)`
      achieves the same (Turnstile typically solves in 5-15s).
- Never report success on a challenge/login/consent page — verify the target
  content before returning results.
- Need screenshots/rendering? Real Chrome. Fast no-challenge scraping? obscura.
  Static HTML, no JS/auth? webrain_scrape.

EXTRACTION MATRIX
- structured list, schema known    -> extract(mode=schema, base_selector, fields)
- schema unknown                   -> extract(mode=autoschema) + eval, then mode=schema
- paginated pages 1..N / many URLs -> webrain_batch(op=extract, urls, base_selector, fields)
- URLs unknown (discover pages)    -> eval: links whose path = current + '/<N>'
                                      + next/prev labels (NO hardcoded classes), derive
                                      range, then webrain_batch
- whole site                       -> crawl(mode=spider, seed_url)
- emails/phones/prices/patterns    -> extract(mode=regex)
- JSON-LD / microdata              -> extract(mode=jsonld)
- tables                           -> extract(mode=table)
- infinite scroll / load-more      -> crawl(mode=scan) then extract; or interact(scroll)
- search                           -> webrain_search (navigate to results page)
- typed SERP results                -> webrain_serp (structured JSON; engine:
                                      duckduckgo|bing|google|brave|auto)
- relevance filter                 -> extract(mode=bm25)
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
"#;

/// Consolidated MCP surface — 17 intent-based tools (firecrawl-style), each
/// with a `what`/`action`/`op`/`mode` selector + when-to-use guidance so the
/// LLM picks the right boundary. `call_tool` routes each call to the legacy
/// per-primitive executor via `map_surface()` (legacy names still dispatch).
pub fn list_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "webrain_guide",
            "description": "Agent decision guide: browser selection (real Chrome vs obscura vs lightpanda vs webrain_scrape), challenge bypass, extraction matrix, delegation doctrine. Call FIRST when unsure which webrain tool to use.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "webrain_eval",
            "description": "Run arbitrary JavaScript in the current page and return the JSON result. The escape hatch for anything a tool doesn't cover (probe DOM, read scripts, call APIs).",
            "inputSchema": {"type": "object", "properties": {
                "js": {"type": "string", "description": "JS expression. Return a JSON-serializable value (JSON.stringify(...))."}
            }, "required": ["js"]}
        }),
        json!({
            "name": "webrain_eval_in_frame",
            "description": "Run JS inside a cross-origin iframe (isolated world). url_contains selects the iframe by src substring; js runs in that frame and returns the JSON result.",
            "inputSchema": {"type": "object", "properties": {
                "url_contains": {"type": "string", "description": "Substring of the target iframe's src to select it"},
                "js": {"type": "string", "description": "JS expression to run inside the iframe"}
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
            "description": "Read the CURRENT page without navigating. Pick `what` (required): state (page state), a11y (accessibility tree, role/filter/max_nodes), semantic (text tree), html (raw HTML — LAST RESORT), images, console (page errors), flatten (Shadow-DOM text), fit (dense content), clean (boiled text), screenshot (base64+file, full_page/dir), pixel (vision tiles), page_info (scroll/viewport), annotate (numbered-box overlay), media (media URLs, url/wait_ms).",
            "inputSchema": {"type": "object", "properties": {
                "what": {"type": "string", "enum": ["state","a11y","semantic","html","images","console","flatten","fit","clean","screenshot","pixel","page_info","annotate","media"]},
                "role": {"type": "string", "description": "a11y: ARIA role filter"},
                "filter": {"type": "string", "description": "a11y: name/value/css substring filter"},
                "max_nodes": {"type": "integer", "description": "a11y: node cap"},
                "selector": {"type": "string", "description": "html: CSS selector to narrow"},
                "full_page": {"type": "boolean", "default": false, "description": "screenshot: full scrollable page"},
                "dir": {"type": "string", "default": "screenshots", "description": "screenshot: output dir"},
                "tile_width": {"type": "number", "default": 800, "description": "pixel: tile width px"},
                "tile_height": {"type": "number", "default": 800, "description": "pixel: tile height px"},
                "max_tiles": {"type": "integer", "default": 16, "description": "pixel: max tiles"},
                "word_threshold": {"type": "integer", "default": 2, "description": "clean: min word length"},
                "exclude_social": {"type": "boolean", "default": true, "description": "clean: drop social links"},
                "url": {"type": "string", "description": "media: URL to load and capture network requests"},
                "wait_ms": {"type": "integer", "default": 0, "description": "media: keep capturing after load"}
            }, "required": ["what"]}
        }),
        json!({
            "name": "webrain_interact",
            "description": "Drive the page. Pick `action` (required): click, click_coords (x,y), drag (x1,y1 → x2,y2: trusted slider/drag CAPTCHAs, crosses cross-origin iframes), type (index,text), press (key), scroll (direction), nav (back|forward|reload), tab (new|switch|close|list + url/id), select (index,value), hover (index), check (index,checked), dialog (accept|dismiss + prompt_text), wait (ms|selector|text + timeout_ms), upload (index,files[]), dismiss_overlays, add_init_script (js). Element indices map to the elements[] from navigate/observe.",
            "inputSchema": {"type": "object", "properties": {
                "action": {"type": "string", "enum": ["click","click_coords","drag","type","press","scroll","nav","tab","select","hover","check","dialog","wait","upload","dismiss_overlays","add_init_script"]},
                "index": {"type": "integer", "description": "Element index from navigate/observe"},
                "text": {"type": "string", "description": "type: text to type; wait: text substring to wait for"},
                "key": {"type": "string", "description": "press: Enter|Tab|Escape|Backspace|ArrowDown|..."},
                "direction": {"type": "string", "enum": ["up","down"], "default": "down", "description": "scroll"},
                "x": {"type": "number", "description": "click_coords: viewport x"},
                "y": {"type": "number", "description": "click_coords: viewport y"},
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
                "ms": {"type": "integer", "description": "wait: fixed delay ms"},
                "selector": {"type": "string", "description": "wait: CSS selector to poll for"},
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
                "url": {"type": "string"}
            }, "required": ["url"]}
        }),
        json!({
            "name": "webrain_batch",
            "description": "Run one op across MANY URLs in parallel tabs. op: fetch (visible text) | extract (CSS schema: base_selector+fields) | interact (async JS per tab) | eval (custom JS extractor) | screenshot. concurrency bounds in-flight tabs; cdp_urls round-robins across N browsers (per-proxy isolation). The workhorse for at-scale scraping.",
            "inputSchema": {"type": "object", "properties": {
                "op": {"type": "string", "enum": ["fetch","extract","interact","eval","screenshot"]},
                "urls": {"type": "array", "items": {"type": "string"}},
                "cdp_urls": {"type": "array", "items": {"type": "string"}, "description": "Round-robin URLs across these CDP backends (per-proxy isolation)"},
                "js": {"type": "string", "description": "eval: JS extractor to run in each tab (required when op=eval)"},
                "interaction": {"type": "string", "description": "interact: async JS to run in each tab"},
                "base_selector": {"type": "string", "description": "extract/interact: repeated-item selector"},
                "fields": {"type": "array", "items": {"type": "object"}, "description": "extract/interact: schema"},
                "concurrency": {"type": "integer", "default": 4},
                "per_backend_concurrency": {"type": "integer", "default": 4, "description": "tabs per backend when cdp_urls set"},
                "dir": {"type": "string", "default": "screenshots", "description": "screenshot: output dir"},
                "output": {"type": "string", "description": "Persist the full payload to this file path"},
                "disable_resources": {"type": "boolean", "default": false},
                "block_trackers": {"type": "boolean", "default": false},
                "network_idle": {"type": "boolean", "default": false},
                "wait_selector": {"type": "string"},
                "wait_selector_state": {"type": "string", "enum": ["attached","visible","hidden","detached"], "default": "visible"},
                "wait_timeout_secs": {"type": "integer", "default": 20, "description": "Max seconds to wait for page load + conditions per URL"}
            }, "required": ["op", "urls"]}
        }),
        json!({
            "name": "webrain_crawl",
            "description": "Site traversal. Pick `mode` (required): spider (BFS/DFS/best-first whole-site crawl: seed_url + depth/pages/allow/deny/autothrottle/checkpoint), sitemap (discover URLs from a site's sitemap: url), scan (auto-scroll infinite-scroll: max_scrolls), validate (alive/dead URL probe: urls[]).",
            "inputSchema": {"type": "object", "properties": {
                "mode": {"type": "string", "enum": ["spider","sitemap","scan","validate"]},
                "seed_url": {"type": "string", "description": "spider: starting URL"},
                "max_depth": {"type": "integer", "default": 2},
                "max_pages": {"type": "integer", "default": 20},
                "strategy": {"type": "string", "enum": ["bfs","dfs","bestfirst"], "default": "bfs"},
                "same_domain": {"type": "boolean", "default": true},
                "allowed_domains": {"type": "array", "items": {"type": "string"}},
                "no_content": {"type": "boolean", "default": false, "description": "spider: link-only fast path"},
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
                "engine": {"type": "string", "enum": ["duckduckgo","google","bing","brave"], "default": "duckduckgo"}
            }, "required": ["q"]}
        }),
        json!({
            "name": "webrain_serp",
            "description": "STRUCTURED search results as typed JSON (position/title/url/domain/snippet) — a SERP API for any LLM. engine: duckduckgo (default) | bing | google | brave | auto. duckduckgo|bing|auto are pure HTTP (no browser). google|brave are JS-gated and auto-launch a guest Chrome (or attach to the connected CDP engine), then paginate the results page for more than 10. auto fetches duckduckgo+bing concurrently (google/brave join via the browser when one is attached, sequential), relevance-filters junk pages, merges + dedupes, limit 1..=100. Features: provider fallback (fallback, default true), pagination (page), safe search (safe), region/locale (region, e.g. us-en/gb-en/gr-el), request_id + ms + per_engine breakdown + source (actual provider after a fallback) in the reply, retry with backoff. Returns {status, query, engine, source, results[], per_engine[], request_id, ms, skipped[]}.",
            "inputSchema": {"type": "object", "properties": {
                "q": {"type": "string"},
                "engine": {"type": "string", "enum": ["duckduckgo","bing","google","brave","auto"], "default": "duckduckgo", "description": "duckduckgo|bing|auto = HTTP (no browser); google|brave = guest-browser flow"},
                "limit": {"type": "integer", "default": 10, "description": "max results, 1..=100"},
                "page": {"type": "integer", "default": 0, "description": "0-based results page"},
                "safe": {"type": "boolean", "default": false},
                "region": {"type": "string", "description": "locale/region, e.g. us-en, gb-en, gr-el, de-de"},
                "fallback": {"type": "boolean", "default": true, "description": "fall back to another provider when the requested engine errors or returns zero"},
                "retries": {"type": "integer", "default": 2, "description": "transient-failure retries with backoff, 0..=5"},
                "proxy": {"type": "string", "description": "HTTP(S)/SOCKS proxy URL (e.g. http://user:pass@host:port) for the HTTP engines (duckduckgo/bing/auto) and baked into the guest Chrome launch for google/brave."}
            }, "required": ["q"]}
        }),
        json!({
            "name": "webrain_pdf",
            "description": "PDF work. Pick `op` (required): page (save current page as PDF) | extract (PDF → markdown/JSON: path or paths[]) | render (PDF pages → PNG tiles for vision: path + optional pages/dpi/tile_size) | images (extract embedded images: path + optional pages).",
            "inputSchema": {"type": "object", "properties": {
                "op": {"type": "string", "enum": ["page","extract","render","images"]},
                "path": {"type": "string", "description": "extract/render/images: PDF file path"},
                "paths": {"type": "array", "items": {"type": "string"}, "description": "extract: batch paths"},
                "pages": {"type": "array", "items": {"type": "integer"}, "description": "render/images: page numbers (1-based), omit = all"},
                "dpi": {"type": "number", "default": 150, "description": "render: DPI"},
                "tile_size": {"type": "integer", "description": "render: split pages into square tiles of this px"}
            }, "required": ["op"]}
        }),
        json!({
            "name": "webrain_download",
            "description": "Download files/media. engine: http (default — stream plain URLs, optional filter_extension narrows to one type) | ytdlp (video/audio via yt-dlp: HLS/DASH/playlists, audio_only, format, args[]). Batch via urls[].",
            "inputSchema": {"type": "object", "properties": {
                "urls": {"type": "array", "items": {"type": "string"}},
                "dir": {"type": "string", "default": "downloads"},
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
            "description": "Browser / auth / session state. Pick `op` (required): open (create named session: session_id+cdp_url), close (destroy session), list (active sessions), cookies (export), setcookies (import cookies[]), save_state / restore_state (auth state ↔ state.json: service+profile+port), profiles (vault names), login (auto-login from vault: service+profile+url), close_launch (stop a launched Chrome: service+profile).",
            "inputSchema": {"type": "object", "properties": {
                "op": {"type": "string", "enum": ["open","close","list","cookies","setcookies","save_state","restore_state","profiles","login","close_launch"]},
                "session_id": {"type": "string", "description": "open/close: session name"},
                "cdp_url": {"type": "string", "description": "open: CDP URL for this session"},
                "cookies": {"type": "array", "items": {"type": "object"}, "description": "setcookies: cookie objects (name, value, domain, path, ...)"},
                "service": {"type": "string", "description": "save/restore_state, login, close_launch: service"},
                "profile": {"type": "string", "description": "save/restore_state, login, close_launch: profile"},
                "port": {"type": "integer", "default": 9222, "description": "state/login: CDP port"},
                "url": {"type": "string", "description": "login: login page to navigate first"}
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
                "x": {"type": "number", "description": "ask: clip region x (default full viewport)"},
                "y": {"type": "number", "description": "ask: clip region y"},
                "w": {"type": "number", "description": "ask: clip region width"},
                "h": {"type": "number", "description": "ask: clip region height"}
            }, "required": ["op"]}
        }),
    ]
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

/// crawl4ai `PruningContentFilter` borrow: no-query "fit" content extractor.
/// Walks the DOM scoring blocks by text-vs-link density + tag importance, drops
/// nav/footer/aside/form/header boilerplate, and returns the dense text — the
/// meat of the page for the LLM, instead of raw innerText full of chrome.
/// ponytail: block tags + score thresholds are a heuristic; tune if a page type
/// over/under-prunes.
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
pub async fn call_tool(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});

    // Consolidated 17-tool surface → legacy executor. Legacy tool names pass
    // through (their own arms still match) — old agents keep working.
    let mapped = map_surface(name, args);
    let name: &str = mapped.as_ref().map(|(n, _)| *n).unwrap_or(name);
    let args: &Value = mapped.as_ref().map(|(_, a)| a).unwrap_or(args);

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

    match name {
        "webrain_guide" => json!({"status": "ok", "guide": AGENT_GUIDE}),
        "webrain_add_init_script" => {
            let js = args.get("js").and_then(|v| v.as_str()).unwrap_or("");
            if js.is_empty() {
                return json!({"status": "error", "message": "js required"});
            }
            match backend.add_init_script(js).await {
                Ok(_) => json!({"status": "ok"}),
                Err(e) => err(e),
            }
        }
        "webrain_navigate" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                return json!({"status": "error", "message": "url required"});
            }
            let opts = nav_opts(args);
            match backend.navigate_opts(url, &opts).await {
                Ok(s) => {
                    json!({"status": "ok", "url": s.url, "title": s.title, "text": s.text, "elements": s.elements, "links": s.links, "challenge": s.challenge, "crippled": s.crippled, "chrome_error": s.chrome_error})
                }
                Err(e) => err(e),
            }
        }
        "webrain_eval" => {
            let js = args.get("js").and_then(|v| v.as_str()).unwrap_or("");
            match backend.evaluate(js).await {
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
            match backend.screenshot(full).await {
                Ok(png) => {
                    use base64::Engine;
                    let mut out = json!({
                        "status": "ok",
                        "screenshot_b64": base64::engine::general_purpose::STANDARD.encode(&png),
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
                        let path = format!("{dir}/shot_{ts}.png");
                        match std::fs::write(&path, &png) {
                            Ok(()) => out["path"] = json!(path),
                            Err(e) => out["path_error"] = json!(e.to_string()),
                        }
                    }
                    out
                }
                Err(e) => err(e),
            }
        }
        "webrain_click" => {
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
            match backend.type_text(idx as usize, text).await {
                Ok(_) => json!({"status": "ok", "typed": idx}),
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
        "webrain_drag" => {
            let x1 = args.get("x1").and_then(|v| v.as_i64()).unwrap_or(-1);
            let y1 = args.get("y1").and_then(|v| v.as_i64()).unwrap_or(-1);
            let x2 = args.get("x2").and_then(|v| v.as_i64()).unwrap_or(-1);
            let y2 = args.get("y2").and_then(|v| v.as_i64()).unwrap_or(-1);
            if x1 < 0 || y1 < 0 || x2 < 0 || y2 < 0 {
                return json!({"status": "error", "message": "x1,y1,x2,y2 required"});
            }
            match backend.drag(x1, y1, x2, y2).await {
                Ok(_) => json!({"status": "ok", "drag": [x1, y1, x2, y2]}),
                Err(e) => err(e),
            }
        }
        "webrain_eval_in_frame" => {
            let url_contains = args
                .get("url_contains")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let js = args
                .get("js")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
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
            match backend.scroll(dir).await {
                Ok(_) => json!({"status": "ok", "direction": dir}),
                Err(e) => err(e),
            }
        }
        "webrain_get_html" => match backend.get_html().await {
            Ok(html) => json!({"status": "ok", "html": html}),
            Err(e) => err(e),
        },
        // Cross-browser session migration: read/write cookies ON THE SESSION
        // BACKEND. Set + batch on the same connection so per-connection
        // isolated browsers (obscura stealth) share the imported session.
        // Chrome login -> webrain_cookies -> webrain_setcookies -> webrain_batch
        // (no cdp_urls) keeps cookies on one connection end-to-end.
        "webrain_cookies" => match backend.cookies().await {
            Ok(c) => json!({"status": "ok", "count": c.len(), "cookies": c}),
            Err(e) => err(e),
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
            match backend.set_cookies(&cookies).await {
                Ok(_) => {
                    let back = match backend.cookies().await {
                        Ok(c) => c.len(),
                        Err(_) => 0,
                    };
                    json!({"status": "ok", "set": cookies.len(), "readback": back})
                }
                Err(e) => err(e),
            }
        }
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
            let concurrency = args
                .get("concurrency")
                .and_then(|v| v.as_i64())
                .unwrap_or(4)
                .max(1) as usize;
            let spider = SpiderEngine::new(depth, pages)
                .with_strategy(strategy)
                .with_same_domain(same_domain)
                .with_allowed_domains(allowed)
                .with_discover_only(discover_only)
                .with_respect_robots(respect_robots)
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
            let results = spider.crawl(backend, seed).await;
            // ponytail: spider stats — consistent with the batch stats block; the
            // LLM sees elapsed + ok/err at a glance for a long crawl.
            let total_ms = t0.elapsed().as_millis() as u64;
            let ok = results.iter().filter(|r| r.page.error.is_none()).count();
            let errs = results.len() - ok;
            let page_ms: u64 = results.iter().map(|r| r.page.duration_ms).sum();
            json!({
                "status": "ok", "pages": results.len(), "results": results,
                "stats": {"elapsed_ms": total_ms, "pages_ok": ok, "pages_err": errs, "page_ms_total": page_ms}
            })
        }
        "webrain_sitemap" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                return json!({"status": "error", "message": "url required"});
            }
            match sitemap_urls(url) {
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
            match engine.tile(backend).await {
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
            let js = r#"JSON.stringify(Array.from(document.querySelectorAll('table')).slice(0,20).map(t => { const headers = Array.from(t.querySelectorAll('th')).map(th => th.textContent.trim()); const rows = Array.from(t.querySelectorAll('tr')).slice(headers.length ? 1 : 0).map(tr => Array.from(tr.querySelectorAll('td,th')).map(td => td.textContent.trim())); return headers.length ? rows.map(r => Object.fromEntries(headers.map((h,i) => [h, r[i] ?? null]))) : rows; }))"#;
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
            let js = format!(
                r#"(()=>{{ const c={{}}; document.querySelectorAll('div[class],li,tr,article,section').forEach(el=>{{ const cls = el.className ? '.'+String(el.className).trim().split(/\s+/).slice(0,2).join('.') : ''; const key=el.tagName.toLowerCase()+cls; c[key]=(c[key]||0)+1; }}); return JSON.stringify(Object.entries(c).filter(([k,n])=>n>={min_occurrences}).sort((a,b)=>b[1]-a[1]).slice(0,8).map(([sel,count])=>({{selector:sel, count}}))); }})()"#
            );
            match backend.evaluate(&js).await {
                Ok(v) => {
                    let arr = parse_json_str(&v);
                    json!({"status": "ok", "candidates": arr})
                }
                Err(e) => err(e),
            }
        }
        "webrain_fetch_http" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                return json!({"status": "error", "message": "url required"});
            }
            match http_fetch(url) {
                Ok(v) => json!({"status": "ok", "result": v}),
                Err(e) => err(e),
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
            let results = validate_urls(&urls);
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
                    // open_tab now starts blank (one load per URL in batch paths);
                    // here we navigate explicitly to keep "new tab at url" semantics.
                    match backend.open_tab("about:blank").await {
                        Ok(id) => match backend.navigate(url).await {
                            Ok(_) => json!({
                                "status": "ok", "tab": id,
                                "tabs": backend.list_tabs().await.unwrap_or(json!([]))
                            }),
                            Err(e) => err(e),
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
            let concurrency = args
                .get("concurrency")
                .and_then(|v| v.as_i64())
                .unwrap_or(4) as usize;
            let opts = nav_opts(args);

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
                match op {
                    "extract" => {
                        batch_extract(b, urls, &base, &base_fields, &fields, concurrency, opts)
                            .await
                    }
                    "interact" => {
                        let interaction = args
                            .get("interaction")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
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
                        let js = args
                            .get("js")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        if js.is_empty() {
                            return vec![BatchResult {
                                url: urls[0].clone(),
                                title: String::new(),
                                text: String::new(),
                                data: None,
                                error: Some("js required (op=eval)".to_string()),
                                ms: 0,
                            }];
                        }
                        batch_eval(b, urls, &js, concurrency, opts).await
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
            let per_backend_concurrency = args
                .get("per_backend_concurrency")
                .and_then(|v| v.as_i64())
                .map(|n| n.max(1) as usize)
                .unwrap_or(concurrency);
            let results = if cdp_urls.is_empty() {
                run_batch(backend, op, &urls, args, &opts, concurrency).await
            } else {
                let mut all = Vec::new();
                let mut backends = Vec::new();
                for u in &cdp_urls {
                    match CdpBackend::connect_with_url(u).await {
                        Ok(b) => backends.push(b),
                        Err(e) => all.push(BatchResult {
                            url: u.clone(),
                            title: String::new(),
                            text: String::new(),
                            data: None,
                            error: Some(format!("cdp connect failed: {e}")),
                            ms: 0,
                        }),
                    }
                }
                let n = backends.len().max(1);
                // Round-robin: url[i] -> backend[i % n]. Each backend runs its share
                // at `per_backend_concurrency` tabs; backends themselves run
                // sequentially (connect cost dominates; true cross-backend
                // concurrency is the agent's job).
                for (bi, b) in backends.iter().enumerate() {
                    let share: Vec<String> = urls
                        .iter()
                        .enumerate()
                        .filter(|(i, _)| i % n == bi)
                        .map(|(_, u)| u.clone())
                        .collect();
                    if !share.is_empty() {
                        all.extend(
                            run_batch(b, op, &share, args, &opts, per_backend_concurrency).await,
                        );
                    }
                }
                all
            };
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
                payload["stats"] =
                    json!({"total": res.len(), "ok": ok, "errors": errs, "ms_total": ms});
            }
            payload
        }
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
                return webrain_core::engines::download_ytdlp(
                    &urls,
                    &dir,
                    audio_only,
                    format.as_deref(),
                    &extra,
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
            let results = download_files(&urls, &dir);
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
            let js = r#"JSON.stringify(Array.from(document.images).map(img => ({src: img.currentSrc || img.src, alt: img.alt || '', width: img.naturalWidth, height: img.naturalHeight})).slice(0, 200))"#;
            match backend.evaluate(js).await {
                Ok(v) => {
                    let arr = parse_json_str(&v);
                    json!({"status": "ok", "count": arr_len(&arr), "images": arr})
                }
                Err(e) => err(e),
            }
        }
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
        "webrain_console" => {
            // ponytail: lazy listener injection; captures uncaught errors + rejections.
            let js = r#"
            (() => {
                if (!window.__wr_logs) {
                    window.__wr_logs = [];
                    window.addEventListener('error', e => { window.__wr_logs.push({type: 'error', text: (e.message || '').slice(0, 300)}); }, true);
                    window.addEventListener('unhandledrejection', e => { window.__wr_logs.push({type: 'rejection', text: String(e.reason || '').slice(0, 300)}); });
                }
                const out = window.__wr_logs;
                window.__wr_logs = [];
                return JSON.stringify(out);
            })()
            "#;
            match backend.evaluate(js).await {
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
            match ask_viewport(backend, &prompt, clip, &tiles, scale).await {
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
    };
    // google/brave need a browser — the lib.rs short-circuit routes HTTP engines
    // around the backend; here (call_tool) a backend may or may not be attached,
    // and lib.rs auto-launches a guest browser when none is.
    if (opts.engine == "brave" || opts.engine == "google") && backend.is_none() {
        return json!({"status": "error", "message": "engine 'google'/'brave' requires a connected browser (set CDP_URL or start Chrome with --remote-debugging-port=9222; webrain_serp auto-launches a guest browser when none is attached); duckduckgo|bing|auto work without one"});
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
}
