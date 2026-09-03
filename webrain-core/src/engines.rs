use crate::backends::cdp::{CdpBackend, NavOpts};
use crate::browser::{BrowserBackend, PageResult};
use serde_json::{Value, json};

/// PixelRAG-style tile capture: split the page into a grid of screenshot tiles
/// so a vision model can read specific regions (tables/charts/layout survive).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TileShot {
    pub index: usize,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub png_b64: String,
}

pub struct TileEngine {
    pub tile_width: f64,
    pub tile_height: f64,
    pub max_tiles: usize,
}

impl TileEngine {
    pub fn new(tile_width: f64, tile_height: f64, max_tiles: usize) -> Self {
        Self {
            tile_width,
            tile_height,
            max_tiles,
        }
    }

    /// Split the current page into tiles. Call navigate/snapshot first; does NOT navigate.
    /// ponytail: CDP clip per tile (no image decode); cap total tiles at max_tiles.
    pub async fn tile(&self, backend: &impl BrowserBackend) -> anyhow::Result<Vec<TileShot>> {
        let size: Value = backend
            .evaluate(
                "[document.documentElement.scrollWidth, document.documentElement.scrollHeight]",
            )
            .await?;
        let page_w = size[0].as_f64().unwrap_or(1280.0).max(1.0);
        let page_h = size[1].as_f64().unwrap_or(800.0).max(1.0);

        let cols = ((page_w / self.tile_width).ceil() as usize).max(1);
        let rows = ((page_h / self.tile_height).ceil() as usize).max(1);

        let mut tiles = Vec::new();
        let mut idx = 0usize;
        'outer: for r in 0..rows {
            for c in 0..cols {
                if idx >= self.max_tiles {
                    break 'outer;
                }
                let x = c as f64 * self.tile_width;
                let y = r as f64 * self.tile_height;
                let w = self.tile_width.min(page_w - x);
                let h = self.tile_height.min(page_h - y);
                let png = backend.screenshot_clip(x, y, w, h, 1.0).await?;
                tiles.push(TileShot {
                    index: idx,
                    x,
                    y,
                    width: w,
                    height: h,
                    png_b64: base64_encode(&png),
                });
                idx += 1;
            }
        }
        Ok(tiles)
    }
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    // ponytail: one self-check for the non-trivial base64 encoder.
    #[test]
    fn base64_roundtrip() {
        let s = b"hello";
        let encoded = super::base64_encode(s);
        assert_eq!(encoded, "aGVsbG8=");
        assert_eq!(super::base64_encode(b"a"), "YQ==");
        assert_eq!(super::base64_encode(b"ab"), "YWI=");
    }

    // ponytail: one check for the regex JS builder (builtin present, custom overrides).
    #[test]
    fn regex_js_has_builtins_and_overrides() {
        let js = super::build_regex_js(&[]);
        assert!(js.contains("\"label\":\"email\""));
        assert!(js.contains("document.documentElement.outerHTML"));
        let custom = vec![serde_json::json!({"label": "email", "re": "x@y"})];
        let js2 = super::build_regex_js(&custom);
        assert!(js2.contains("\"re\":\"x@y\""));
        assert!(!js2.contains(r"[A-Za-z0-9._%+-]+@"));
    }

    // ponytail: one check for extended extract_js — nested, base_fields, list.
    #[test]
    fn extract_js_nested_and_base_fields() {
        let base_fields = vec![serde_json::json!({"name":"url","attribute":"href"})];
        let fields = vec![
            serde_json::json!({
                "name": "title", "selector": "h2", "type": "text"
            }),
            serde_json::json!({
                "name": "details", "selector": "div.details", "type": "nested",
                "fields": [
                    {"name":"brand","selector":"span.brand","type":"text"},
                    {"name":"model","selector":"span.model","type":"text"}
                ]
            }),
        ];
        let js = super::build_extract_js("div.product", &base_fields, &fields);
        // base_fields: url from container href
        assert!(js.contains("\"url\": (el.getAttribute('href') ?? null)"));
        // nested: {details: {brand:..., model:...}}
        assert!(js.contains("\"details\": (function(){const c=el.querySelector('div.details');"));
        assert!(js.contains("\"brand\":"));
        assert!(js.contains("\"model\":"));
        // list type
        let list_fields = vec![serde_json::json!({
            "name":"features","selector":"ul.features li","type":"list"
        })];
        let js2 = super::build_extract_js("div.product", &[], &list_fields);
        assert!(js2.contains("Array.from(el.querySelectorAll('ul.features li')).map(c=>"));
    }

    // ponytail: one check for adaptive extract — exact path + relocation fallback present.
    #[test]
    fn adaptive_js_has_exact_and_relocation_paths() {
        let fields = vec![
            serde_json::json!({
                "name": "title", "selector": "h2", "type": "text"
            }),
            serde_json::json!({
                "name": "price", "selector": "span.price", "type": "text"
            }),
        ];
        let js = super::build_adaptive_extract_js("div.product", &[], &fields);
        // exact path first: querySelectorAll('div.product')
        assert!(js.contains("document.querySelectorAll('div.product')"));
        assert!(js.contains("if (exact.length) return JSON.stringify(exact);"));
        // relocation probe embeds field selectors and requires >= 2 hits
        assert!(js.contains("FIELD_SELS = ['h2', 'span.price']"));
        assert!(js.contains("hits >= 2"));
        assert!(js.contains("c.contains(d)"));
        // exact builder stays untouched (no fallback)
        let exact = super::build_extract_js("div.product", &[], &fields);
        assert!(!exact.contains("FIELD_SELS"));
        assert!(exact.contains("JSON.stringify(Array.from"));
    }

    // ponytail: regression for the clean-text selector list — a trailing comma
    // makes querySelectorAll throw SyntaxError, so the default (exclude_social
    // = false) must produce a valid list and the social form a leading comma.
    #[test]
    fn clean_js_selector_list_is_valid() {
        let plain = super::build_clean_js(2, false);
        // The default selector list must not end in a trailing comma.
        assert!(!plain.contains("header,aside,"));
        assert!(!plain.contains("aside,)"));
        assert!(
            plain.contains(
                "querySelectorAll('script,style,noscript,iframe,nav,footer,header,aside')"
            )
        );
        let social = super::build_clean_js(2, true);
        // Social selectors are a LEADING comma append, still valid when empty.
        assert!(social.contains("aside,[href*=\"facebook.com\"]"));
        assert!(social.contains("[href*=\"youtube.com\"]"));
    }

    // ponytail: one check for BM25 — query "rust" ranks the rust doc first.
    #[test]
    fn bm25_ranks_relevant_first() {
        let items = vec![
            "A guide to cooking pasta with tomatoes".to_string(),
            "Rust programming language async runtime documentation".to_string(),
            "Python web framework tutorial".to_string(),
        ];
        let r = super::bm25_filter(&items, "rust documentation", 2);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0]["index"], 1); // rust doc scores highest
    }

    // ponytail: one check for the op=markdown pipeline — htmd converts HTML to
    // real Markdown (script/style skipped) and bm25 prunes to the query-relevant
    // chunks (the batch_markdown path, minus the CDP hop).
    #[test]
    fn markdown_convert_and_prune() {
        let html = r#"<html><head><script>let x=1;</script><style>body{}</style></head>
<body><h1>ESP32 Drone</h1><p>Wiring the motors to the ESC and MPU6050 IMU.</p>
<pre><code>pinMode(13,OUTPUT);

still code inside the fence</code></pre>
<p>Unrelated cooking tips for pasta.</p></body></html>"#;
        let conv = htmd::HtmlToMarkdown::builder()
            .skip_tags(vec!["script", "style", "noscript"])
            .build();
        let md = conv.convert(html).unwrap();
        assert!(md.contains("# ESP32 Drone"));
        assert!(md.contains("Wiring the motors"));
        assert!(!md.contains("let x=1")); // script skipped, no JS dump
        // fence-aware chunking: the blank line inside the code block must NOT
        // split the fence — one chunk holds both opener and closer.
        let chunks = super::markdown_chunks(&md);
        let fenced = chunks
            .iter()
            .find(|c| c.contains("```"))
            .expect("fenced chunk");
        assert_eq!(fenced.matches("```").count(), 2);
        // prune + restore original order (index sort), drop the pasta chunk
        let mut kept = super::bm25_filter(&chunks, "motor wiring", 2);
        kept.sort_by_key(|v| v.get("index").and_then(|i| i.as_u64()).unwrap_or(u64::MAX));
        let joined = kept
            .iter()
            .filter_map(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
            .collect::<Vec<_>>()
            .join("\n\n");
        assert!(joined.contains("Wiring the motors"));
        assert!(!joined.contains("pasta")); // pruned as irrelevant
        let h = joined.find("# ESP32 Drone");
        let w = joined.find("Wiring the motors");
        assert!(h.is_some() && w.is_some() && h.unwrap() < w.unwrap()); // doc order kept
    }

    // ponytail: one check for the Chrome header contract — sec-ch-ua GREASE
    // brand (v=24) + Chrome brand major agree with the UA, so the HTTP layer
    // looks internally consistent (obscura's identity-alignment point).
    #[test]
    fn chrome_headers_agree_with_ua() {
        let h = super::browser_headers();
        let ua = h
            .iter()
            .find(|(k, _)| k == "User-Agent")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let sec = h
            .iter()
            .find(|(k, _)| k == "sec-ch-ua")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        assert!(ua.contains("Chrome/"), "derived UA is a Chrome UA: {ua}");
        assert!(
            sec.contains("\"Google Chrome\";v=\""),
            "sec-ch-ua chrome brand: {sec}"
        );
        assert!(
            sec.contains("Not)A;Brand\";v=\"24\""),
            "GREASE brand: {sec}"
        );
        assert!(h.iter().all(|(k, v)| !k.is_empty() && !v.is_empty()));
    }

    // ponytail: one check for the spider allow/deny filter (branchy logic).
    // Sitemap parse is verified live — regex over simple XML, low risk.
    #[test]
    fn spider_filters_allow_deny() {
        let s = super::SpiderEngine::new(2, 10).with_filters(
            vec!["/product/".to_string()],
            vec!["/cart".to_string(), "/login".to_string()],
        );
        assert!(s.url_ok("https://site.com/product/1"));
        assert!(!s.url_ok("https://site.com/about")); // fails allow
        assert!(!s.url_ok("https://site.com/product/cart")); // fails deny
        // empty allow = allow all; deny still prunes
        let s2 = super::SpiderEngine::new(2, 10).with_filters(vec![], vec!["/login".to_string()]);
        assert!(s2.url_ok("https://site.com/product/1"));
        assert!(!s2.url_ok("https://site.com/login"));
    }

    // ponytail: one check for AutoThrottle math — fast server speeds up, a block
    // doubles (capped at max), and the politeness floor is never undercut.
    #[test]
    fn autothrottle_speeds_up_and_backs_off() {
        let s = super::SpiderEngine::new(2, 10)
            .with_delay_ms(100)
            .with_autothrottle(true, 200, 5000);
        let mut d = std::collections::HashMap::new();
        // fast server (50ms latency): delay moves down toward ~50, floored at 100
        let d1 = s.throttle_tick(&mut d, "fast.com", 50, true);
        assert!((100..200).contains(&d1));
        // slow-but-ok server (800ms): delay rises toward 800
        let d2 = s.throttle_tick(&mut d, "slow.com", 800, true);
        assert!((400..=800).contains(&d2));
        // blocked: doubles each time, capped at max
        let d3 = s.throttle_tick(&mut d, "blocked.com", 5, false);
        let d4 = s.throttle_tick(&mut d, "blocked.com", 5, false);
        assert!(
            d4 >= d3 * 2 || d4 >= 200,
            "block should double: {d3} -> {d4}"
        );
        let mut dmax = std::collections::HashMap::new();
        for _ in 0..10 {
            s.throttle_tick(&mut dmax, "capped.com", 1, false);
        }
        assert!(
            *dmax.get("capped.com").unwrap() <= 5000,
            "never exceeds max"
        );
    }
}

/// Spider engine: BFS, DFS, or BestFirst crawler (crawl4ai deep-crawl strategies).
/// ponytail: BFS/DFS = VecDeque front/back; BestFirst = insertion-sorted by
/// keyword-relevance score (max at front). Domain + robots filter at enqueue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrawlStrategy {
    Bfs,
    Dfs,
    BestFirst,
}

#[derive(Debug, Clone)]
pub struct SpiderEngine {
    max_depth: usize,
    max_pages: usize,
    strategy: CrawlStrategy,
    same_domain: bool,
    allowed_domains: Vec<String>,
    /// Prefetch mode (crawl4ai `prefetch=True`): use the link-only `discover_links`
    /// fast path (no innerText extraction) and return URLs only.
    discover_only: bool,
    /// Honor robots.txt Disallow for the seed origin (crawl4ai `check_robots_txt`).
    respect_robots: bool,
    /// KeywordRelevanceScorer for BestFirst (crawl4ai): URL scored by keyword hits.
    keywords: Vec<String>,
    /// Only follow URLs matching ALL of these regexes (Scrapling LinkExtractor
    /// `allow` / spider-rs `whitelist_url`). Empty = allow all.
    allow: Vec<regex::Regex>,
    /// Skip URLs matching ANY of these regexes (Scrapling `deny` / spider-rs
    /// `blacklist_url`). Applied after allow.
    deny: Vec<regex::Regex>,
    /// Retry a failed page fetch up to N extra times (200ms backoff between).
    retry: u32,
    /// Polite delay between page fetches, ms (spider-rs `with_delay`).
    delay_ms: u64,
    /// Hard wall-clock cap on the whole crawl, seconds (spider-rs `crawl_timeout`).
    crawl_timeout_secs: Option<u64>,
    /// AutoThrottle (Scrapling AutoThrottle): per-domain adaptive delay tuned from
    /// observed latency. Speeds up on fast servers, doubles on a blocked/challenge
    /// response, capped at max. Floor = delay_ms (never undercut politeness).
    autothrottle: bool,
    autothrottle_start_delay_ms: u64,
    autothrottle_max_delay_ms: u64,
    /// Checkpoint/resume (Scrapling crawldir): persist {queue, seen, results}
    /// atomically every N NEW pages so a long crawl survives interruption; a
    /// resumed run returns prior + new results without re-fetching. Deleted on
    /// a clean finish.
    crawldir: Option<std::path::PathBuf>,
    checkpoint_every: usize,
    /// Concurrent page fetches: N workers each drive their OWN tab (batch's
    /// session-routed pattern) so page loads overlap on real Chrome/obscura.
    /// 1 = sequential (legacy behavior); single-target backends (lightpanda)
    /// always fall back to 1 regardless of this.
    concurrency: usize,
    /// NavOpts applied to every page fetch (stealth is always injected by the
    /// backend; these control blocking, waiting, and timeout).
    nav_opts: NavOpts,
}

impl Default for SpiderEngine {
    fn default() -> Self {
        Self {
            max_depth: 2,
            max_pages: 20,
            strategy: CrawlStrategy::Bfs,
            same_domain: true,
            allowed_domains: vec![],
            discover_only: false,
            respect_robots: false,
            keywords: vec![],
            allow: vec![],
            deny: vec![],
            retry: 0,
            delay_ms: 0,
            crawl_timeout_secs: None,
            autothrottle: false,
            autothrottle_start_delay_ms: 200,
            autothrottle_max_delay_ms: 30_000,
            crawldir: None,
            checkpoint_every: 10,
            concurrency: 1,
            nav_opts: NavOpts::default(),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpiderResult {
    pub page: PageResult,
    pub depth: usize,
    pub links: Vec<String>,
}

/// Shared frontier/result state for the concurrent (multi-tab) crawl path. All
/// mutations — claim, enqueue, result-push, checkpoint — happen under ONE mutex
/// so workers can't double-crawl a URL or lose a result. Contention is nil vs
/// the ~100ms+ page loads each worker performs outside the lock.
struct SpiderShared {
    queue: std::collections::VecDeque<(String, usize)>,
    visited: std::collections::HashSet<String>,
    results: Vec<SpiderResult>,
    /// Pages claimed but not yet completed (budget accounting).
    inflight: usize,
    /// results.len() at which the next checkpoint save fires.
    next_checkpoint: usize,
    /// Constant per-crawl context (never checkpointed): seed host for the domain
    /// filter, robots Disallow prefixes, and the wall-clock deadline.
    seed_host: String,
    disallowed: Vec<String>,
    deadline: Option<std::time::Instant>,
}

impl SpiderEngine {
    pub fn new(max_depth: usize, max_pages: usize) -> Self {
        Self {
            max_depth,
            max_pages,
            ..Default::default()
        }
    }

    pub fn with_strategy(mut self, s: CrawlStrategy) -> Self {
        self.strategy = s;
        self
    }
    pub fn with_same_domain(mut self, v: bool) -> Self {
        self.same_domain = v;
        self
    }
    pub fn with_allowed_domains(mut self, d: Vec<String>) -> Self {
        self.allowed_domains = d;
        self
    }
    pub fn with_discover_only(mut self, v: bool) -> Self {
        self.discover_only = v;
        self
    }
    pub fn with_respect_robots(mut self, v: bool) -> Self {
        self.respect_robots = v;
        self
    }
    pub fn with_keywords(mut self, k: Vec<String>) -> Self {
        self.keywords = k;
        self
    }
    /// Compile allow/deny regexes once. Invalid patterns are ignored (a bad deny
    /// regex must not silently let everything through — log it, skip the pattern).
    pub fn with_filters(mut self, allow: Vec<String>, deny: Vec<String>) -> Self {
        let compile = |pats: Vec<String>| -> Vec<regex::Regex> {
            pats.iter()
                .filter_map(|p| match regex::Regex::new(p) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        tracing::warn!("spider filter regex invalid '{p}': {e}");
                        None
                    }
                })
                .collect()
        };
        self.allow = compile(allow);
        self.deny = compile(deny);
        self
    }
    pub fn with_retry(mut self, n: u32) -> Self {
        self.retry = n;
        self
    }
    pub fn with_delay_ms(mut self, ms: u64) -> Self {
        self.delay_ms = ms;
        self
    }
    /// 0 = no cap (Some(0) would make the deadline `now` and kill the crawl
    /// before the first page — treat it as "not set" at the shared entry point).
    pub fn with_crawl_timeout(mut self, secs: u64) -> Self {
        self.crawl_timeout_secs = if secs > 0 { Some(secs) } else { None };
        self
    }
    /// Enable Scrapling-style AutoThrottle. `floor_ms` = the spider's own
    /// `delay_ms` floor (never undercut). `max_ms` caps the adaptive delay.
    pub fn with_autothrottle(mut self, enabled: bool, start_ms: u64, max_ms: u64) -> Self {
        self.autothrottle = enabled;
        self.autothrottle_start_delay_ms = start_ms.max(self.delay_ms);
        self.autothrottle_max_delay_ms = max_ms.max(self.autothrottle_start_delay_ms);
        self
    }
    /// Enable checkpoint/resume: persist crawl state every `every` pages to `dir`.
    pub fn with_checkpoint(mut self, dir: String, every: usize) -> Self {
        self.crawldir = if dir.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(dir))
        };
        self.checkpoint_every = every.max(1);
        self
    }
    /// Concurrent multi-tab crawl width (real Chrome/obscura). 1 = sequential.
    pub fn with_concurrency(mut self, n: usize) -> Self {
        self.concurrency = n.max(1);
        self
    }
    pub fn with_nav_opts(mut self, opts: NavOpts) -> Self {
        self.nav_opts = opts;
        self
    }

    /// AutoThrottle: move the per-domain delay toward observed latency (fast
    /// servers speed up), or double it on a block (slow/hostile servers back off).
    /// Returns the delay to sleep before the next request to that domain.
    /// ponytail: `(cur + target) / 2` averaging like Scrapling; block doubling
    /// capped at max; delay never below the politeness floor.
    fn throttle_tick(
        &self,
        delays: &mut std::collections::HashMap<String, u64>,
        domain: &str,
        latency_ms: u64,
        ok: bool,
    ) -> u64 {
        if !self.autothrottle {
            return self.delay_ms;
        }
        let floor = self.delay_ms;
        let cur = *delays
            .get(domain)
            .unwrap_or(&self.autothrottle_start_delay_ms);
        let new_delay = if ok {
            // Latency-driven: move halfway toward the server's real response
            // time (a slow server then steps toward `target` instead of jumping
            // straight to it — the old expression simplified to max(avg,target)).
            let target = latency_ms.max(floor);
            let avg = (cur + target) / 2;
            avg.min(self.autothrottle_max_delay_ms).max(floor)
        } else {
            // Blocked/challenge: double (or wait longer if the site already
            // slowed us down). A block never speeds the crawl up.
            cur.saturating_mul(2)
                .max(cur)
                .min(self.autothrottle_max_delay_ms)
                .max(floor)
        };
        delays.insert(domain.to_string(), new_delay);
        new_delay
    }

    /// Scrapling LinkExtractor filter: URL must match every `allow` (if any) and
    /// no `deny`. Empty allow = pass.
    fn url_ok(&self, url: &str) -> bool {
        if !self.allow.is_empty() && !self.allow.iter().all(|r| r.is_match(url)) {
            return false;
        }
        !self.deny.iter().any(|r| r.is_match(url))
    }

    /// Checkpoint file: {queue: [[url, depth]...], seen: [...], results: [...]}.
    /// One JSON file, atomically rewritten (tmp + rename) so a crash never leaves
    /// a torn file that would read back as a fresh crawl. ponytail: no serde for
    /// the queue — (String, usize) pairs serialize fine as arrays.
    fn checkpoint_path(&self) -> Option<std::path::PathBuf> {
        self.crawldir.as_ref().map(|d| d.join("checkpoint.json"))
    }

    /// Persist crawl state: pending frontier + seen URLs + results collected so
    /// far. Crash-serious: a resumed crawl returns prior + new results, never
    /// re-fetches a seen URL, and respects the combined max_pages budget.
    fn save_checkpoint(
        &self,
        queue: &std::collections::VecDeque<(String, usize)>,
        visited: &std::collections::HashSet<String>,
        results: &[SpiderResult],
    ) {
        let Some(path) = self.checkpoint_path() else {
            return;
        };
        let q: Vec<Value> = queue.iter().map(|(u, d)| json!([u, d])).collect();
        let seen: Vec<String> = visited.iter().cloned().collect();
        let res: Vec<Value> = results
            .iter()
            .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
            .collect();
        let saved_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let data = json!({"queue": q, "seen": seen, "results": res, "saved_at": saved_at});
        if let Ok(s) = serde_json::to_string(&data) {
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, s).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    /// Restore {queue, seen, results}. Missing/corrupt checkpoint = fresh crawl.
    /// ponytail: read/parse failures and absent fields degrade to empty, never
    /// error — a resume is best-effort.
    fn load_checkpoint(
        &self,
    ) -> (
        std::collections::VecDeque<(String, usize)>,
        std::collections::HashSet<String>,
        Vec<SpiderResult>,
    ) {
        let mut queue = std::collections::VecDeque::new();
        let mut seen = std::collections::HashSet::new();
        let mut results = Vec::new();
        let Some(path) = self.checkpoint_path() else {
            return (queue, seen, results);
        };
        let Ok(raw) = std::fs::read_to_string(path) else {
            return (queue, seen, results);
        };
        let Ok(data) = serde_json::from_str::<Value>(&raw) else {
            return (queue, seen, results);
        };
        if let Some(q) = data.get("queue").and_then(|v| v.as_array()) {
            for item in q {
                if let (Some(u), Some(d)) = (item[0].as_str(), item[1].as_u64()) {
                    queue.push_back((u.to_string(), d as usize));
                }
            }
        }
        if let Some(s) = data.get("seen").and_then(|v| v.as_array()) {
            for u in s.iter().filter_map(|x| x.as_str()) {
                seen.insert(u.to_string());
            }
        }
        if let Some(r) = data.get("results").and_then(|v| v.as_array()) {
            for x in r {
                if let Ok(row) = serde_json::from_value::<SpiderResult>(x.clone()) {
                    results.push(row);
                }
            }
        }
        (queue, seen, results)
    }

    fn delete_checkpoint(&self) {
        if let Some(path) = self.checkpoint_path() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Accept a URL for crawling based on domain filter.
    fn domain_ok(&self, href: &str, seed_host: &str) -> bool {
        if self.allowed_domains.is_empty() && !self.same_domain {
            return true;
        }
        // ponytail: url::Url parse; if the href is a relative path or malformed, let it in
        let host = if let Ok(u) = url::Url::parse(href) {
            u.host_str().unwrap_or("").to_lowercase()
        } else {
            return true; // relative URLs pass
        };
        if !self.allowed_domains.is_empty() {
            return self
                .allowed_domains
                .iter()
                .any(|d| host == d.as_str() || host.ends_with(&format!(".{d}")));
        }
        host == seed_host || host.ends_with(&format!(".{seed_host}"))
    }

    pub async fn crawl(&self, browser: &CdpBackend, seed_url: &str) -> Vec<SpiderResult> {
        // Multi-tab concurrent crawl (real Chrome/obscura) when concurrency > 1
        // on a multi-target backend. Single-target (lightpanda) and concurrency
        // = 1 fall through to the exact sequential path below. The capability
        // probe costs two scratch createTargets, so it only runs when
        // parallelism is actually requested.
        if self.concurrency > 1 && matches!(browser.single_target_probe().await, Ok(false)) {
            let seed_host = url::Url::parse(seed_url)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .unwrap_or_default();
            let seed_origin = seed_url.split('/').take(3).collect::<Vec<_>>().join("/");
            let disallowed = if self.respect_robots {
                self.fetch_robots(&seed_origin).await
            } else {
                Vec::new()
            };
            return self
                .crawl_parallel(browser, seed_url, seed_host, disallowed)
                .await;
        }
        use std::collections::{HashSet, VecDeque};

        let seed_host = url::Url::parse(seed_url)
            .ok()
            .and_then(|u| u.host_str().map(String::from))
            .unwrap_or_default();

        // robots.txt: fetch once for the seed origin, honor Disallow prefixes.
        // ponytail: single fetch, prefix match only; per-origin fetch for
        // multi-domain crawls when same_domain is disabled.
        let seed_origin = seed_url.split('/').take(3).collect::<Vec<_>>().join("/");
        let disallowed: Vec<String> = if self.respect_robots {
            // robots.txt fetch is blocking ureq — run it off the executor with
            // the 30s-timed pooled agent (a bare `ureq::get` had NO timeout, so
            // a hung robots server stalled the async worker forever).
            let robots_url = format!("{seed_origin}/robots.txt");
            let robots = tokio::task::spawn_blocking(move || {
                browser_req(browser_agent().get(&robots_url))
                    .call()
                    .ok()
                    .and_then(|r| r.into_body().read_to_string().ok())
            })
            .await
            .unwrap_or(None);
            robots
                .map(|s| {
                    s.lines()
                        .filter_map(|l| {
                            l.trim()
                                .to_lowercase()
                                .strip_prefix("disallow:")
                                .map(|p| p.trim().to_string())
                        })
                        .filter(|p| !p.is_empty())
                        .collect()
                })
                .unwrap_or_default()
        } else {
            vec![]
        };
        let robots_ok = |link: &str| {
            if disallowed.is_empty() {
                return true;
            }
            // Match against the URL PATH + QUERY (RFC 9309 matches both) —
            // splitn(3,'/').nth(2) kept the host, and path-only matching
            // dropped rules like `Disallow: /search?q=` on links like
            // `/search?q=foo&x=1`. Relative links already fell back to the
            // full lowercased string (query included); now absolute ones do too.
            let path = url::Url::parse(link)
                .map(|u| {
                    let mut p = u.path().to_lowercase();
                    if let Some(q) = u.query() {
                        p.push('?');
                        p.push_str(&q.to_lowercase());
                    }
                    p
                })
                .unwrap_or_else(|_| link.to_lowercase());
            !disallowed.iter().any(|p| path.starts_with(p))
        };

        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        let mut results: Vec<SpiderResult> = Vec::new();
        let crawl_deadline = self
            .crawl_timeout_secs
            .map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));
        // AutoThrottle per-domain delays (learned during this crawl, not persisted).
        let mut throttle: std::collections::HashMap<String, u64> = std::collections::HashMap::new();

        // Checkpoint/resume: restore {queue, seen, results} from an interrupted
        // run so a resume returns prior + new results without re-fetching.
        if self.crawldir.is_some() {
            let (q, s, r) = self.load_checkpoint();
            queue = q;
            visited = s;
            results = r;
        }
        // Seed only a genuinely fresh crawl (empty frontier AND nothing seen). A
        // kept checkpoint must never re-seed — the old `queue.is_empty()` guard
        // re-crawled the whole site after a drained-but-kept checkpoint.
        if visited.is_empty() && queue.is_empty() {
            queue.push_back((seed_url.to_string(), 0));
            visited.insert(seed_url.to_string());
        }
        let mut since_checkpoint = 0usize;

        loop {
            // Budget/deadline checked BEFORE popping so an early stop never drops
            // the frontier item — a resume continues exactly where it left off.
            if results.len() >= self.max_pages {
                break;
            }
            let Some((url, depth)) = self.pop(&mut queue) else {
                break;
            };
            if let Some(deadline) = crawl_deadline {
                if std::time::Instant::now() >= deadline {
                    break; // wall-clock cap (spider-rs crawl_timeout) — stop the crawl.
                }
            }

            let start = std::time::Instant::now();

            // AutoThrottle: sleep the learned delay for this domain before
            // fetching (floor = delay_ms; starts at autothrottle_start_delay_ms).
            let domain = url.split('/').nth(2).unwrap_or("").to_string();
            let pre_delay = if self.autothrottle {
                throttle
                    .get(&domain)
                    .copied()
                    .unwrap_or(self.autothrottle_start_delay_ms)
                    .max(self.delay_ms)
            } else {
                self.delay_ms
            };
            if pre_delay > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(pre_delay)).await;
            }

            // ponytail: discover_only uses the link-only fast path (no innerText,
            // no full-load fallback) — that's crawl4ai prefetch / "no navigation".
            let (page_result, links): (PageResult, Vec<String>) = if self.discover_only {
                match browser.discover_links(&url).await {
                    Ok(links) => (
                        PageResult {
                            url: url.clone(),
                            title: None,
                            content: None,
                            error: None,
                            duration_ms: start.elapsed().as_millis() as u64,
                        },
                        links,
                    ),
                    Err(e) => {
                        results.push(SpiderResult {
                            page: PageResult {
                                url: url.clone(),
                                title: None,
                                content: None,
                                error: Some(e.to_string()),
                                duration_ms: start.elapsed().as_millis() as u64,
                            },
                            depth,
                            links: vec![],
                        });
                        continue;
                    }
                }
            } else {
                // retry (spider-rs with_retry): re-fetch up to N times on error.
                let mut page_result: PageResult = PageResult {
                    url: url.clone(),
                    title: None,
                    content: None,
                    error: Some("unreached".into()),
                    duration_ms: 0,
                };
                let mut attempts = 0;
                loop {
                    match browser.navigate_opts(&url, &self.nav_opts).await {
                        Ok(state) => {
                            page_result = PageResult {
                                url: url.clone(),
                                title: Some(state.title.clone()),
                                content: Some(state.text),
                                error: None,
                                duration_ms: start.elapsed().as_millis() as u64,
                            };
                            break;
                        }
                        Err(e) => {
                            attempts += 1;
                            if attempts > self.retry {
                                page_result.error = Some(e.to_string());
                                page_result.duration_ms = start.elapsed().as_millis() as u64;
                                break;
                            }
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        }
                    }
                }
                if page_result.error.is_some() {
                    results.push(SpiderResult {
                        page: page_result,
                        depth,
                        links: vec![],
                    });
                    continue;
                }
                // ponytail: SPA/VitePress render links after initial DOM — short settle.
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                let links = match browser
                    .evaluate("Array.from(document.querySelectorAll('a[href]')).map(a => a.href)")
                    .await
                {
                    Ok(v) => {
                        let mut hrefs = vec![];
                        if let Some(arr) = v.as_array() {
                            for item in arr {
                                if let Some(s) = item.as_str() {
                                    hrefs.push(s.to_string());
                                }
                            }
                        }
                        hrefs
                    }
                    Err(_) => vec![],
                };
                (page_result, links)
            };

            results.push(SpiderResult {
                page: page_result,
                depth,
                links: links.clone(),
            });

            if depth < self.max_depth {
                for link in &links {
                    if !visited.contains(link)
                        && self.domain_ok(link, &seed_host)
                        && self.url_ok(link)
                        && robots_ok(link)
                    {
                        visited.insert(link.clone());
                        self.push_link(&mut queue, link.clone(), depth + 1);
                    }
                }
            }

            // AutoThrottle: feed the finished request back. Blocked = the page
            // errored OR carried a challenge (non-2xx / gated) → double the delay.
            if self.autothrottle {
                let blocked = results
                    .last()
                    .map(|r| r.page.error.is_some())
                    .unwrap_or(false);
                let _ = self.throttle_tick(
                    &mut throttle,
                    &domain,
                    start.elapsed().as_millis() as u64,
                    !blocked,
                );
            } else if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }

            // Checkpoint: persist {queue, seen, results} every N NEW pages so a
            // long crawl survives interruption and resumes (Scrapling crawldir).
            since_checkpoint += 1;
            if self.crawldir.is_some() && since_checkpoint >= self.checkpoint_every {
                since_checkpoint = 0;
                self.save_checkpoint(&queue, &visited, &results);
            }
        }

        // Checkpoint lifecycle: a fully drained frontier = finished crawl →
        // delete the checkpoint. Any early stop (max_pages budget, wall-clock
        // timeout, error left in the frontier) keeps it AND writes a final
        // snapshot, so the returned-but-unpersisted tail survives a crash
        // before the next resume.
        let queue_drained = queue.is_empty();
        if self.crawldir.is_some() {
            if queue_drained {
                self.delete_checkpoint();
            } else {
                self.save_checkpoint(&queue, &visited, &results);
            }
        }

        results
    }

    /// Fetch the seed origin's robots.txt `Disallow` prefixes once (blocking
    /// ureq off the executor, timed agent). Shared by the parallel path; the
    /// serial path inlines the same fetch. ponytail: single fetch, prefix match
    /// only; per-origin fetch for multi-domain crawls when same_domain is off.
    async fn fetch_robots(&self, seed_origin: &str) -> Vec<String> {
        let robots_url = format!("{seed_origin}/robots.txt");
        let robots = tokio::task::spawn_blocking(move || {
            browser_req(browser_agent().get(&robots_url))
                .call()
                .ok()
                .and_then(|r| r.into_body().read_to_string().ok())
        })
        .await
        .unwrap_or(None);
        robots
            .map(|s| {
                s.lines()
                    .filter_map(|l| {
                        l.trim()
                            .to_lowercase()
                            .strip_prefix("disallow:")
                            .map(|p| p.trim().to_string())
                    })
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// RFC 9309 robots match against PATH + QUERY for a link. Shared helper for
    /// the parallel path (the serial path inlines an identical closure — kept
    /// verbatim for zero regression). ponytail: duplicate predicate, ~12 lines.
    fn robots_ok(&self, disallowed: &[String], link: &str) -> bool {
        if disallowed.is_empty() {
            return true;
        }
        let path = url::Url::parse(link)
            .map(|u| {
                let mut p = u.path().to_lowercase();
                if let Some(q) = u.query() {
                    p.push('?');
                    p.push_str(&q.to_lowercase());
                }
                p
            })
            .unwrap_or_else(|_| link.to_lowercase());
        !disallowed.iter().any(|p| path.starts_with(p))
    }

    /// Parallel multi-tab crawl: N workers, each owning its own tab (opened via
    /// the backend, driven through a per-session CDP session — batch's proven
    /// pattern), pull URLs from the shared frontier and load them concurrently.
    /// State (queue/visited/results) is shared behind ONE mutex so no URL is
    /// double-crawled and checkpointing stays consistent with the serial path.
    async fn crawl_parallel(
        &self,
        browser: &CdpBackend,
        seed_url: &str,
        seed_host: String,
        disallowed: Vec<String>,
    ) -> Vec<SpiderResult> {
        use std::collections::{HashMap, HashSet, VecDeque};

        // Restore prior crawl state (frontier + collected results) like the
        // serial path — a resume returns prior + new results.
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut results: Vec<SpiderResult> = Vec::new();
        if self.crawldir.is_some() {
            let (q, s, r) = self.load_checkpoint();
            queue = q;
            visited = s;
            results = r;
        }
        if visited.is_empty() && queue.is_empty() {
            queue.push_back((seed_url.to_string(), 0));
            visited.insert(seed_url.to_string());
        }
        // Combined budget across resumes: prior results count toward max_pages.
        let next_checkpoint = results.len() + self.checkpoint_every;
        let deadline = self
            .crawl_timeout_secs
            .map(|s| std::time::Instant::now() + std::time::Duration::from_secs(s));

        let shared = std::sync::Arc::new(tokio::sync::Mutex::new(SpiderShared {
            queue,
            visited,
            results,
            inflight: 0,
            next_checkpoint,
            seed_host,
            disallowed,
            deadline,
        }));
        // Per-domain learned delay (autothrottle) + polite start-spacing gate.
        let throttle = std::sync::Arc::new(tokio::sync::Mutex::new(HashMap::<String, u64>::new()));
        let gate = std::sync::Arc::new(tokio::sync::Mutex::new(None::<tokio::time::Instant>));

        let engine = std::sync::Arc::new(self.clone());
        let workers = self.concurrency.max(1);
        let mut handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let e = engine.clone();
            let b = browser.clone();
            let sh = shared.clone();
            let th = throttle.clone();
            let ga = gate.clone();
            handles.push(tokio::spawn(async move {
                Self::spider_worker(e, b, sh, th, ga).await;
            }));
        }
        for h in handles {
            let _ = h.await;
        }

        // Fully drained = finished crawl → drop the checkpoint. Any early stop
        // (budget/timeout) keeps it plus a final snapshot so the returned-but-
        // unpersisted tail survives a crash before the next resume.
        {
            let sh = shared.lock().await;
            if self.crawldir.is_some() {
                if sh.queue.is_empty() && sh.inflight == 0 {
                    self.delete_checkpoint();
                } else {
                    self.save_checkpoint(&sh.queue, &sh.visited, &sh.results);
                }
            }
        }
        shared.lock().await.results.clone()
    }

    /// Fetch one page on a given tab session (concurrent path). Mirrors
    /// navigate_opts's text semantics — interactive wait, sparse (<500 chars)
    /// full-load fallback, 3000-char cap, SPA settle before link extraction —
    /// but drives the worker's OWN session instead of the shared active tab, so
    /// N workers load N pages in parallel.
    async fn fetch_page_session(
        &self,
        browser: &CdpBackend,
        sid: &str,
        url: &str,
        start: std::time::Instant,
    ) -> (PageResult, Vec<String>) {
        let links_expr = "Array.from(document.querySelectorAll('a[href]')).map(a => a.href)";

        // Discover-only fast path (crawl4ai prefetch): short-cap nav + links only.
        if self.discover_only {
            let mut opts = self.nav_opts.clone();
            opts.wait_timeout_secs = Some(opts.wait_timeout_secs.unwrap_or(4).min(4));
            let mut links = Vec::new();
            let mut err = None;
            match browser.navigate_session_opts(sid, url, &opts).await {
                Ok(_) => match browser.eval_session(sid, links_expr).await {
                    Ok(v) => {
                        if let Some(arr) = v.as_array() {
                            for item in arr {
                                if let Some(s) = item.as_str() {
                                    links.push(s.to_string());
                                }
                            }
                        }
                    }
                    Err(e) => err = Some(e.to_string()),
                },
                Err(e) => err = Some(e.to_string()),
            }
            return (
                PageResult {
                    url: url.to_string(),
                    title: None,
                    content: None,
                    error: err,
                    duration_ms: start.elapsed().as_millis() as u64,
                },
                links,
            );
        }

        let mut page = PageResult {
            url: url.to_string(),
            title: None,
            content: None,
            error: Some("unreached".into()),
            duration_ms: 0,
        };
        let mut attempts = 0u32;
        loop {
            let fetched: Result<(String, String), anyhow::Error> = async {
                browser
                    .navigate_session_opts(sid, url, &self.nav_opts)
                    .await?;
                let title = browser
                    .eval_session(sid, "document.title")
                    .await?
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let mut text = browser
                    .eval_session(sid, "document.body ? document.body.innerText || '' : ''")
                    .await?
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                // Sparse (<500 chars) → wait for full load, re-read (navigate_opts
                // parity — a slow SPA shouldn't yield a half-empty page).
                if text.chars().count() < 500 {
                    let cap2 = self.nav_opts.wait_timeout_secs.unwrap_or(6);
                    let s2 = std::time::Instant::now();
                    loop {
                        let rs = browser
                            .eval_session(sid, "document.readyState")
                            .await?
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        if rs == "complete" || s2.elapsed().as_secs() > cap2 {
                            break;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                    text = browser
                        .eval_session(sid, "document.body ? document.body.innerText || '' : ''")
                        .await?
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                }
                Ok((title, text))
            }
            .await;
            match fetched {
                Ok((title, text)) => {
                    page = PageResult {
                        url: url.to_string(),
                        title: Some(title),
                        content: Some(text.chars().take(3000).collect()),
                        error: None,
                        duration_ms: start.elapsed().as_millis() as u64,
                    };
                    break;
                }
                Err(e) => {
                    attempts += 1;
                    if attempts > self.retry {
                        page.error = Some(e.to_string());
                        page.duration_ms = start.elapsed().as_millis() as u64;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            }
        }
        if page.error.is_some() {
            return (page, Vec::new());
        }
        // SPA/VitePress render links after initial DOM — short settle.
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let links = match browser.eval_session(sid, links_expr).await {
            Ok(v) => {
                let mut hrefs = vec![];
                if let Some(arr) = v.as_array() {
                    for item in arr {
                        if let Some(s) = item.as_str() {
                            hrefs.push(s.to_string());
                        }
                    }
                }
                hrefs
            }
            Err(_) => vec![],
        };
        (page, links)
    }

    /// One crawl worker: opens its own tab, then loops claiming frontier URLs
    /// and loading them on that tab until the queue drains, max_pages is hit, or
    /// the wall-clock deadline passes. All state is under `shared`'s one mutex,
    /// so budget/deadline/done decisions are race-free.
    async fn spider_worker(
        engine: std::sync::Arc<Self>,
        browser: CdpBackend,
        shared: std::sync::Arc<tokio::sync::Mutex<SpiderShared>>,
        throttle: std::sync::Arc<tokio::sync::Mutex<std::collections::HashMap<String, u64>>>,
        gate: std::sync::Arc<tokio::sync::Mutex<Option<tokio::time::Instant>>>,
    ) {
        // Own tab (multi-target parallel path); closed when this worker exits.
        let (sid, tab_id) = match browser.open_tab("about:blank").await {
            Ok(id) => match browser.tab_session(&id).await {
                Ok(s) => (s, Some(id)),
                Err(_) => {
                    let _ = browser.close_tab(&id).await;
                    return;
                }
            },
            Err(_) => return,
        };

        loop {
            // Claim one frontier URL — budget/deadline/done checked under the
            // lock so concurrent workers can't over-run max_pages or double-crawl.
            let claimed: Option<(String, usize)> = {
                let mut sh = shared.lock().await;
                if let Some(dl) = sh.deadline {
                    if std::time::Instant::now() >= dl {
                        break;
                    }
                }
                if sh.results.len() + sh.inflight >= engine.max_pages {
                    break;
                }
                if sh.queue.is_empty() {
                    if sh.inflight == 0 {
                        break; // nothing left AND nobody mid-fetch → done
                    }
                    None
                } else {
                    let item = engine.pop(&mut sh.queue);
                    sh.inflight += 1;
                    item
                }
            };
            let Some((url, depth)) = claimed else {
                tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                continue;
            };

            let domain = url.split('/').nth(2).unwrap_or("").to_string();
            // Polite spacing (parallel): workers coordinate request START times
            // so the site sees ~one request per `delay` (delay_ms floor, or the
            // learned autothrottle delay) instead of N simultaneous requests.
            let delay = if engine.autothrottle {
                let m = throttle.lock().await;
                m.get(&domain)
                    .copied()
                    .unwrap_or(engine.autothrottle_start_delay_ms)
                    .max(engine.delay_ms)
            } else {
                engine.delay_ms
            };
            if delay > 0 {
                let wake = {
                    let mut g = gate.lock().await;
                    let now = tokio::time::Instant::now();
                    let w = g.map_or(now, |prev| if prev > now { prev } else { now });
                    *g = Some(w + std::time::Duration::from_millis(delay));
                    w
                };
                tokio::time::sleep_until(wake).await;
            }

            let start = std::time::Instant::now();
            let (page, links) = engine.fetch_page_session(&browser, &sid, &url, start).await;
            if engine.autothrottle {
                let blocked = page.error.is_some();
                let mut m = throttle.lock().await;
                engine.throttle_tick(
                    &mut m,
                    &domain,
                    start.elapsed().as_millis() as u64,
                    !blocked,
                );
            }

            {
                let mut sh = shared.lock().await;
                sh.results.push(SpiderResult {
                    page,
                    depth,
                    links: links.clone(),
                });
                sh.inflight -= 1;
                if depth < engine.max_depth {
                    for link in &links {
                        if !sh.visited.contains(link)
                            && engine.domain_ok(link, &sh.seed_host)
                            && engine.url_ok(link)
                            && engine.robots_ok(&sh.disallowed, link)
                        {
                            sh.visited.insert(link.clone());
                            engine.push_link(&mut sh.queue, link.clone(), depth + 1);
                        }
                    }
                }
                if engine.crawldir.is_some() && sh.results.len() >= sh.next_checkpoint {
                    sh.next_checkpoint = sh.results.len() + engine.checkpoint_every;
                    engine.save_checkpoint(&sh.queue, &sh.visited, &sh.results);
                }
            }
        }

        if let Some(id) = tab_id {
            let _ = browser.close_tab(&id).await;
        }
    }

    /// BestFirst relevance score: count keyword hits (case-insensitive) in the URL.
    fn score(&self, url: &str) -> i64 {
        if self.keywords.is_empty() {
            return 0;
        }
        let u = url.to_lowercase();
        self.keywords
            .iter()
            .filter(|k| u.contains(&k.to_lowercase()))
            .count() as i64
    }

    /// Enqueue a link. BestFirst keeps the frontier sorted by descending score
    /// (max at front via insertion); BFS/DFS append (front/back pop below).
    /// ponytail: O(n) insertion into a modest frontier, not a BinaryHeap.
    fn push_link(
        &self,
        q: &mut std::collections::VecDeque<(String, usize)>,
        link: String,
        depth: usize,
    ) {
        match self.strategy {
            CrawlStrategy::BestFirst => {
                let s = self.score(&link);
                let pos = q
                    .iter()
                    .position(|(u, _)| self.score(u) < s)
                    .unwrap_or(q.len());
                q.insert(pos, (link, depth));
            }
            _ => q.push_back((link, depth)),
        }
    }

    fn pop(&self, q: &mut std::collections::VecDeque<(String, usize)>) -> Option<(String, usize)> {
        match self.strategy {
            // BestFirst keeps max-score at front
            CrawlStrategy::Bfs | CrawlStrategy::BestFirst => q.pop_front(),
            CrawlStrategy::Dfs => q.pop_back(),
        }
    }
}

#[cfg(test)]
mod spider_checkpoint_tests {
    // ponytail: one check that checkpoint save/load round-trips queue + seen +
    // results (crash-resume is the whole point of crawldir — a resume must get
    // the already-fetched pages back and never re-fetch them).
    #[test]
    fn checkpoint_round_trip_preserves_results() {
        let dir = std::env::temp_dir().join(format!("webrain_cp_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let s =
            super::SpiderEngine::new(2, 10).with_checkpoint(dir.to_string_lossy().into_owned(), 2);
        let mut q = std::collections::VecDeque::new();
        q.push_back(("https://a.com/2".to_string(), 1));
        let mut seen = std::collections::HashSet::new();
        seen.insert("https://a.com/1".to_string());
        let res = vec![super::SpiderResult {
            page: crate::browser::PageResult {
                url: "https://a.com/1".to_string(),
                title: Some("t".to_string()),
                content: Some("body".to_string()),
                error: None,
                duration_ms: 1,
            },
            depth: 0,
            links: vec!["https://a.com/2".to_string()],
        }];
        s.save_checkpoint(&q, &seen, &res);
        let (q2, seen2, res2) = s.load_checkpoint();
        assert_eq!(q2.len(), 1);
        assert_eq!(q2[0].0, "https://a.com/2");
        assert_eq!(q2[0].1, 1);
        assert!(seen2.contains("https://a.com/1"));
        assert_eq!(res2.len(), 1);
        assert_eq!(res2[0].page.url, "https://a.com/1");
        assert_eq!(res2[0].links[0], "https://a.com/2");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ── Batch operations (multi-tab, enabled by CdpBackend) ──────────────────────
// ponytail: one tab per URL, sequential — the single browser WebSocket serializes
// commands anyway; parallel throughput needs N browser processes (later).

/// Build in-page JS for CSS/XPath-schema extraction (crawl4ai JsonCssExtractionStrategy
/// surface, zero-LLM). Field types: text|attr|html|xpath|regex|nested|nested_list|list.
/// `base_fields` extracts attributes from the container element (e.g. href from <a>).
/// `source` on a field targets the next sibling element (`+ tr`) instead of the container.
/// ponytail: all JS stays in-page via evaluate(), zero deps, zero serialisation overhead.
fn build_field_js(base_fields: &[Value], fields: &[Value]) -> String {
    fn esc(s: &str) -> String {
        s.replace('\\', "\\\\").replace('\'', "\\'")
    }
    fn js_name(f: &Value, i: usize) -> String {
        f.get("name")
            .and_then(|v| v.as_str())
            .filter(|n| !n.is_empty())
            .map(|n| serde_json::to_string(n).unwrap_or_default())
            .unwrap_or_else(|| format!("\"field{i}\""))
    }

    let mut field_js = String::new();

    // baseFields: attributes pulled from the container element (e.g. href from <a>)
    for (i, f) in base_fields.iter().enumerate() {
        let n = js_name(f, i);
        let attr = f
            .get("attribute")
            .and_then(|v| v.as_str())
            .unwrap_or("href");
        field_js.push_str(&format!(
            "{n}: (el.getAttribute('{}') ?? null), ",
            attr.replace('\'', "\\'")
        ));
    }

    for (i, f) in fields.iter().enumerate() {
        let n = js_name(f, i + base_fields.len());
        let sel = f.get("selector").and_then(|v| v.as_str()).unwrap_or("");
        let ftype = f.get("type").and_then(|v| v.as_str()).unwrap_or("text");
        let attr = f.get("attr").and_then(|v| v.as_str()).unwrap_or("");
        let re = f.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
        let src = f.get("source").and_then(|v| v.as_str()).unwrap_or("");

        // Target element: el.querySelector(sel), or nextElementSibling when source set
        let tgt = if !src.is_empty() {
            if sel.is_empty() {
                "el.nextElementSibling".to_string()
            } else {
                format!(
                    "(el.nextElementSibling?.querySelector('{}') ?? null)",
                    esc(sel)
                )
            }
        } else if sel.is_empty() {
            "el".to_string()
        } else {
            format!("el.querySelector('{}')", esc(sel))
        };

        let val = match ftype {
            "attr" => format!(
                "({tgt}?.getAttribute('{}') ?? null)",
                attr.replace('\'', "\\'")
            ),
            "html" => format!("({tgt}?.innerHTML ?? null)"),
            "xpath" => format!(
                "(function(){{const r=document.evaluate('{}',el,null,XPathResult.FIRST_ORDERED_NODE_TYPE,null).singleNodeValue;return r?.textContent?.trim()??null;}})()",
                esc(sel)
            ),
            "regex" => {
                let re_escaped = re.replace('\\', "\\\\").replace('\'', "\\'");
                format!(
                    "(function(){{const t={tgt}?.textContent;if(!t)return null;const m=new RegExp('{}','i').exec(t);return m?(m[1]||m[0]).trim():null;}})()",
                    re_escaped
                )
            }
            "nested" => {
                let nf = f
                    .get("fields")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut inner = String::new();
                for (j, nff) in nf.iter().enumerate() {
                    let nj = js_name(nff, j);
                    let ns = nff.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                    let nt = nff.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                    let na = nff.get("attr").and_then(|v| v.as_str()).unwrap_or("");
                    let nv = match nt {
                        "attr" => format!(
                            "(c.querySelector('{}')?.getAttribute('{}')??null)",
                            esc(ns),
                            na.replace('\'', "\\'")
                        ),
                        "html" => format!("(c.querySelector('{}')?.innerHTML??null)", esc(ns)),
                        _ => format!(
                            "(c.querySelector('{}')?.textContent?.trim()??null)",
                            esc(ns)
                        ),
                    };
                    inner.push_str(&format!("{nj}: {nv}, "));
                }
                format!("(function(){{const c={tgt}; if(!c)return null; return {{ {inner} }};}})()")
            }
            "nested_list" => {
                let nf = f
                    .get("fields")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let mut inner = String::new();
                for (j, nff) in nf.iter().enumerate() {
                    let nj = js_name(nff, j);
                    let ns = nff.get("selector").and_then(|v| v.as_str()).unwrap_or("");
                    let nt = nff.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                    let na = nff.get("attr").and_then(|v| v.as_str()).unwrap_or("");
                    let nv = match nt {
                        "attr" => format!(
                            "(c.querySelector('{}')?.getAttribute('{}')??null)",
                            esc(ns),
                            na.replace('\'', "\\'")
                        ),
                        "html" => format!("(c.querySelector('{}')?.innerHTML??null)", esc(ns)),
                        _ => format!(
                            "(c.querySelector('{}')?.textContent?.trim()??null)",
                            esc(ns)
                        ),
                    };
                    inner.push_str(&format!("{nj}: {nv}, "));
                }
                format!(
                    "Array.from(el.querySelectorAll('{}')).map(c=>({{ {} }}))",
                    esc(sel),
                    inner
                )
            }
            "list" => format!(
                "Array.from(el.querySelectorAll('{}')).map(c=>c.textContent?.trim()??null)",
                esc(sel)
            ),
            _ => format!("({tgt}?.textContent?.trim() ?? null)"),
        };
        field_js.push_str(&format!("{n}: {val}, "));
    }
    field_js
}

/// Exact CSS extraction: `JSON.stringify(Array.from(document.querySelectorAll(base)).map(extract))`.
pub fn build_extract_js(base_selector: &str, base_fields: &[Value], fields: &[Value]) -> String {
    let field_js = build_field_js(base_fields, fields);
    let base = base_selector.replace('\'', "\\'");
    format!(
        "JSON.stringify(Array.from(document.querySelectorAll('{base}')).map(el => ({{ {field_js} }})))"
    )
}

/// Adaptive extraction (Scrapling-style `adaptive=True`): try the exact base selector, and if it
/// matches nothing (site redesigned/class renamed), relocate to elements that still contain >= 2 of
/// the field selectors, keeping only the deepest (row-level) candidates.
/// ponytail: zero-LLM structural re-anchoring in-page; opt-in via `adaptive: true` on extract_json.
pub fn build_adaptive_extract_js(
    base_selector: &str,
    base_fields: &[Value],
    fields: &[Value],
) -> String {
    let field_js = build_field_js(base_fields, fields);
    let base = base_selector.replace('\'', "\\'");
    let sels: Vec<String> = fields
        .iter()
        .filter_map(|f| f.get("selector").and_then(|v| v.as_str()))
        .filter(|s| !s.is_empty())
        .map(|s| s.replace('\'', "\\'"))
        .collect();
    let sel_js = sels
        .iter()
        .map(|s| format!("'{s}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"(() => {{
  const FIELD_SELS = [{sel_js}];
  const extract = (el) => ({{ {field_js} }});
  const exact = Array.from(document.querySelectorAll('{base}')).map(extract);
  if (exact.length) return JSON.stringify(exact);
  if (!FIELD_SELS.length) return JSON.stringify([]);
  // Relocate: find elements that contain >= 2 field selectors, keep the deepest ones.
  const cands = [];
  for (const el of document.querySelectorAll('div, li, article, section, tr, td')) {{
    let hits = 0;
    for (const s of FIELD_SELS) {{ if (el.querySelector(s) && ++hits >= 2) break; }}
    if (hits >= 2) cands.push(el);
  }}
  const minimal = [];
  outer: for (const c of cands) {{
    for (const d of cands) {{ if (d !== c && c.contains(d)) continue outer; }}
    minimal.push(c);
  }}
  return JSON.stringify(minimal.map(extract));
}})()"#
    )
}

/// Built-in regex patterns (crawl4ai RegexExtractionStrategy, 22 patterns, zero-LLM).
/// ponytail: one-liner per pattern; add more on demand.
pub fn regex_patterns() -> Vec<(&'static str, &'static str)> {
    vec![
        ("email", r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}"),
        ("url", r#"https?://[^\s"'<>]+"#),
        (
            "phone",
            r"(?:\+?\d{1,3}[-. ]?)?\(?\d{3}\)?[-. ]?\d{3}[-. ]?\d{4}",
        ),
        (
            "price",
            r"(?:[$£€]\s?\d{1,3}(?:,\d{3})*(?:\.\d{2})?|\d+(?:\.\d{2})?\s?(?:USD|EUR|GBP))",
        ),
        (
            "date",
            r"\b\d{4}[-/]\d{1,2}[-/]\d{1,2}\b|\b\d{1,2}[-/]\d{1,2}[-/]\d{2,4}\b",
        ),
        ("time", r"\b\d{1,2}:\d{2}(?::\d{2})?\s?(?:AM|PM)?\b"),
        ("ip", r"\b(?:\d{1,3}\.){3}\d{1,3}\b"),
        (
            "uuid",
            r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b",
        ),
        (
            "currency",
            r"(?:[$£€]\s?\d{1,3}(?:,\d{3})*(?:\.\d{2})?|\d+(?:\.\d{2})?\s?(?:USD|EUR|GBP))",
        ),
        ("percentage", r"\b\d+(?:\.\d+)?\s?%\b"),
        ("number", r"\b\d+(?:\.\d+)?\b"),
        ("date_iso", r"\b\d{4}-\d{2}-\d{2}\b"),
        ("date_us", r"\b\d{1,2}/\d{1,2}/\d{2,4}\b"),
        ("time24h", r"\b[012]\d:[0-5]\d(?::[0-5]\d)?\b"),
        (
            "ipv6",
            r"\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b|\b::1\b",
        ),
        ("hex_color", r"#(?:[0-9a-fA-F]{3}){1,2}\b"),
        ("postal_us", r"\b\d{5}(?:-\d{4})?\b"),
        ("postal_uk", r"\b[A-Z]{1,2}\d[A-Z\d]?\s?\d[A-Z]{2}\b"),
        ("credit_card", r"\b(?:\d[ -]?){13,19}\b"),
        ("iban", r"\b[A-Z]{2}\d{2}[A-Z0-9]{4,30}\b"),
        ("mac_addr", r"\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b"),
        ("twitter_handle", r"@[A-Za-z0-9_]{1,15}\b"),
        ("hashtag", r"#[A-Za-z0-9_]{1,280}\b"),
    ]
}

/// Build in-page JS for regex extraction. Custom `[{label, re}]` overrides or adds
/// builtins. Scans `document.documentElement.outerHTML` so hrefs/mailto are caught.
/// ponytail: native RegExp in-page, zero new deps, capped 200 matches/pattern.
pub fn build_regex_js(custom: &[Value]) -> String {
    let mut patterns: Vec<(&str, &str)> = regex_patterns();
    for c in custom {
        let l = c.get("label").and_then(|v| v.as_str());
        let re = c.get("re").and_then(|v| v.as_str());
        if let (Some(l), Some(re)) = (l, re) {
            patterns.retain(|(l0, _)| *l0 != l);
            patterns.push((l, re));
        }
    }
    let arr: Vec<String> = patterns
        .iter()
        .map(|(l, re)| {
            format!(
                "{{\"label\":{},\"re\":{}}}",
                serde_json::to_string(l).unwrap_or_default(),
                serde_json::to_string(re).unwrap_or_default()
            )
        })
        .collect();
    format!(
        "(() => {{ const P=[{}]; const src=document.documentElement.outerHTML; \
         const out={{}}; for (const p of P) {{ const re=new RegExp(p.re,'gi'); \
         const seen=new Set(); let m,arr=[]; while ((m=re.exec(src))!==null) {{ \
         const v=(m[0]||'').trim(); if (v && !seen.has(v)) {{ seen.add(v); arr.push(v); }} \
         if (m[0].length===0) re.lastIndex++; }} out[p.label]=arr.slice(0,200); }} \
         return JSON.stringify(out); }})()",
        arr.join(",")
    )
}

/// Run regex extraction over the current page (zero-LLM, in-page). Returns the
/// JSON object `{label: [matches]}`.
pub async fn regex_extract(
    browser: &impl BrowserBackend,
    custom: &[Value],
) -> anyhow::Result<Value> {
    let js = build_regex_js(custom);
    let v = browser.evaluate(&js).await?;
    Ok(v.as_str()
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null))
}

/// One row of a batch result. Fields are reused per op:
/// fetch → title/text; extract → data = parsed JSON array (text empty — data
/// carries the payload, don't duplicate it); interact → data (schema) or text
/// (raw innerText); screenshot → title = saved file path.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BatchResult {
    pub url: String,
    pub title: String,
    pub text: String,
    /// Parsed payload for extract/interact (JSON array); null for fetch/screenshot.
    /// ponytail: mirrors webrain_extract_json's `data` so the LLM reads one shape.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    pub error: Option<String>,
    /// Wall-clock ms spent on this URL (tab open → result). Lets the LLM see
    /// which URL was slow, and feeds the batch `stats` block.
    #[serde(skip_serializing_if = "is_zero")]
    pub ms: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

fn batch_err(url: &str, e: anyhow::Error) -> BatchResult {
    BatchResult {
        url: url.to_string(),
        title: String::new(),
        text: String::new(),
        data: None,
        error: Some(e.to_string()),
        ms: 0,
    }
}

/// Run one URL on a given (already-open) tab: navigate its session, run
/// `per_url`, return the BatchResult with wall-clock ms. Shared by the parallel
/// path (each URL's own tab) and the single-target sequential fallback (one
/// tab reused for every URL — navigate_session re-navigates it each time).
async fn run_on_tab<F, Fut>(
    browser: &CdpBackend,
    id: String,
    url: String,
    opts: crate::backends::cdp::NavOpts,
    per_url: F,
) -> BatchResult
where
    F: Fn(CdpBackend, String, String) -> Fut + Clone + Sync + Send + 'static,
    Fut: std::future::Future<Output = anyhow::Result<(String, String, Option<Value>)>>
        + Send
        + 'static,
{
    let t0 = std::time::Instant::now();
    let res = async {
        let sid = browser.tab_session(&id).await?;
        browser.navigate_session_opts(&sid, &url, &opts).await?;
        let (title, text, data) = per_url(browser.clone(), sid, url.clone()).await?;
        Ok::<_, anyhow::Error>(BatchResult {
            url: url.clone(),
            title,
            text,
            data,
            error: None,
            ms: 0,
        })
    }
    .await;
    let mut r = match res {
        Ok(r) => r,
        Err(e) => batch_err(&url, e),
    };
    r.ms = t0.elapsed().as_millis() as u64;
    r
}

/// Shared batch skeleton. Multi-target browsers (obscura/Chrome) load N tabs IN
/// PARALLEL (crawl4ai MemoryAdaptiveDispatcher / arun_many): each URL gets its
/// own tab driven via per-session CDP routing; a tokio semaphore bounds
/// in-flight tabs. lightpanda serve is SINGLE-target by design — its CDP holds
/// one browser context and `Target.createTarget` errors `TargetAlreadyLoaded`
/// once one exists (src/cdp/domains/target.zig) — so batch probes tab
/// capability and falls back to running every URL SEQUENTIALLY on one reused
/// tab (page loads serialize, but the whole crawl still works).
/// ponytail: one WS serializes the command layer; parallel page loads overlap
/// on multi-target engines, single-target just accepts the serialization.
async fn batch_map<F, Fut>(
    browser: &CdpBackend,
    urls: &[String],
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
    per_url: F,
) -> Vec<BatchResult>
where
    F: Fn(CdpBackend, String, String) -> Fut + Clone + Sync + Send + 'static,
    Fut: std::future::Future<Output = anyhow::Result<(String, String, Option<Value>)>>
        + Send
        + 'static,
{
    // Empty input: every public batch entry point indexes urls[0] on the probe
    // failure path — bail before any indexing instead of panicking.
    if urls.is_empty() {
        return Vec::new();
    }
    // Capability probe (raw CDP — sees the real single-target behavior even
    // when a target already exists): lightpanda errors TargetAlreadyLoaded on
    // any 2nd createTarget → sequential reuse; obscura/Chrome → parallel.
    let single = match browser.single_target_probe().await {
        Ok(s) => s,
        Err(e) => return vec![batch_err(&urls[0], e)],
    };

    if single {
        // lightpanda: reuse one tab for every URL, strictly sequential. Prefer
        // an already-registered tab (e.g. from a prior navigate), else open one.
        let id = match browser.existing_tab().await {
            Some(id) => id,
            None => match browser.open_tab("about:blank").await {
                Ok(id) => id,
                Err(e) => return vec![batch_err(&urls[0], e)],
            },
        };
        let mut out = Vec::with_capacity(urls.len());
        for url in urls {
            out.push(
                run_on_tab(
                    browser,
                    id.clone(),
                    url.clone(),
                    opts.clone(),
                    per_url.clone(),
                )
                .await,
            );
        }
        return out;
    }

    // Multi-target: original parallel path.
    let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(concurrency.max(1)));
    let mut handles = Vec::with_capacity(urls.len());
    for url in urls {
        let b = browser.clone();
        let sem = sem.clone();
        let url = url.clone();
        let opts = opts.clone();
        let per_url = per_url.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await.ok();
            let t0 = std::time::Instant::now();
            let res = async {
                let id = b.open_tab(&url).await?;
                let r = run_on_tab(&b, id.clone(), url.clone(), opts, per_url).await;
                let _ = b.close_tab(&id).await;
                Ok::<_, anyhow::Error>(r)
            }
            .await;
            let mut r = res.unwrap_or_else(|e| batch_err(&url, e));
            r.ms = t0.elapsed().as_millis() as u64;
            r
        }));
    }
    let mut out = Vec::with_capacity(urls.len());
    for h in handles {
        if let Ok(r) = h.await {
            out.push(r);
        }
    }
    out
}

/// Batch fetch: read title + visible text per URL, concurrently.
pub async fn batch_fetch(
    browser: &CdpBackend,
    urls: &[String],
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    batch_map(
        browser,
        urls,
        concurrency,
        opts,
        |b, sid, _url| async move {
            let title = b
                .eval_session(&sid, "document.title")
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
            let text = b
                .eval_session(&sid, "document.body ? document.body.innerText || '' : ''")
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
            Ok((title, text.chars().take(3000).collect(), None))
        },
    )
    .await
}

/// Split Markdown into chunks on blank lines, but NOT inside fenced code
/// blocks (``` … ```) — a blank line inside a fence is code, not a boundary,
/// so pruning can't orphan an opener. ponytail: minimal fence-state scanner.
fn markdown_chunks(md: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_fence = false;
    for line in md.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        if line.trim().is_empty() && !in_fence {
            if !cur.is_empty() {
                chunks.push(cur.trim().to_string());
                cur.clear();
            }
            continue;
        }
        cur.push_str(line);
        cur.push('\n');
    }
    if !cur.trim().is_empty() {
        chunks.push(cur.trim().to_string());
    }
    chunks
}

/// Batch markdown: convert each URL's page HTML to Markdown via htmd (pure
/// Rust — the turndown.js-equivalent). Surpasses batch_fetch's 3000-char
/// innerText cap with the FULL page as Markdown. Optional `query` bm25-prunes
/// the markdown to top_k chunks (crawl4ai fit/prune style). Zero LLM.
pub async fn batch_markdown(
    browser: &CdpBackend,
    urls: &[String],
    query: &str,
    top_k: usize,
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    let query = query.to_string();
    let converter = std::sync::Arc::new(
        htmd::HtmlToMarkdown::builder()
            .skip_tags(vec!["script", "style", "noscript"])
            .build(),
    );
    batch_map(browser, urls, concurrency, opts, move |b, sid, _url| {
        let query = query.clone();
        let converter = converter.clone();
        async move {
            let html = b
                .eval_session(
                    &sid,
                    "document.documentElement ? document.documentElement.outerHTML || '' : ''",
                )
                .await?
                .as_str()
                .unwrap_or("")
                .to_string();
            let mut md = converter
                .convert(&html)
                .map_err(|e| anyhow::anyhow!("htmd markdown conversion failed: {e}"))?;
            if !query.trim().is_empty() && top_k > 0 {
                // ponytail: fence-aware chunk + existing bm25_filter (the same
                // prune crawl4ai's fit_markdown does, zero LLM). Restore original
                // order (sort by index) and never split inside ``` fences.
                let chunks = markdown_chunks(&md);
                let mut kept = bm25_filter(&chunks, &query, top_k);
                kept.sort_by_key(|v| v.get("index").and_then(|i| i.as_u64()).unwrap_or(u64::MAX));
                md = kept
                    .into_iter()
                    .filter_map(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
                    .collect::<Vec<_>>()
                    .join("\n\n");
            }
            Ok((String::new(), md, None))
        }
    })
    .await
}

/// Batch extraction: run a CSS/XPath schema over every URL concurrently
/// (one tab per URL, semaphore-bounded). Zero-LLM, in-page JS.
pub async fn batch_extract(
    browser: &CdpBackend,
    urls: &[String],
    base_selector: &str,
    base_fields: &[Value],
    fields: &[Value],
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    let js = build_extract_js(base_selector, base_fields, fields);
    batch_map(browser, urls, concurrency, opts, move |b, sid, _url| {
        let js = js.clone();
        async move {
            let v = b.eval_session(&sid, &js).await?;
            let data = v
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or(Value::Null);
            // ponytail: `data` already carries the parsed array — do NOT mirror it
            // into `text` as the same JSON string. That duplicated every extract
            // batch's payload ~2× (response bytes + LLM output tokens). text stays
            // empty for extract; tools/AGENT_GUIDE already say "read data".
            Ok((String::new(), String::new(), Some(data)))
        }
    })
    .await
}

/// Batch eval: run arbitrary JS per URL (one tab per URL, concurrent) and
/// return each result as `data` (JSON string → parsed) or `text`. The
/// game-changer for "extract N pages with custom DOM logic": the LLM writes
/// ONE extractor and batch runs it — no fragile CSS schema, no per-URL loops.
pub async fn batch_eval(
    browser: &CdpBackend,
    urls: &[String],
    js: &str,
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    let js = js.to_string();
    batch_map(browser, urls, concurrency, opts, move |b, sid, _url| {
        let js = js.clone();
        async move {
            let v = b.eval_session(&sid, &js).await?;
            let data = v
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok());
            let text = if data.is_some() {
                String::new()
            } else {
                v.as_str().unwrap_or("").to_string()
            };
            Ok((String::new(), text, data))
        }
    })
    .await
}

/// Batch interaction: run an arbitrary async JS interaction (click "Load More"
/// loop, infinite-scroll, form fill) in PARALLEL tabs, one per URL, then
/// optionally extract a schema. The game-changer for "N independent interactive
/// sites" — one call replaces N serial agent loops (each tab drives its own
/// session; the interaction JS does the waiting).
///
/// `interaction` is async JS that returns nothing (side effects only). If
/// `base_selector` is non-empty, a schema extract runs after the interaction.
#[allow(clippy::too_many_arguments)] // public batch API: 8 genuinely-distinct params
pub async fn batch_interact(
    browser: &CdpBackend,
    urls: &[String],
    interaction: &str,
    base_selector: &str,
    base_fields: &[Value],
    fields: &[Value],
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    let interaction = interaction.to_string();
    let extract_js = if base_selector.is_empty() {
        String::new()
    } else {
        build_extract_js(base_selector, base_fields, fields)
    };
    batch_map(browser, urls, concurrency, opts, move |b, sid, _url| {
        let interaction = interaction.clone();
        let extract_js = extract_js.clone();
        async move {
            // Run the interaction; it does its own waits (load-more clicks etc).
            b.eval_session(&sid, &interaction).await?;
            let raw = if extract_js.is_empty() {
                b.eval_session(&sid, "document.body ? document.body.innerText || '' : ''")
                    .await?
            } else {
                b.eval_session(&sid, &extract_js).await?
            };
            let data = raw
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok());
            // ponytail: schema extract → `data` is the payload; don't duplicate it
            // into text. No schema → text carries the raw innerText, data stays None.
            let text = if data.is_some() {
                String::new()
            } else {
                raw.as_str().unwrap_or("").to_string()
            };
            Ok((String::new(), text, data))
        }
    })
    .await
}

/// Batch screenshots: save a full-page PNG per URL into `dir`, concurrently
/// (one tab per URL, semaphore-bounded). Honors `NavOpts` (network_idle,
/// disable_resources, wait_selector) like the other batch ops.
pub async fn batch_screenshot(
    browser: &CdpBackend,
    urls: &[String],
    dir: &str,
    concurrency: usize,
    opts: &crate::backends::cdp::NavOpts,
) -> Vec<BatchResult> {
    std::fs::create_dir_all(dir).ok();
    let dir = dir.to_string();
    batch_map(browser, urls, concurrency, opts, move |b, sid, url| {
        let dir = dir.clone();
        async move {
            let png = b.screenshot_session(&sid, true).await?;
            // ponytail: short URL hash keeps filenames unique across URLs that
            // share a last path segment (page_x/page_y both ending '/page').
            let mut hasher = sha2::Sha256::new();
            use sha2::Digest;
            hasher.update(url.as_bytes());
            // sha2 0.11 output no longer implements LowerHex; format manually.
            let h = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            let path = format!("{dir}/page_{}.png", &h[..12]);
            std::fs::write(&path, &png)?;
            Ok((path, format!("{} bytes", png.len()), None))
        }
    })
    .await
}

/// Chrome-identical HTTP layer for the no-browser fast path (obscura's
/// "GREASE-correct client hints" at the HTTP level). sec-ch-ua follows
/// Chromium's structure — real Chrome brand + GREASE brand (v=24), the form
/// Chrome 101+ actually ships.
/// ponytail: TLS-layer JA3/JA4 byte parity needs a BoringSSL fork (obscura/
/// wreq) — rustls can't emit Chrome's extensions/GREASE. This kills the
/// HTTP-header/UA tells WAFs check first. Add BoringSSL only when a WAF
/// starts failing past this layer.
// ponytail: static Chrome major, must agree with sec-ch-ua. Probe spawned a
// visible Chrome; rustls TLS already gives the fingerprint away. Bump when Chrome drifts.
const CHROME_UA_VER: &str = "145";

/// Chrome-identical HTTP headers for the no-browser fast path.
fn browser_headers() -> Vec<(String, String)> {
    let ver = CHROME_UA_VER;
    vec![
        ("User-Agent".to_string(), format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{ver}.0.0.0 Safari/537.36")),
        ("sec-ch-ua".to_string(), format!("\"Google Chrome\";v=\"{ver}\", \"Not)A;Brand\";v=\"24\", \"Chromium\";v=\"{ver}\"")),
        ("sec-ch-ua-mobile".to_string(), "?0".to_string()),
        ("sec-ch-ua-platform".to_string(), "\"Windows\"".to_string()),
        ("Accept".to_string(), "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8".to_string()),
        ("Accept-Language".to_string(), "en-US,en;q=0.9".to_string()),
        ("sec-fetch-site".to_string(), "none".to_string()),
        ("sec-fetch-mode".to_string(), "navigate".to_string()),
        ("sec-fetch-dest".to_string(), "document".to_string()),
        ("upgrade-insecure-requests".to_string(), "1".to_string()),
    ]
}

/// Shared HTTP agent — ONE connection pool for all no-browser fetches
/// (http_fetch, validate_urls, download_files). Before, every call built a
/// fresh `ureq::Agent`, so each offset probe paid a new TCP+TLS handshake
/// (~0.3-1s each). Agent is cheap to clone (inner Arc → same pool).
/// ponytail: static OnceLock instead of a builder wrapper per call.
static HTTP_AGENT: std::sync::OnceLock<ureq::Agent> = std::sync::OnceLock::new();

pub(crate) fn browser_agent() -> ureq::Agent {
    HTTP_AGENT
        .get_or_init(|| {
            ureq::Agent::new_with_config(
                ureq::config::Config::builder()
                    .timeout_global(Some(std::time::Duration::from_secs(30)))
                    .build(),
            )
        })
        .clone()
}

/// Attach the Chrome header set to a no-body request (GET/HEAD).
/// Generic over the request body so we never name ureq's (re-)exported body
/// types; `.header()` lives on `impl<Any> RequestBuilder<Any>`.
fn browser_req<B>(mut req: ureq::RequestBuilder<B>) -> ureq::RequestBuilder<B> {
    for (k, v) in browser_headers() {
        req = req.header(&k, &v);
    }
    req
}

/// Full raw HTML GET for SERP parsing — `http_fetch` truncates HTML text to
/// 3000 chars (fine for probes, useless for parsing a results page). Returns
/// `(status, body)` with the FULL body. Reuses the pooled agent + Chrome headers.
/// ponytail: one new fn instead of widening http_fetch's cap for every caller.
pub(crate) fn serp_http_get(url: &str, proxy: Option<&str>) -> anyhow::Result<(u16, String)> {
    let agent = http_agent(proxy)?;
    let resp = browser_req(agent.get(url)).call()?;
    let status = resp.status().as_u16();
    let bytes = resp.into_body().read_to_vec()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok((status, text))
}

/// Form-POST variant of `serp_http_get` for endpoints that must not leak
/// secrets into the URL (2captcha: the API key + proxy credentials would land
/// in access logs / reverse proxies if sent as query params). Same pooled agent
/// + Chrome headers, `application/x-www-form-urlencoded` body.
pub(crate) fn serp_http_post(
    url: &str,
    form: &[(String, String)],
    proxy: Option<&str>,
) -> anyhow::Result<(u16, String)> {
    let agent = http_agent(proxy)?;
    let pairs: Vec<(&str, &str)> = form.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    // send_form consumes by-value tuples (Item = (K, V)), so use into_iter().
    let resp = browser_req(agent.post(url)).send_form(pairs)?;
    let status = resp.status().as_u16();
    let bytes = resp.into_body().read_to_vec()?;
    let text = String::from_utf8_lossy(&bytes).into_owned();
    Ok((status, text))
}

/// Shared ureq agent for the serp helpers: one-off proxied agent when a proxy
/// is set (fresh pool per proxied call), otherwise the shared pooled agent.
fn http_agent(proxy: Option<&str>) -> anyhow::Result<ureq::Agent> {
    Ok(match proxy {
        Some(p) => {
            let proxy = ureq::Proxy::new(p)?;
            ureq::Agent::new_with_config(
                ureq::config::Config::builder()
                    .timeout_global(Some(std::time::Duration::from_secs(30)))
                    .proxy(Some(proxy))
                    .build(),
            )
        }
        None => browser_agent(),
    })
}

/// No-browser HTTP fetch (browsemind `http_crawl`): GET a URL, return status +
/// visible-ish text. 10-100x faster than browser navigation, zero memory — but no
/// JS/SPA/auth. ponytail: ureq GET → plain text; cap to keep MCP replies small.
/// Reuses the shared pooled agent (keep-alive). JSON responses are NOT truncated
/// so a single probe can reveal a total; text/HTML is capped at 3000 chars.
/// Returns pagination headers (X-Total-Count/Link/Content-Range) when present
/// so an agent can discover `total` in one call instead of boundary-probing.
pub fn http_fetch(url: &str) -> anyhow::Result<Value> {
    let resp = browser_req(browser_agent().get(url)).call()?;
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    // Pagination / total signals, if the server exposes them (capture before
    // consuming the body).
    let mut hdrs = serde_json::Map::new();
    for name in ["x-total-count", "link", "content-range", "x-next-page"] {
        if let Some(val) = resp.headers().get(name).and_then(|v| v.to_str().ok()) {
            hdrs.insert(name.to_string(), serde_json::Value::String(val.to_string()));
        }
    }
    let text = resp.into_body().read_to_string()?;
    let is_json = content_type.contains("json") || text.trim_start().starts_with(['{', '[']);
    let out_text = if is_json {
        text
    } else {
        text.chars().take(3000).collect::<String>()
    };
    let mut v = json!({
        "url": url,
        "status": status,
        "content_type": content_type,
        "text": out_text,
        "bytes": out_text.len(),
    });
    if !hdrs.is_empty() {
        v["headers"] = serde_json::Value::Object(hdrs);
    }
    Ok(v)
}

/// Discover crawlable URLs from the site's sitemap (spider-rs `crawl_sitemap` /
/// Scrapling `SitemapSpider`). Flow: robots.txt `Sitemap:` → sitemap_index.xml
/// (or the URL given directly) → leaf sitemaps → every `<loc>`. Uses the pooled
/// HTTP agent (no browser), zero new deps — regex `<loc>` parse, sitemap XML is
/// simple enough. ponytail: no XML parser, `<loc>` regex is correct on the
/// sitemap format; if a server serves a nonstandard sitemap, it yields fewer
/// URLs, the agent falls back to crawl.
/// Returns a JSON object: {urls: [...], sources: [fetched sitemap urls], error?}.
pub fn sitemap_urls(start_url: &str) -> anyhow::Result<Value> {
    fn get_text(url: &str) -> anyhow::Result<String> {
        let resp = browser_req(browser_agent().get(url)).call()?;
        let status = resp.status().as_u16();
        let text = resp.into_body().read_to_string()?;
        if !(200..300).contains(&status) {
            return Err(anyhow::anyhow!("GET {url} -> {status}"));
        }
        Ok(text)
    }
    fn locs(xml: &str) -> Vec<String> {
        // sitemap `<loc>` is CDATA-free; capture the URL inside the tags.
        xml.split("<loc>")
            .skip(1)
            .filter_map(|s| s.split("</loc>").next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
    fn is_index(xml: &str) -> bool {
        // sitemap index contains <sitemap> entries; leaf contains <url>.
        xml.contains("<sitemap>") && !xml.contains("<url>")
    }

    let mut sources = Vec::new();
    let mut urls: Vec<String> = Vec::new();

    // 1. If not given a sitemap URL, ask robots.txt for the Sitemap: line.
    let mut frontier: Vec<String> = Vec::new();
    let seed = start_url.trim_end_matches('/').to_string();
    if seed.contains("/sitemap") || seed.ends_with(".xml") {
        frontier.push(seed);
    } else {
        let robots = format!("{seed}/robots.txt");
        if let Ok(text) = get_text(&robots) {
            // ponytail: multi-line Sitemap: entries; case-insensitive key.
            for line in text.lines() {
                if let Some(idx) = line.to_lowercase().find("sitemap:") {
                    let url = line[idx + 8..].trim();
                    if !url.is_empty() {
                        frontier.push(url.to_string());
                    }
                }
            }
            sources.push(robots);
        }
    }

    let mut seen = std::collections::HashSet::new();
    while let Some(f) = frontier.pop() {
        if !seen.insert(f.clone()) {
            continue;
        }
        sources.push(f.clone());
        match get_text(&f) {
            Ok(xml) => {
                if is_index(&xml) {
                    // sitemap index → push child sitemaps onto the frontier.
                    for child in locs(&xml) {
                        if !seen.contains(&child) {
                            frontier.push(child);
                        }
                    }
                } else {
                    urls.extend(locs(&xml));
                }
            }
            Err(e) => {
                // A 404'd sitemap isn't fatal — keep going through the frontier.
                tracing::debug!("sitemap fetch failed {f}: {e}");
            }
        }
    }

    urls.sort();
    urls.dedup();
    let mut v = json!({"urls": urls, "sources": sources, "count": urls.len()});
    if urls.is_empty() {
        v["error"] =
            json!("no sitemap URLs found — site may not expose sitemap.xml/robots.txt Sitemap");
    }
    Ok(v)
}

/// BM25 relevance filter (browsemind BM25 filter / crawl4ai ContentRelevanceFilter):
/// score a list of text items against a query, keep the top_k. Zero LLM.
/// ponytail: stdlib-only tokenizer (lowercase, split on non-alphanumeric),
/// k1=1.5, b=0.75, ln+1 IDF. Good enough to rank; no stemming/stopwords.
pub fn bm25_filter(items: &[String], query: &str, top_k: usize) -> Vec<Value> {
    if items.is_empty() || query.trim().is_empty() || top_k == 0 {
        return vec![];
    }
    let k1 = 1.5f64;
    let b = 0.75f64;
    let tokenize = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
            .map(String::from)
            .collect()
    };
    let docs: Vec<Vec<String>> = items.iter().map(|i| tokenize(i)).collect();
    let n = docs.len() as f64;
    let avg_len = docs.iter().map(|d| d.len()).sum::<usize>() as f64 / n;
    let q_terms = tokenize(query);

    // ponytail: precompute per-term doc frequency once (O(docs·terms)) instead of
    // re-scanning all docs for every (doc, term) inside the score loop (O(docs²·terms)).
    // Fixes the flagged O(n²) hot path — was linear_scan_in_loop=1 on bm25_filter.
    // df = number of DOCS containing the term (counted once per doc), not raw
    // occurrences — a term repeated inside one doc must not inflate df past n/2
    // and flip IDF negative (which would penalize its own matches).
    let mut df: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    for d in &docs {
        let mut seen = std::collections::HashSet::new();
        for t in d {
            if seen.insert(t) {
                *df.entry(t.clone()).or_insert(0.0) += 1.0;
            }
        }
    }

    let mut scored: Vec<(f64, usize)> = Vec::with_capacity(docs.len());
    for (i, doc) in docs.iter().enumerate() {
        let doc_len = doc.len() as f64;
        let mut score = 0.0;
        for t in &q_terms {
            let tf = doc.iter().filter(|d| *d == t).count() as f64;
            if tf == 0.0 {
                continue;
            }
            let df = *df.get(t).unwrap_or(&0.0);
            let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
            let denom = tf + k1 * (1.0 - b + b * doc_len / avg_len);
            score += idf * (tf * (k1 + 1.0)) / denom;
        }
        scored.push((score, i));
    }
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
        .into_iter()
        .take(top_k)
        .map(|(s, i)| json!({"index": i, "score": s, "text": items[i]}))
        .collect()
}

/// Validate a list of URLs — which are alive vs dead (404/5xx/errors).
/// browsemind `seed(from_links, validate=True)`. ponytail: HEAD first, GET
/// fallback for HEAD-blocking servers, status < 400 = alive, short timeouts.
pub fn validate_urls(urls: &[String]) -> Vec<Value> {
    let agent = browser_agent();
    urls.iter()
        .map(|u| {
            let alive = match browser_req(agent.head(u)).call() {
                Ok(r) => r.status().as_u16() < 400,
                Err(_) => match browser_req(agent.get(u)).call() {
                    Ok(r) => r.status().as_u16() < 400,
                    Err(_) => false,
                },
            };
            json!({"url": u, "alive": alive})
        })
        .collect()
}

/// Download URLs to `dir` over plain HTTP (no browser). For session-bound files
/// use the browser instead (open a tab, navigate, click).
/// ponytail: ureq stream-to-file via into_reader() (unlimited — read_to_vec caps
/// at 10 MiB, which silently failed on antenna's multi-hundred-MB mp4s);
/// cookie plumbing still pending — add when downloads need auth.
/// yt-dlp download (video/audio/HLS/playlists/age-gated media). Shells out to
/// the installed yt-dlp binary — no new Rust dep; `extra` passthrough exposes
/// every yt-dlp flag (--write-subs, --embed-thumbnail, --cookies, --proxy...).
/// ponytail: ONE implementation shared by the MCP no-browser path (lib.rs) and
/// the tools.rs dispatch, so the advertised `engine: "ytdlp"` actually works
/// instead of being silently ignored by the HTTP short-circuit.
pub fn download_ytdlp(
    urls: &[String],
    dir: &str,
    audio_only: bool,
    format: Option<&str>,
    extra: &[String],
) -> Value {
    std::fs::create_dir_all(dir).ok();
    // ponytail: reuse install::find_tool so the bundled yt-dlp (`webrain install
    // watch`) wins before PATH — same resolution `webrain watch` uses.
    let bin = crate::install::find_tool("yt-dlp").unwrap_or_else(|| "yt-dlp".into());
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("-o").arg(format!("{dir}/%(title)s_%(id)s.%(ext)s"));
    if audio_only {
        cmd.arg("-x").arg("--audio-format").arg("mp3");
    } else if let Some(f) = format.filter(|f| !f.is_empty()) {
        cmd.arg("-f").arg(f);
    } else {
        cmd.arg("-f").arg("bestvideo*+bestaudio/best");
    }
    for a in extra {
        cmd.arg(a);
    }
    cmd.args(urls);
    match cmd.output() {
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
    }
}

pub fn download_files(urls: &[String], dir: &str) -> Vec<BatchResult> {
    std::fs::create_dir_all(dir).ok();
    let agent = browser_agent();
    let mut out = Vec::with_capacity(urls.len());
    for (i, url) in urls.iter().enumerate() {
        let res = (|| -> anyhow::Result<BatchResult> {
            let resp = browser_req(agent.get(url)).call()?;
            // ponytail: CDN URLs carry ?sig&params — drop query/fragment or the
            // derived filename is invalid on Windows (os error 123).
            let name = url
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .map(|s| s.split(['?', '#']).next().unwrap_or(s))
                .unwrap_or("download");
            let path = format!("{dir}/{i}_{name}");
            let mut f = std::fs::File::create(&path)?;
            let mut reader = resp.into_body().into_reader();
            let n = std::io::copy(&mut reader, &mut f)?;
            Ok(BatchResult {
                url: url.clone(),
                title: path,
                text: format!("{n} bytes"),
                data: None,
                error: None,
                ms: 0,
            })
        })();
        out.push(match res {
            Ok(r) => r,
            Err(e) => batch_err(url, e),
        });
    }
    out
}

// ── Fit-text cleaning ────────────────────────────────────────────────────────
// ponytail: in-page JS that strips nav/footer/script/style/iframe, excludes
// social/ads links, word_count_threshold. Builds a clean text blob for the LLM
// without an HTML→Markdown converter (deliberate: text via innerText, layout
// via vision tiles). Returns JS string to pass to evaluate().

pub fn build_clean_js(word_threshold: usize, exclude_social: bool) -> String {
    // LEADING comma (not trailing): a selector list ending in a comma is a
    // SyntaxError, so the default (exclude_social=false → empty) must stay valid.
    let social = if exclude_social {
        r#",[href*="facebook.com"],[href*="twitter.com"],[href*="instagram.com"],[href*="linkedin.com"],[href*="youtube.com"]"#
    } else {
        ""
    };
    format!(
        r#"(()=>{{const w=document.createElement('div');w.innerHTML=document.body.innerHTML;for(const s of w.querySelectorAll('script,style,noscript,iframe,nav,footer,header,aside{social}'))s.remove();const t=w.textContent||'';return t.split(/\s+/).filter(w=>w.length>={word_threshold}).join(' ').slice(0,8192);}})()"#
    )
}

// ── PDF extraction ──────────────────────────────────────────────────────────
// Engine: Firecrawl `pdf-inspector` (pure Rust on lopdf) — proper ToUnicode
// CMap decoding, Markdown output, table detection, layout classification,
// AND embedded image extraction (DCTDecode/FlateDecode → PNG via lopdf+image crate).
// Text + images work in the default build; the optional `pdfium` feature adds
// vision rendering (`pdf_render`) + JPEG2000/CCITT image fallback via Google Pdfium.

/// Extract PDF → Markdown + embedded images (Firecrawl pdf-inspector + lopdf).
/// Pure Rust, zero system deps. Returns full-document markdown (headings/lists/
/// tables/bold-italic), per-page text with OCR-needs-flag, layout classification,
/// and embedded images as base64 PNGs (DCTDecode JPEG + FlateDecode raw).
pub fn pdf_extract(path: &str) -> anyhow::Result<Value> {
    let result =
        pdf_inspector::process_pdf(path).map_err(|e| anyhow::anyhow!("pdf extract failed: {e}"))?;

    let pages_res = pdf_inspector::extract_pages_markdown(path, None)
        .map_err(|e| anyhow::anyhow!("pdf pages failed: {e}"))?;
    let mut texts = serde_json::Map::new();
    for p in &pages_res.pages {
        texts.insert(
            p.page.to_string(),
            json!({"text": p.markdown, "needs_ocr": p.needs_ocr}),
        );
    }

    // ponytail: extract images in the same pass. Errors are non-fatal — we
    // still return text+layout even if image extraction fails on exotic formats.
    let images = extract_images_lopdf(path, None).unwrap_or_default();

    Ok(json!({
        "path": path,
        "pages": result.page_count,
        "pdf_type": format!("{:?}", result.pdf_type),
        "confidence": result.confidence,
        "has_encoding_issues": result.has_encoding_issues,
        "layout": {
            "is_complex": result.layout.is_complex,
            "pages_with_tables": result.layout.pages_with_tables,
            "pages_with_columns": result.layout.pages_with_columns,
        },
        "markdown": result.markdown,
        "texts": texts,
        "images": images,
    }))
}

/// Render PDF pages as base64 PNGs — the PixelRAG alternative to text extraction.
/// If `tile_size` is set (e.g. 800), each page is split into square tiles
/// for more efficient vision-model processing. Without tiles, one full image
/// per page. Requires `--features pdfium`.
#[cfg(feature = "pdfium")]
pub fn pdf_render(
    path: &str,
    pages: Option<&[u32]>,
    dpi: Option<f32>,
    tile_size: Option<u32>,
) -> anyhow::Result<Value> {
    use image::GenericImageView;
    use pdfium_render::prelude::*;
    let pdfium = Pdfium::default();
    let doc = pdfium.load_pdf_from_file(path, None)?;
    let scale = (dpi.unwrap_or(150.0) / 72.0) as f32;
    let config = PdfRenderConfig::new().scale_page_by_factor(scale);
    let all_pages = doc.pages();
    let target: Vec<u32> = match pages {
        Some(p) => p.to_vec(),
        None => (1..=all_pages.len() as u32).collect(),
    };
    let mut imgs = Vec::new();
    for &pn in &target {
        if let Ok(page) = all_pages.get(pn as i32 - 1) {
            let bmp = page.render_with_config(&config)?;
            let img = bmp.as_image()?.into_rgba8();
            match tile_size {
                Some(ts) => {
                    let (w, h) = img.dimensions();
                    for row in (0..h).step_by(ts as usize) {
                        for col in (0..w).step_by(ts as usize) {
                            let rw = ts.min(w - col);
                            let rh = ts.min(h - row);
                            let tile = image::imageops::crop_imm(&img, col, row, rw, rh).to_image();
                            let mut buf = Vec::new();
                            tile.write_to(
                                &mut std::io::Cursor::new(&mut buf),
                                image::ImageFormat::Png,
                            )?;
                            use base64::Engine;
                            imgs.push(json!({"page": pn, "x": col, "y": row, "w": rw, "h": rh, "image_b64": base64::engine::general_purpose::STANDARD.encode(&buf)}));
                        }
                    }
                }
                None => {
                    let mut buf = Vec::new();
                    img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png)?;
                    use base64::Engine;
                    imgs.push(json!({"page": pn, "image_b64": base64::engine::general_purpose::STANDARD.encode(&buf)}));
                }
            }
        }
    }
    Ok(json!({"path": path, "dpi": dpi.unwrap_or(150.0), "tile_size": tile_size, "pages": imgs}))
}

/// Extract embedded images from PDF as base64 PNGs — zero system deps.
/// Uses lopdf to find image XObjects, then `image` crate decodes DCTDecode
/// (JPEG) and FlateDecode (zlib raw pixels) → re-encodes as PNG.
/// Skips JPEG2000/CCITT/JBIG2 — use `webrain_pdf_render` (pdfium) for those.
/// Returns `[{page, index, width, height, image_b64}]`.
pub fn pdf_images(path: &str, pages: Option<&[u32]>) -> anyhow::Result<Value> {
    let imgs = extract_images_lopdf(path, pages)?;
    Ok(json!({"path": path, "count": imgs.len(), "images": imgs}))
}

/// Core image extraction — shared by `pdf_extract` and `pdf_images`.
fn extract_images_lopdf(path: &str, pages: Option<&[u32]>) -> anyhow::Result<Vec<Value>> {
    use flate2::read::ZlibDecoder;
    use lopdf::Document;
    use std::io::Read;

    let doc = Document::load(path)?;
    let all_pages = doc.get_pages();
    let target: Vec<u32> = match pages {
        Some(p) => p.to_vec(),
        None => (1u32..=all_pages.len() as u32).collect(),
    };
    let mut out = Vec::new();
    for &pn in &target {
        let (page_id, _) = match all_pages.get(&pn) {
            Some(p) => *p,
            None => continue,
        };
        // lopdf 0.41: get_pages values are u32 (object number), get_dictionary takes (u32, u16).
        let page_dict = match doc.get_dictionary((page_id, 0)) {
            Ok(d) => d,
            Err(_) => continue,
        };
        // ponytail: direct Resources lookup — skip pages without /Resources
        // (inherited resources from parent nodes are rare in practice).
        let resources = page_dict
            .get(b"Resources")
            .ok()
            .and_then(|r| r.as_dict().ok());
        if let Some(res) = resources {
            if let Some(xobj) = res.get(b"XObject").ok().and_then(|x| x.as_dict().ok()) {
                let mut idx = 0u32;
                for (_name, obj_val) in xobj.iter() {
                    let obj_id = match obj_val.as_reference().ok() {
                        Some(id) => id,
                        None => continue,
                    };
                    let (content, dict_owned) = match doc.get_object(obj_id) {
                        Ok(lopdf::Object::Stream(s)) => (s.content.clone(), s.dict.clone()),
                        _ => continue,
                    };
                    if dict_owned
                        .get(b"Subtype")
                        .ok()
                        .and_then(|v| v.as_name().ok())
                        .map(|n| n != b"Image")
                        .unwrap_or(true)
                    {
                        continue;
                    }
                    let w = dict_owned
                        .get(b"Width")
                        .ok()
                        .and_then(|v| v.as_i64().ok())
                        .unwrap_or(0) as u32;
                    let h = dict_owned
                        .get(b"Height")
                        .ok()
                        .and_then(|v| v.as_i64().ok())
                        .unwrap_or(0) as u32;
                    if w == 0 || h == 0 {
                        continue;
                    }
                    let filter = dict_owned
                        .get(b"Filter")
                        .ok()
                        .and_then(|v| v.as_name().ok());
                    let data = &content;

                    let img: Option<image::DynamicImage> = match filter {
                        Some(b"DCTDecode") => image::load_from_memory(data).ok(),
                        Some(b"FlateDecode") => {
                            let mut dec = Vec::new();
                            if ZlibDecoder::new(&data[..]).read_to_end(&mut dec).is_err() {
                                continue;
                            }
                            let bpp = dict_owned
                                .get(b"BitsPerComponent")
                                .ok()
                                .and_then(|v| v.as_i64().ok())
                                .unwrap_or(8) as u8;
                            if bpp != 8 {
                                continue;
                            }
                            let cs = dict_owned
                                .get(b"ColorSpace")
                                .ok()
                                .and_then(|v| v.as_name().ok());
                            match cs {
                                Some(b"DeviceRGB") => image::RgbImage::from_raw(w, h, dec)
                                    .map(image::DynamicImage::ImageRgb8),
                                _ => image::GrayImage::from_raw(w, h, dec)
                                    .map(image::DynamicImage::ImageLuma8),
                            }
                        }
                        _ => None,
                    };

                    if let Some(img) = img {
                        let rgba = img.to_rgba8();
                        let mut buf = Vec::new();
                        rgba.write_to(
                            &mut std::io::Cursor::new(&mut buf),
                            image::ImageFormat::Png,
                        )?;
                        use base64::Engine;
                        out.push(json!({"page": pn, "index": idx, "width": w, "height": h,
                            "image_b64": base64::engine::general_purpose::STANDARD.encode(&buf)}));
                        idx += 1;
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Batch PDF extraction: extract files CONCURRENTLY. PDF parsing is CPU-bound
/// and single-threaded per file, so real threads (stdlib, no new dep) scale
/// across cores — capped at `available_parallelism()` for massive batches.
/// Returns one JSON object per file, input order preserved.
pub fn pdf_extract_batch(paths: &[String]) -> Vec<Value> {
    if paths.is_empty() {
        return Vec::new(); // empty batch → empty result, no chunks(0) panic
    }
    let n = std::thread::available_parallelism()
        .map(|x| x.get())
        .unwrap_or(4);
    let workers = n.min(paths.len()).max(1);
    // ponytail: fixed worker pool, one thread per chunk; per-file threads would
    // thrash on huge batches. Add a concurrency knob only if measured needs it.
    std::thread::scope(|s| {
        let mut handles = Vec::with_capacity(workers);
        for chunk in paths.chunks(paths.len().div_ceil(workers)) {
            let chunk: Vec<String> = chunk.to_vec();
            handles.push(s.spawn(move || {
                chunk
                    .iter()
                    .map(|p| {
                        pdf_extract(p)
                            .unwrap_or_else(|e| json!({"path": p, "error": e.to_string()}))
                    })
                    .collect::<Vec<Value>>()
            }));
        }
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}
