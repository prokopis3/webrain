use webrain_core::backends::cdp::CdpBackend;
use webrain_core::browser::BrowserBackend;
use webrain_core::engines::{
    batch_extract, batch_fetch, batch_interact, batch_screenshot, bm25_filter, build_clean_js,
    build_adaptive_extract_js, build_extract_js, download_files, http_fetch, regex_extract,
    validate_urls, BatchResult, CrawlStrategy, SpiderEngine, TileEngine,
};
use webrain_core::vision::{index_current_page, retrieve as vision_retrieve};
use serde_json::{json, Value};

/// Render the AX tree as compact `role "name"` lines for LLM reading.
/// ponytail: flat walk, capped 200 nodes, no hierarchy (a11y gives JSON for depth).
fn semantic_tree_text(nodes: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    if let Some(arr) = nodes.as_array() {
        for n in arr.iter().take(200) {
            let role = n.get("role").and_then(|r| r.get("value")).and_then(|v| v.as_str()).unwrap_or("");
            let name = n.get("name").and_then(|r| r.get("value")).and_then(|v| v.as_str()).unwrap_or("");
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

BROWSER / CHALLENGE HANDLING
- webrain_navigate returns a `challenge` field — read it on EVERY navigate.
- challenge == null -> page OK, go extract.
- challenge != null (cloudflare_challenge | blocked | captcha) -> page is gated.
    * obscura / lightpanda CANNOT pass interactive challenges (no paint engine;
      challenge JS crashes).
    * FIX (the "chrome way"): run the real-Chrome stealth sidecar once:
        python scripts/stealth_solve.py <login_or_challenge_url> --cdp-port 9222 --headed
      it waits out the challenge, logs in (--creds user:pass), keeps Chrome alive
      on 9222. Then set CDP_URL=http://127.0.0.1:9222 and re-navigate — the
      authenticated session (cf_clearance) is shared with webrain.
    * Non-interactive Turnstile / basic bot detection may pass with obscura --stealth.
- Need screenshots/rendering? Real Chrome. Fast no-challenge scraping? obscura.
  Static HTML, no JS/auth? webrain_fetch_http.

EXTRACTION MATRIX
- structured list, schema known        -> webrain_extract_json(base_selector, fields)
- schema unknown                       -> webrain_autoschema (container) + webrain_eval
                                        (probe descendant tag/class/sample) then extract_json
- paginated pages 1..N / many URLs     -> webrain_batch(op=extract, urls, base_selector,
                                        fields, concurrency=8)
- URLs unknown (discover pagination)   -> webrain_eval: links whose path = current + '/<N>'
                                        plus next/prev labels (NO hardcoded classes), derive
                                        range, then webrain_batch
- whole site                           -> webrain_spider
- emails/phones/prices/patterns        -> webrain_extract_regex
- JSON-LD / microdata                  -> webrain_get_jsonld
- tables                               -> webrain_table
- infinite scroll / load-more          -> webrain_scan then extract; or webrain_click loop
- search                               -> webrain_search (engine: duckduckgo|google|bing via plain
                                        HTTP; brave returns an SPA shell -> use
                                        webrain_navigate("https://search.brave.com/search?q=..."))
- static, no browser                   -> webrain_fetch_http
- relevance filter                     -> webrain_bm25

FROM-SCRATCH DISCOVERY (schema + urls unknown)
  1. webrain_navigate(seed)
  2. webrain_eval -> pagination hrefs (structural, no class assumptions) -> max page -> urls
  3. webrain_autoschema -> container selector
  4. webrain_eval -> descendant tags/classes + samples -> fields
  5. webrain_batch(op=extract, urls, base_selector, fields, concurrency=8) -> aggregate
  6. done(summary="Extracted N items across M pages")

RULES
- Never guess selectors/browsers from memory — discover via autoschema/eval and
  read the `challenge` field on every navigate.
- Prefer extract_json / table / regex over get_html (token-cheap).
- webrain_fetch_http needs NO browser (pure HTTP, 10-100x faster for static pages).
- webrain_tab manages tabs: new(url) | switch(id) | close(id) | list. Use tabs to
  isolate parallel scrapes or pre-load login sessions in one tab while scraping
  in another.

PARALLEL / MULTI-BROWSER EXECUTION
- webrain's MCP server creates per-connection sessions. An orchestrator LLM can
  spawn subagents with different CDP_URL values for true parallel execution:
    Subagent A → CDP_URL=http://127.0.0.1:9222  (real Chrome, solving CF)
    Subagent B → CDP_URL=http://127.0.0.1:9224  (obscura, batch scraping)
    Subagent C → (no CDP) + webrain_fetch_http   (static pages, zero browser)
- webrain_batch supports concurrency=N (parallel tabs within one browser).
"#;

pub fn list_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "webrain_guide",
            "description": "Agent decision guide: browser selection (real Chrome vs obscura vs lightpanda vs fetch_http), how to bypass Cloudflare/CAPTCHA/Turnstile challenges (check the `challenge` field after webrain_navigate; run scripts/stealth_solve.py for gated pages), and the extraction tool matrix. Call FIRST when unsure which webrain tool/browser to use.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "webrain_eval",
            "description": "Run arbitrary JavaScript in the current page and return the JSON result. Use for precise structured extraction (e.g. product schemas).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "js": {"type": "string", "description": "JavaScript expression to evaluate. Return a JSON-serializable value (e.g. JSON.stringify(...))."}
                },
                "required": ["js"]
            }
        }),
        json!({
            "name": "webrain_navigate",
            "description": "Navigate to a URL and return page state (title, visible text, interactive elements) plus a `challenge` field (cloudflare_challenge|blocked|captcha) when the page is gated by an anti-bot challenge. If `challenge` is set, see webrain_guide for the real-Chrome bypass (obscura/lightpanda cannot pass interactive challenges). Optional request-quality params (Scrapling-style): disable_resources (block fonts/images/media for speed+token savings), network_idle (wait until no new network activity), wait_selector + wait_selector_state (attached|visible|hidden|detached) to wait for a specific element, css_selector to narrow returned text to one element.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "The URL to navigate to"},
                    "disable_resources": {"type": "boolean", "description": "Block font/image/media/stylesheet requests (faster, fewer tokens)", "default": false},
                    "block_trackers": {"type": "boolean", "description": "Also block the 3500-domain tracker/ad list (max privacy; ~35KB over CDP per navigate)", "default": false},
                    "network_idle": {"type": "boolean", "description": "Wait until no new network resource entries (~400ms stable) before returning", "default": false},
                    "wait_selector": {"type": "string", "description": "Wait for this CSS selector before returning"},
                    "wait_selector_state": {"type": "string", "enum": ["attached", "visible", "hidden", "detached"], "description": "State to wait for (default visible)", "default": "visible"},
                    "css_selector": {"type": "string", "description": "Narrow returned text to this element's innerText (token saver)"}
                },
                "required": ["url"]
            }
        }),
        json!({
            "name": "webrain_screenshot",
            "description": "Take a screenshot of the current page. Returns base64-encoded PNG.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "full_page": {"type": "boolean", "description": "Capture full scrollable page", "default": false}
                }
            }
        }),
        json!({
            "name": "webrain_click",
            "description": "Click an interactive element by its index (from webrain_navigate elements list)",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "index": {"type": "integer", "description": "Element index from page state"}
                },
                "required": ["index"]
            }
        }),
        json!({
            "name": "webrain_type",
            "description": "Type text into an input element by its index",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "index": {"type": "integer", "description": "Input element index"},
                    "text": {"type": "string", "description": "Text to type"}
                },
                "required": ["index", "text"]
            }
        }),
        json!({
            "name": "webrain_scroll",
            "description": "Scroll the page up or down",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "direction": {"type": "string", "enum": ["up", "down"], "default": "down"}
                }
            }
        }),
        json!({
            "name": "webrain_get_html",
            "description": "Get the HTML of the current page or a specific element by CSS selector",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "selector": {"type": "string", "description": "Optional CSS selector for a specific element"}
                }
            }
        }),
        json!({
            "name": "webrain_spider",
            "description": "Crawl a website starting from a seed URL using BFS, following links",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "seed_url": {"type": "string", "description": "Starting URL for the crawl"},
                    "max_depth": {"type": "integer", "description": "Maximum link depth", "default": 2},
                    "max_pages": {"type": "integer", "description": "Maximum pages to crawl", "default": 20}
                },
                "required": ["seed_url"]
            }
        }),
        json!({
            "name": "webrain_snapshot",
            "description": "Re-capture current page state WITHOUT navigating. D1 DOM-fingerprint skip: returns the cached state unchanged when the page hasn't mutated, saving tokens.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "webrain_pixel",
            "description": "PixelRAG-style tile capture: split the current page into a grid of screenshot tiles (base64 PNGs) so a vision model can read regions (tables/charts/layout survive).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tile_width": {"type": "number", "description": "Tile width in CSS px", "default": 800},
                    "tile_height": {"type": "number", "description": "Tile height in CSS px", "default": 800},
                    "max_tiles": {"type": "integer", "description": "Max tiles to capture", "default": 16}
                }
            }
        }),
        json!({
            "name": "webrain_extract_json",
            "description": "CSS-schema extraction: build a JSON array from a base selector + field selectors. Zero-LLM structured extraction (crawl4ai JsonCssExtractionStrategy style). Set adaptive:true to auto-relocate the container when the base selector matches nothing (site redesigned) — finds elements still containing >=2 of the field selectors.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "base_selector": {"type": "string", "description": "CSS selector for each repeated item, e.g. '.product'"},
                    "fields": {"type": "array", "items": {"type": "object"}, "description": "[{\"name\": \"title\", \"selector\": \"h3 a\", \"type\": \"text\"}] — type: text|attr|html|xpath, attr required for attr"},
                    "base_fields": {"type": "array", "items": {"type": "object"}, "description": "Optional attributes pulled from the container element, e.g. [{\"name\": \"href\", \"attribute\": \"href\"}]"},
                    "adaptive": {"type": "boolean", "description": "If true and base_selector matches 0 items, relocate to elements still containing >=2 field selectors (survives class renames). Default false."}
                },
                "required": ["base_selector", "fields"]
            }
        }),
        json!({
            "name": "webrain_extract_regex",
            "description": "Regex pattern extraction over the current page (zero-LLM): built-ins email/url/phone/price/date/time/ip/uuid + custom [{label, re}]. Scans page HTML (catches href/mailto). crawl4ai RegexExtractionStrategy style.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "patterns": {"type": "array", "items": {"type": "object"}, "description": "Optional custom patterns: [{\"label\": \"sku\", \"re\": \"[A-Z]{2}\\\\d{4}\"}] — a custom label overrides the built-in of the same name"}
                }
            }
        }),
        json!({
            "name": "webrain_get_jsonld",
            "description": "Extract JSON-LD / microdata from the current page (browsemind extract_identity). Zero LLM, zero cost. Returns the parsed <script type=application/ld+json> blocks — schema.org product/article/organization data for free.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "webrain_table",
            "description": "Extract all HTML tables on the current page to JSON (browsemind extract_table). Zero LLM. Returns arrays of {header: cell} row objects per <table>.",
            "inputSchema": {"type": "object", "properties": {}}
        }),
        json!({
            "name": "webrain_scan",
            "description": "Auto-scroll the page to trigger infinite-scroll / load-more content (browsemind scan_full_page). Returns {scrolls, height}. Run before extraction on SPA feeds.",
            "inputSchema": {
                "type": "object",
                "properties": {"max_scrolls": {"type": "integer", "description": "Max scroll steps", "default": 15}}
            }
        }),
        json!({
            "name": "webrain_autoschema",
            "description": "Detect repeated container patterns on the page (browsemind auto-detect CSS schema). Returns candidate base-selectors with occurrence counts for the LLM to build a webrain_extract_json schema. Zero LLM.",
            "inputSchema": {
                "type": "object",
                "properties": {"min_occurrences": {"type": "integer", "description": "Min repeats to report", "default": 3}}
            }
        }),
        json!({
            "name": "webrain_fetch_http",
            "description": "No-browser HTTP fetch (browsemind http_crawl): GET a URL, return {url, status, text}. 10-100x faster than browser navigation, zero memory — but no JS/SPA/auth. Use for static pages.",
            "inputSchema": {
                "type": "object",
                "properties": {"url": {"type": "string", "description": "URL to fetch over plain HTTP"}},
                "required": ["url"]
            }
        }),
        json!({
            "name": "webrain_bm25",
            "description": "BM25 relevance filter (browsemind BM25 filter / crawl4ai ContentRelevanceFilter): score a list of text items against a query, keep the top_k. Zero LLM. Use after extraction to keep only relevant results.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string", "description": "What the kept items should be about"},
                    "items": {"type": "array", "items": {"type": "string"}, "description": "Text items to rank"},
                    "top_k": {"type": "integer", "description": "Max results to return", "default": 10}
                },
                "required": ["query", "items"]
            }
        }),
        json!({
            "name": "webrain_validate_urls",
            "description": "Validate a list of URLs — which are alive vs dead (browsemind seed(from_links, validate=True)). Filters 404s/5xx/errors. HEAD first, GET fallback. Use before batch extraction.",
            "inputSchema": {
                "type": "object",
                "properties": {"urls": {"type": "array", "items": {"type": "string"}}},
                "required": ["urls"]
            }
        }),
        json!({
            "name": "webrain_pdf",
            "description": "Save the current page as PDF (base64-encoded).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "webrain_tab",
            "description": "Manage browser tabs. action 'new': open url in a new tab (returns its id, becomes active). 'switch': activate an existing tab by id. 'close': close a tab by id. 'list': show all tabs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action": {"type": "string", "enum": ["new", "switch", "close", "list"]},
                    "url": {"type": "string", "description": "URL for action=new"},
                    "id": {"type": "string", "description": "Tab id for action=switch/close"}
                },
                "required": ["action"]
            }
        }),
        json!({
            "name": "webrain_a11y",
            "description": "Accessibility-tree snapshot of the current page: [{role, name, value}]. Read-only — understand page structure, then interact via webrain_navigate/webrain_snapshot elements[] indices (click/type).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "webrain_semantic_tree",
            "description": "Semantic-tree text snapshot of the current page (lightpanda LP.getSemanticTree style): role \"name\" lines for the LLM, plus the raw AX JSON.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "webrain_batch",
            "description": "Batch over many URLs using concurrent tabs (crawl4ai arun_many + MemoryAdaptiveDispatcher). REQUIRES a browser backend (CDP) — if no browser is running it errors fast (5s timeout). op 'fetch': read visible text. 'extract': run a CSS/XPath schema (needs base_selector + fields, zero-LLM). 'interact': run an async JS `interaction` in parallel tabs (click Load More loop / infinite-scroll / form fill), then optionally extract a schema — one call replaces N serial agent loops for N interactive sites. 'screenshot': save full-page PNGs to dir. Returns one result per URL. `concurrency` bounds in-flight tabs (default 4); pages load in parallel. Optional request-quality params shared with navigate (all ops incl. screenshot): disable_resources, network_idle, wait_selector, wait_selector_state. Optional `cdp_urls` (list) fans the batch out across N CDP backends round-robin — per-proxy isolation (each browser = own proxy/cookies/fingerprint) in one call, no subagents needed. Optional `output` path persists the full payload to disk (survives temp-file GC between turns).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": ["fetch", "extract", "interact", "screenshot"]},
                    "urls": {"type": "array", "items": {"type": "string"}},
                    "cdp_urls": {"type": "array", "items": {"type": "string"}, "description": "Optional CDP backends to round-robin URLs across (per-proxy isolation). Default: the session backend."},
                    "interaction": {"type": "string", "description": "Async JS interaction to run in each tab (op=interact), e.g. click '#load-more-btn' N times / scroll loop. Does its own waits."},
                    "base_selector": {"type": "string", "description": "CSS selector for repeated items (op=extract / interact)"},
                    "fields": {"type": "array", "items": {"type": "object"}, "description": "[{name, selector, type: text|attr|html|xpath, attr?}] (op=extract / interact)"},
                    "concurrency": {"type": "integer", "description": "Max in-flight tabs (parallel loads)", "default": 4},
                    "per_backend_concurrency": {"type": "integer", "description": "Max tabs per CDP backend when `cdp_urls` is set (memory cap — total tabs = this × backends; default = concurrency)", "default": 4},
                    "dir": {"type": "string", "description": "Output dir (op=screenshot)", "default": "screenshots"},
                    "output": {"type": "string", "description": "Optional file path to persist the full batch payload (JSON) — survives temp-file GC"},
                    "disable_resources": {"type": "boolean", "description": "Block font/image/media/stylesheet requests (faster, fewer tokens)", "default": false},
                    "block_trackers": {"type": "boolean", "description": "Also block the 3500-domain tracker/ad list (max privacy)", "default": false},
                    "network_idle": {"type": "boolean", "description": "Wait until no new network resource entries before extracting", "default": false},
                    "wait_selector": {"type": "string", "description": "Wait for this CSS selector on each page before extracting"},
                    "wait_selector_state": {"type": "string", "enum": ["attached", "visible", "hidden", "detached"], "description": "State to wait for (default visible)", "default": "visible"}
                },
                "required": ["op", "urls"]
            }
        }),
        json!({
            "name": "webrain_download",
            "description": "Download single or many (urls[]) files/video/audio. engine 'http' (default): stream URLs over plain HTTP, optional filter_extension narrows to one type (.mp4/.pdf/.js...). engine 'ytdlp': primary for video/audio — HLS/DASH/.m3u8, playlists, age/cookie-bound media — via the installed yt-dlp binary, full feature passthrough via args (--write-subs, --embed-thumbnail, --cookies, --proxy...). Single URL = pass one element.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "urls": {"type": "array", "items": {"type": "string"}, "description": "One or more URLs (single or batch)"},
                    "dir": {"type": "string", "default": "downloads"},
                    "engine": {"type": "string", "enum": ["http", "ytdlp"], "default": "http", "description": "http = plain streaming; ytdlp = yt-dlp binary"},
                    "filter_extension": {"type": "string", "description": "Only download URLs whose path ends with this extension, e.g. '.mp4' (engine=http)"},
                    "audio_only": {"type": "boolean", "default": false, "description": "Extract audio only as mp3 (engine=ytdlp)"},
                    "format": {"type": "string", "description": "yt-dlp -f value (default 'bestvideo*+bestaudio/best', engine=ytdlp)"},
                    "args": {"type": "array", "items": {"type": "string"}, "description": "Extra yt-dlp CLI flags, e.g. ['--write-subs', '--embed-thumbnail'] (engine=ytdlp)"}
                },
                "required": ["urls"]
            }
        }),
        json!({
            "name": "webrain_search",
            "description": "Search the web and navigate to the results page (browsemind 4-engine pattern). duckduckgo is HTML-lite and scrape-friendly (default). google and bing also return plain HTML via HTTP. brave returns an SPA shell (JS-rendered) — use webrain_navigate to Brave's URL instead for real results.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "q": {"type": "string", "description": "Search query"},
                    "engine": {"type": "string", "description": "duckduckgo (default) | google | bing | brave. brave needs a browser (SPA).", "enum": ["duckduckgo", "bing", "brave", "google"], "default": "duckduckgo"}
                },
                "required": ["q"]
            }
        }),
        json!({
            "name": "webrain_nav",
            "description": "Browser navigation: go back, forward, or reload the current page.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "op": {"type": "string", "enum": ["back", "forward", "reload"], "default": "back"}
                }
            }
        }),
        json!({
            "name": "webrain_press",
            "description": "Press a key in the focused element (Enter, Tab, Escape, Backspace, ArrowDown...). Use after webrain_type to submit forms.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "key": {"type": "string", "description": "Key to press", "default": "Enter"}
                }
            }
        }),
        json!({
            "name": "webrain_get_images",
            "description": "List images on the current page: [{src, alt, width, height}]. Useful for extracting product/photo URLs.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "webrain_media",
            "description": "Discover media URLs the page loads (browsemind find_media_urls). With a url: CDP Network capture of the full load — catches JS-loaded .m3u8/.mp4/manifest/player-API requests that static-HTML regex misses (e.g. antenna Phaistos player). Without a url: Performance API + <video>/<audio>/<source> scan of the current page.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "url": {"type": "string", "description": "Optional page URL to load and capture (default: scan current page)"},
                    "wait_ms": {"type": "integer", "description": "Optional ms to keep capturing after load for late player requests (default 0)", "default": 0}
                }
            }
        }),
        json!({
            "name": "webrain_console",
            "description": "Return captured page errors/warnings (uncaught errors + unhandled rejections) since the last call. Injects a listener on first call (browsemind console list pattern).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "webrain_dismiss_overlays",
            "description": "Remove visible fixed/sticky overlays (cookie banners, popups, modals) that block interaction (browsemind C1-C3 overlay defense, manual trigger).",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "webrain_vision_index",
            "description": "PixelRAG: capture the CURRENT page as vision tiles, embed each tile via EMBED_URL (Qwen3-VL-Embedding-2B / vLLM), and add them to a cosine index (persisted to vision/{tag}.jsonl). Requires EMBED_URL set to a running /embeddings endpoint.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tag": {"type": "string", "description": "Index name", "default": "default"},
                    "max_tiles": {"type": "integer", "description": "Max tiles to embed", "default": 8},
                    "tile_width": {"type": "number", "default": 800},
                    "tile_height": {"type": "number", "default": 800}
                }
            }
        }),
        json!({
            "name": "webrain_vision_retrieve",
            "description": "Embed a text query and return the cosine top-k stored tile ids from a vision index (the reverse of webrain_vision_index — semantic page retrieval).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "tag": {"type": "string", "default": "default"},
                    "query": {"type": "string", "description": "Text query to match against indexed pages"},
                    "k": {"type": "integer", "default": 5}
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "webrain_clean",
            "description": "Clean page text: strip nav/footer/script/style/iframe, exclude social/ads links, filter by word length. Returns clean text blob (max 8KB). In-page JS, zero-LLM, no HTML→Markdown.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "word_threshold": {"type": "integer", "description": "Minimum word length to keep", "default": 2},
                    "exclude_social": {"type": "boolean", "description": "Remove social media links", "default": true}
                }
            }
        }),
        json!({
            "name": "webrain_pdf_extract",
            "description": "Convert a PDF to Markdown (Firecrawl pdf-inspector engine, pure Rust on lopdf). Returns JSON: page count, pdf_type (TextBased/Scanned/Mixed), confidence, has_encoding_issues, layout (is_complex, pages_with_tables, pages_with_columns), full 'markdown' (headings/lists/tables/bold-italic), and per-page 'texts'. Proper ToUnicode CMap decoding fixes LaTeX/CID-font PDFs. Accepts a single 'path' or batch 'paths'. No external deps, no OCR needed for text-based PDFs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to PDF file on disk"},
                    "paths": {"type": "array", "items": {"type": "string"}, "description": "Batch: multiple PDF file paths"}
                }
            }
        }),
        json!({
            "name": "webrain_pdf_render",
            "description": "Render PDF pages as base64 PNG images — the PixelRAG / vision-model alternative to text extraction. Renders pages visually so a vision-capable LLM can read the text directly, bypassing font encoding issues entirely. Optional tile_size (e.g. 800) splits each page into square tiles for more efficient vision processing of multi-page PDFs. Requires --features pdfium. Returns [{page, image_b64}] or [{page, x, y, w, h, image_b64}] per tile.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to PDF file on disk"},
                    "pages": {"type": "array", "items": {"type": "integer"}, "description": "Specific page numbers to render (1-based). Omit for all pages."},
                    "dpi": {"type": "number", "description": "Render DPI (default 150)", "default": 150},
                    "tile_size": {"type": "integer", "description": "If set, split each page into square tiles of this pixel size (e.g. 800). Smaller tiles = faster vision processing, more chunks."}
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "webrain_pdf_images",
            "description": "Extract embedded images/figures from a PDF as base64 PNGs — zero system deps (lopdf + image crate). Handles DCTDecode (JPEG) and FlateDecode (zlib raw pixels). Skips JPEG2000/CCITT/JBIG2 (use webrain_pdf_render for those). Returns [{page, index, width, height, image_b64}]. Works in the default build, no --features pdfium needed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Path to PDF file on disk"},
                    "pages": {"type": "array", "items": {"type": "integer"}, "description": "Specific page numbers to scan (1-based). Omit for all pages."}
                },
                "required": ["path"]
            }
        }),
        json!({
            "name": "webrain_open_session",
            "description": "Create a named browser session pool. The calling LLM can then direct webrain_batch/webrain_navigate calls to this session via the session_id argument. Use different sessions to isolate tasks or browsers (e.g. one per CDP_URL for parallel subagents). Returns the session_id and whether it already existed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {"type": "string", "description": "Custom name for this session (default: auto-generated sess-N)."},
                    "cdp_url": {"type": "string", "description": "CDP URL for this session's browser (default: server CDP_URL env)."}
                }
            }
        }),
        json!({
            "name": "webrain_close_session",
            "description": "Destroy a named session pool and its browser backend. The 'default' session cannot be closed. Returns whether the session was found and closed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "session_id": {"type": "string", "description": "Session ID to close (from webrain_open_session or webrain_list_sessions)."}
                },
                "required": ["session_id"]
            }
        }),
        json!({
            "name": "webrain_list_sessions",
            "description": "List all active session pools with their session IDs and CDP URLs. Use this to discover which sessions are available for subagent routing.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
    ]
}

/// Dispatch tool calls to the shared CDP backend.
/// ponytail: match arm, no registry pattern.
pub async fn call_tool(backend: &CdpBackend, name: &str, args: &Value) -> Value {
    let err = |e: anyhow::Error| json!({"status": "error", "message": e.to_string()});

    // Parse Scrapling-style request-quality params from tool args (shared by
    // navigate + batch). All optional; defaults keep legacy behavior.
    fn nav_opts(args: &Value) -> webrain_core::backends::cdp::NavOpts {
        use webrain_core::backends::cdp::NavOpts;
        NavOpts {
            disable_resources: args.get("disable_resources").and_then(|v| v.as_bool()).unwrap_or(false),
            block_trackers: args.get("block_trackers").and_then(|v| v.as_bool()).unwrap_or(false),
            network_idle: args.get("network_idle").and_then(|v| v.as_bool()).unwrap_or(false),
            wait_selector: args.get("wait_selector").and_then(|v| v.as_str()).map(String::from),
            wait_selector_state: args.get("wait_selector_state").and_then(|v| v.as_str()).unwrap_or("visible").to_string(),
            css_selector: args.get("css_selector").and_then(|v| v.as_str()).map(String::from),
        }
    }

    match name {
        "webrain_guide" => json!({"status": "ok", "guide": AGENT_GUIDE}),
        "webrain_navigate" => {
            let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("");
            if url.is_empty() {
                return json!({"status": "error", "message": "url required"});
            }
            let opts = nav_opts(args);
            match backend.navigate_opts(url, &opts).await {
                Ok(s) => json!({"status": "ok", "url": s.url, "title": s.title, "text": s.text, "elements": s.elements, "challenge": s.challenge}),
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
            let full = args.get("full_page").and_then(|v| v.as_bool()).unwrap_or(false);
            match backend.screenshot(full).await {
                Ok(png) => {
                    use base64::Engine;
                    json!({"status": "ok", "screenshot_b64": base64::engine::general_purpose::STANDARD.encode(png)})
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
        "webrain_scroll" => {
            let dir = args.get("direction").and_then(|v| v.as_str()).unwrap_or("down");
            match backend.scroll(dir).await {
                Ok(_) => json!({"status": "ok", "direction": dir}),
                Err(e) => err(e),
            }
        }
        "webrain_get_html" => {
            match backend.get_html().await {
                Ok(html) => json!({"status": "ok", "html": html}),
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
            let strategy = match args.get("strategy").and_then(|v| v.as_str()).unwrap_or("bfs") {
                "dfs" => CrawlStrategy::Dfs,
                "bestfirst" => CrawlStrategy::BestFirst,
                _ => CrawlStrategy::Bfs,
            };
            let keywords: Vec<String> = args.get("keywords")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let same_domain = args.get("same_domain").and_then(|v| v.as_bool()).unwrap_or(true);
            let discover_only = args.get("no_content").and_then(|v| v.as_bool()).unwrap_or(false);
            let respect_robots = args.get("respect_robots").and_then(|v| v.as_bool()).unwrap_or(false);
            let allowed: Vec<String> = args.get("allowed_domains")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let spider = SpiderEngine::new(depth, pages)
                .with_strategy(strategy)
                .with_same_domain(same_domain)
                .with_allowed_domains(allowed)
                .with_discover_only(discover_only)
                .with_respect_robots(respect_robots)
                .with_keywords(keywords);
            let results = spider.crawl(backend, seed).await;
            json!({"status": "ok", "pages": results.len(), "results": results})
        }
        "webrain_snapshot" => {
            match backend.snapshot().await {
                Ok(s) => json!({"status": "ok", "url": s.url, "title": s.title, "text": s.text, "elements": s.elements, "challenge": s.challenge}),
                Err(e) => err(e),
            }
        }
        "webrain_pixel" => {
            let tw = args.get("tile_width").and_then(|v| v.as_f64()).unwrap_or(800.0);
            let th = args.get("tile_height").and_then(|v| v.as_f64()).unwrap_or(800.0);
            let mt = args.get("max_tiles").and_then(|v| v.as_i64()).unwrap_or(16) as usize;
            let engine = TileEngine::new(tw, th, mt);
            match engine.tile(backend).await {
                Ok(tiles) => json!({"status": "ok", "count": tiles.len(), "tiles": tiles}),
                Err(e) => err(e),
            }
        }
        "webrain_extract_json" => {
            let base = args.get("base_selector").and_then(|v| v.as_str()).unwrap_or("");
            if base.is_empty() {
                return json!({"status": "error", "message": "base_selector required"});
            }
            let base_fields = args.get("base_fields").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let fields = args.get("fields").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            let adaptive = args.get("adaptive").and_then(|v| v.as_bool()).unwrap_or(false);
            let js = if adaptive {
                build_adaptive_extract_js(&base, &base_fields, &fields)
            } else {
                build_extract_js(&base, &base_fields, &fields)
            };
            match backend.evaluate(&js).await {
                Ok(v) => {
                    let arr = v
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(Value::Null);
                    json!({"status": "ok", "count": arr.as_array().map(|a| a.len()).unwrap_or(0), "data": arr})
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
                    let arr = v
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(Value::Null);
                    json!({"status": "ok", "count": arr.as_array().map(|a| a.len()).unwrap_or(0), "jsonld": arr})
                }
                Err(e) => err(e),
            }
        }
        "webrain_table" => {
            // browsemind extract_table: HTML tables -> JSON rows, zero LLM.
            let js = r#"JSON.stringify(Array.from(document.querySelectorAll('table')).slice(0,20).map(t => { const headers = Array.from(t.querySelectorAll('th')).map(th => th.textContent.trim()); const rows = Array.from(t.querySelectorAll('tr')).slice(headers.length ? 1 : 0).map(tr => Array.from(tr.querySelectorAll('td,th')).map(td => td.textContent.trim())); return headers.length ? rows.map(r => Object.fromEntries(headers.map((h,i) => [h, r[i] ?? null]))) : rows; }))"#;
            match backend.evaluate(js).await {
                Ok(v) => {
                    let arr = v
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(Value::Null);
                    json!({"status": "ok", "count": arr.as_array().map(|a| a.len()).unwrap_or(0), "tables": arr})
                }
                Err(e) => err(e),
            }
        }
        "webrain_scan" => {
            // browsemind scan_full_page: auto-scroll to load infinite-scroll content.
            let max_scrolls = args.get("max_scrolls").and_then(|v| v.as_i64()).unwrap_or(15) as usize;
            let js = format!(
                r#"(async()=>{{ const MAX={max_scrolls}; let last=document.body.scrollHeight; let done=0; for(let i=0;i<MAX;i++){{ window.scrollTo(0, document.body.scrollHeight); await new Promise(r=>setTimeout(r,250)); const h=document.body.scrollHeight; done=i+1; if(h===last && i>2) break; last=h; }} return JSON.stringify({{scrolls: done, height: document.body.scrollHeight}}); }})()"#
            );
            match backend.evaluate(&js).await {
                Ok(v) => {
                    let obj = v
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(Value::Null);
                    json!({"status": "ok", "result": obj})
                }
                Err(e) => err(e),
            }
        }
        "webrain_autoschema" => {
            // browsemind auto-detect CSS schema: find repeated container patterns.
            // Returns top candidate base-selectors for the LLM to build a full schema.
            let min_occurrences = args.get("min_occurrences").and_then(|v| v.as_i64()).unwrap_or(3) as usize;
            let js = format!(
                r#"(()=>{{ const c={{}}; document.querySelectorAll('div[class],li,tr,article,section').forEach(el=>{{ const cls = el.className ? '.'+String(el.className).trim().split(/\s+/).slice(0,2).join('.') : ''; const key=el.tagName.toLowerCase()+cls; c[key]=(c[key]||0)+1; }}); return JSON.stringify(Object.entries(c).filter(([k,n])=>n>={min_occurrences}).sort((a,b)=>b[1]-a[1]).slice(0,8).map(([sel,count])=>({{selector:sel, count}}))); }})()"#
            );
            match backend.evaluate(&js).await {
                Ok(v) => {
                    let arr = v
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(Value::Null);
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
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let results = bm25_filter(&items, query, k);
            json!({"status": "ok", "count": results.len(), "results": results})
        }
        "webrain_validate_urls" => {
            // browsemind seed(from_links, validate=True): which URLs are alive vs dead.
            let urls: Vec<String> = args
                .get("urls")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
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
        "webrain_pdf" => {
            match backend.pdf().await {
                Ok(bytes) => {
                    use base64::Engine;
                    json!({"status": "ok", "pdf_b64": base64::engine::general_purpose::STANDARD.encode(bytes)})
                }
                Err(e) => err(e),
            }
        }
        "webrain_tab" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("list");
            match action {
                "new" => {
                    let url = args.get("url").and_then(|v| v.as_str()).unwrap_or("about:blank");
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
        "webrain_a11y" => match backend.a11y().await {
            Ok(nodes) => json!({"status": "ok", "nodes": nodes}),
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
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            if urls.is_empty() {
                return json!({"status": "error", "message": "urls required"});
            }
            let concurrency = args.get("concurrency").and_then(|v| v.as_i64()).unwrap_or(4) as usize;
            let opts = nav_opts(args);

            // The per-backend op dispatcher — shared by the single-backend path and
            // the multi-backend round-robin so a fix here covers every caller.
            async fn run_batch(
                b: &CdpBackend, op: &str, urls: &[String], args: &Value,
                opts: &webrain_core::backends::cdp::NavOpts, concurrency: usize,
            ) -> Vec<BatchResult> {
                let base = args.get("base_selector").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let base_fields = args.get("base_fields").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                let fields = args.get("fields").and_then(|v| v.as_array()).cloned().unwrap_or_default();
                match op {
                    "extract" => batch_extract(b, urls, &base, &base_fields, &fields, concurrency, opts).await,
                    "interact" => {
                        let interaction = args.get("interaction").and_then(|v| v.as_str()).unwrap_or("").to_string();
                        batch_interact(b, urls, &interaction, &base, &base_fields, &fields, concurrency, opts).await
                    }
                    "screenshot" => {
                        let dir = args.get("dir").and_then(|v| v.as_str()).unwrap_or("screenshots").to_string();
                        batch_screenshot(b, urls, &dir, concurrency, opts).await
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
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
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
                run_batch(backend, &op, &urls, args, &opts, concurrency).await
            } else {
                let mut all = Vec::new();
                let mut backends = Vec::new();
                for u in &cdp_urls {
                    match CdpBackend::connect_with_url(u).await {
                        Ok(b) => backends.push(b),
                        Err(e) => all.push(BatchResult {
                            url: u.clone(), title: String::new(), text: String::new(),
                            error: Some(format!("cdp connect failed: {e}")),
                        }),
                    }
                }
                let n = backends.len().max(1);
                // Round-robin: url[i] -> backend[i % n]. Each backend runs its share
                // at `per_backend_concurrency` tabs; backends themselves run
                // sequentially (connect cost dominates; true cross-backend
                // concurrency is the agent's job).
                for (bi, b) in backends.iter().enumerate() {
                    let share: Vec<String> = urls.iter().enumerate()
                        .filter(|(i, _)| i % n == bi)
                        .map(|(_, u)| u.clone())
                        .collect();
                    if !share.is_empty() {
                        all.extend(run_batch(b, &op, &share, args, &opts, per_backend_concurrency).await);
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
            payload
        }
        "webrain_download" => {
            // Combined download: single or batch. engine 'ytdlp' routes to the
            // installed yt-dlp binary (video/audio/HLS/playlists/features);
            // default 'http' keeps the streaming path (browsemind download_many).
            let mut urls: Vec<String> = args
                .get("urls")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            if urls.is_empty() {
                return json!({"status": "error", "message": "urls required"});
            }
            let dir = args
                .get("dir")
                .and_then(|v| v.as_str())
                .unwrap_or("downloads")
                .to_string();
            if args.get("engine").and_then(|v| v.as_str()).unwrap_or("http") == "ytdlp" {
                // ponytail: shell out to the installed yt-dlp binary — no new Rust
                // dep; `args` passthrough exposes every other yt-dlp flag.
                std::fs::create_dir_all(&dir).ok();
                let mut cmd = std::process::Command::new("yt-dlp");
                cmd.arg("-o").arg(format!("{dir}/%(title)s.%(ext)s"));
                if args.get("audio_only").and_then(|v| v.as_bool()).unwrap_or(false) {
                    cmd.arg("-x").arg("--audio-format").arg("mp3");
                } else if let Some(f) = args
                    .get("format")
                    .and_then(|v| v.as_str())
                    .filter(|f| !f.is_empty())
                {
                    cmd.arg("-f").arg(f);
                } else {
                    cmd.arg("-f").arg("bestvideo*+bestaudio/best");
                }
                if let Some(extra) = args.get("args").and_then(|v| v.as_array()) {
                    for a in extra.iter().filter_map(|x| x.as_str()) {
                        cmd.arg(a);
                    }
                }
                cmd.args(&urls);
                return match cmd.output() {
                    Ok(o) => {
                        let stdout = String::from_utf8_lossy(&o.stdout);
                        let stderr = String::from_utf8_lossy(&o.stderr);
                        let cap = |s: &str| -> String { s.chars().take(2000).collect() };
                        json!({
                            "status": if o.status.success() { "ok" } else { "error" },
                            "exit": o.status.code(),
                            "dir": dir,
                            "message": if o.status.success() { cap(&stdout) } else { cap(&stderr) },
                        })
                    }
                    Err(e) => json!({"status": "error", "message": format!("failed to run yt-dlp: {e}")}),
                };
            }
            // browsemind download_many: narrow to one file type (.mp4/.pdf/.js...).
            if let Some(ext) = args
                .get("filter_extension")
                .and_then(|v| v.as_str())
                .map(|s| s.trim_start_matches('.').to_lowercase())
                .filter(|e| !e.is_empty())
            {
                urls = urls
                    .into_iter()
                    .filter(|u| {
                        let path = u.split('?').next().unwrap_or(u);
                        path.to_lowercase().ends_with(&format!(".{ext}"))
                    })
                    .collect();
                if urls.is_empty() {
                    return json!({"status": "error", "message": format!("no URLs match extension '.{ext}'")});
                }
            }
            let results = download_files(&urls, &dir);
            json!({"status": "ok", "count": results.len(), "results": results})
        }
        "webrain_search" => {
            let q = args.get("q").and_then(|v| v.as_str()).unwrap_or("");
            if q.is_empty() {
                return json!({"status": "error", "message": "q required"});
            }
            let engine = args.get("engine").and_then(|v| v.as_str()).unwrap_or("duckduckgo");
            let encoded = q.replace(' ', "+");
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
                    "challenge": s.challenge
                }),
                Err(e) => err(e),
            }
        }
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
            let js = format!(
                r#"(function() {{
                    const k = '{key}';
                    const map = {{'Enter':'Enter','Tab':'Tab','Escape':'Escape','Backspace':'Backspace','ArrowDown':'ArrowDown','ArrowUp':'ArrowUp','Space':' '}};
                    const key = map[k] || k;
                    const el = document.activeElement || document.body;
                    const opts = {{ key, code: key, bubbles: true, cancelable: true }};
                    el.dispatchEvent(new KeyboardEvent('keydown', opts));
                    el.dispatchEvent(new KeyboardEvent('keyup', opts));
                    if (el.form && k === 'Enter') {{ el.form.dispatchEvent(new Event('submit', {{ bubbles: true, cancelable: true }})); return 'form-submitted'; }}
                    return 'pressed ' + k;
                }})()"#
            );
            match backend.evaluate(&js).await {
                Ok(v) => json!({"status": "ok", "result": v}),
                Err(e) => err(e),
            }
        }
        "webrain_get_images" => {
            let js = r#"JSON.stringify(Array.from(document.images).map(img => ({src: img.currentSrc || img.src, alt: img.alt || '', width: img.naturalWidth, height: img.naturalHeight})).slice(0, 200))"#;
            match backend.evaluate(js).await {
                Ok(v) => {
                    let arr = v
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(Value::Null);
                    json!({"status": "ok", "count": arr.as_array().map(|a| a.len()).unwrap_or(0), "images": arr})
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
                        let obj = v
                            .as_str()
                            .and_then(|s| serde_json::from_str::<Value>(s).ok())
                            .unwrap_or(Value::Null);
                        let media = obj.get("media").cloned().unwrap_or(Value::Null);
                        let total = obj.get("total").and_then(|t| t.as_u64()).unwrap_or(0);
                        json!({"status": "ok", "mode": "performance", "total_resources": total, "count": media.as_array().map(|a| a.len()).unwrap_or(0), "media": media})
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
                    let arr = v
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .unwrap_or(Value::Null);
                    json!({"status": "ok", "count": arr.as_array().map(|a| a.len()).unwrap_or(0), "logs": arr})
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
            let tag = args.get("tag").and_then(|v| v.as_str()).unwrap_or("default").to_string();
            let tw = args.get("tile_width").and_then(|v| v.as_f64()).unwrap_or(800.0);
            let th = args.get("tile_height").and_then(|v| v.as_f64()).unwrap_or(800.0);
            let mt = args.get("max_tiles").and_then(|v| v.as_i64()).unwrap_or(8) as usize;
            match index_current_page(backend, &tag, tw, th, mt).await {
                Ok(v) => v,
                Err(e) => err(e),
            }
        }
        "webrain_vision_retrieve" => {
            let tag = args.get("tag").and_then(|v| v.as_str()).unwrap_or("default").to_string();
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if query.is_empty() {
                return json!({"status": "error", "message": "query required"});
            }
            let k = args.get("k").and_then(|v| v.as_i64()).unwrap_or(5) as usize;
            match vision_retrieve(&tag, &query, k) {
                Ok(v) => v,
                Err(e) => err(e),
            }
        }
        "webrain_clean" => {
            let wt = args.get("word_threshold").and_then(|v| v.as_i64()).unwrap_or(2) as usize;
            let es = args.get("exclude_social").and_then(|v| v.as_bool()).unwrap_or(true);
            let js = build_clean_js(wt, es);
            match backend.evaluate(&js).await {
                Ok(v) => json!({"status": "ok", "text": v}),
                Err(e) => err(e),
            }
        }
        _ => json!({"error": format!("Unknown tool: {name}")}),
    }
}
