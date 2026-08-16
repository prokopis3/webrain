/// A single page operation result: navigation, interaction, or extraction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PageResult {
    pub url: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub error: Option<String>,
    pub duration_ms: u64,
}

/// What the browser returned after navigating to a URL.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PageState {
    pub url: String,
    pub title: String,
    /// Visible text (not full HTML — keeps MCP tokens manageable).
    pub text: String,
    /// Interactive elements indexed by position for click/type tools.
    pub elements: Vec<InteractiveElement>,
    /// Same-origin internal links (deduped) — one-call crawl discovery.
    pub links: Vec<String>,
    /// Anti-bot challenge/block kind (`cloudflare_challenge`, `blocked`, `captcha`) when detected.
    pub challenge: Option<String>,
    /// Loaded but stripped to almost no interactivity (bot-limited shell).
    /// Soft hint, not a block. `#[serde(default)]` so old serialized states load.
    #[serde(default)]
    pub crippled: bool,
    /// Chrome error page (dead domain / cert / server error). spider-rs
    /// `is_chrome_error_page` borrow — the `ERR_*` code, or "CHROME_ERROR" for a
    /// generic interstitial. Lets the LLM stop scraping garbage error pages.
    #[serde(default)]
    pub chrome_error: Option<String>,
}

/// Classify a page as an anti-bot challenge/block page from its title+visible text.
/// crawl4ai `antibot_detector`, trimmed to the markers we actually hit. No HTML/network.
/// ponytail: tiny marker list; add structural HTML markers if sites evade it.
pub fn detect_antibot(title: &str, text: &str) -> Option<String> {
    let hay = format!("{title}\n{text}").to_ascii_lowercase();
    // Strong technical markers — near-certain challenge signals (a legit page
    // doesn't contain "__cf_chl" / "challenge-platform" / a captcha id in its
    // visible text). Single match is enough.
    const STRONG: &[(&str, &str)] = &[
        ("cloudflare_challenge", "__cf_chl"),
        ("cloudflare_challenge", "challenge-platform"),
        ("blocked", "cf-error-code"),
        ("captcha", "h-captcha"),
        ("captcha", "g-recaptcha"),
    ];
    for (kind, m) in STRONG {
        if hay.contains(m) {
            return Some(kind.to_string());
        }
    }
    // Generic phrases that can appear in legitimate prose ("forbidden",
    // "access denied", "just a moment") — require TWO corroborating markers
    // before declaring a challenge, so a support article that merely mentions
    // one isn't misread as a blocked page (challenge gates whether the LLM
    // keeps scraping).
    const WEAK: &[(&str, &str)] = &[
        ("cloudflare_challenge", "checking your browser"),
        ("cloudflare_challenge", "just a moment"),
        ("cloudflare_challenge", "verify you are human"),
        ("blocked", "access denied"),
        ("blocked", "forbidden"),
    ];
    let hits: Vec<&str> = WEAK
        .iter()
        .filter(|(_, m)| hay.contains(m))
        .map(|(kind, _)| *kind)
        .collect();
    if hits.len() >= 2 {
        if hits.contains(&"cloudflare_challenge") {
            return Some("cloudflare_challenge".to_string());
        }
        return Some(hits[0].to_string());
    }
    None
}

/// A page Chrome rendered for a dead/cert/5xx URL (spider-rs
/// `is_chrome_error_page`/`extract_chrome_error_code` borrow): the error page
/// LOOKS like content, so the LLM scrapes garbage silently. Returns the `ERR_*`
/// code when present, else "CHROME_ERROR" for a generic interstitial. Title+
/// text only (no HTML/network), mirrors `detect_antibot`.
pub fn detect_chrome_error(title: &str, text: &str) -> Option<String> {
    let hay = format!("{title}\n{text}");
    // Chrome error pages carry a code — ERR_NAME_NOT_RESOLVED, DNS_PROBE_STARTED,
    // NET::ERR_CERT_* … grab the first one.
    let extract = |region: &str| -> Option<String> {
        for marker in ["ERR_", "DNS_"] {
            if let Some(i) = region.find(marker) {
                let code: String = region[i..]
                    .chars()
                    .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == '_')
                    .collect();
                if !code.is_empty() {
                    return Some(code);
                }
            }
        }
        None
    };
    // A code in the <title> is authoritative — real interstitials put it there.
    if let Some(code) = extract(title) {
        return Some(code);
    }
    // Interstitials without a bare code (apostrophe may be curly — match both).
    let l = hay.to_ascii_lowercase();
    const PHRASES: &[&str] = &[
        "this site can't be reached",
        "this site can’t be reached",
        "this page isn’t working",
        "this page isn't working",
        "your connection is not private",
        "no internet",
    ];
    // A body-only code needs a corroborating interstitial phrase — an ordinary
    // support article that merely mentions "ERR_*" must not be flagged as dead.
    if PHRASES.iter().any(|p| l.contains(*p)) {
        if let Some(code) = extract(&hay) {
            return Some(code);
        }
        return Some("CHROME_ERROR".to_string());
    }
    None
}

/// A loaded page with almost no interactive elements and no challenge is likely
/// a bot-limited "crippled" shell (YouTube/Twitter/X serve stripped pages to
/// automation). Soft hint, not a block.
/// ponytail: naive count heuristic; a <5-element real page (login) trips it too —
/// raise the threshold or scope per-site if it's noisy.
pub fn detect_crippled(challenge: &Option<String>, element_count: usize) -> bool {
    challenge.is_none() && element_count < 5
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InteractiveElement {
    pub index: usize,
    pub tag: String,
    pub text: String,
    pub selector: String,
    pub visible: bool,
}

/// JS that indexes interactive elements for click/type tools (shared by
/// navigate and snapshot so both produce identical element lists).
/// Interaction is index-based only (querySelectorAll order); precise
/// selectors for extraction come from webrain_a11y (css_path/xpath).
/// Housed here (not in a backend module) so the backend-agnostic
/// `BrowserBackend::snapshot` default doesn't depend on a concrete backend.
pub const ELEMENTS_JS: &str = r#"
        (() => {
            const esc = (s) => (window.CSS && CSS.escape) ? CSS.escape(s) : s.replace(/([^a-zA-Z0-9_-])/g, '\\$1');
            // Index-stable unique selector: id → class → nth-of-type chain from
            // body. A bare tag name ("input") is NOT used — DOM.querySelector
            // would resolve it to the FIRST match, not the element at `index`
            // (broke DOM.setFileInputFiles for id/class-less <input type=file>).
            const uniqSel = (el) => {
                if (el.id) return '#' + esc(el.id);
                if (el.className && typeof el.className === 'string' && el.className.trim()) return '.' + esc(el.className.trim().split(/\s+/)[0]);
                const parts = [];
                let cur = el;
                while (cur && cur.nodeType === 1 && cur !== document.documentElement) {
                    let nth = 1, sib = cur.previousElementSibling;
                    while (sib) { if (sib.tagName === cur.tagName) nth++; sib = sib.previousElementSibling; }
                    parts.unshift(cur.tagName.toLowerCase() + ':nth-of-type(' + nth + ')');
                    cur = cur.parentNode;
                }
                return parts.join(' > ') || el.tagName.toLowerCase();
            };
            const elems = document.querySelectorAll('a, button, input, select, textarea, [role="button"]');
            return Array.from(elems).slice(0, 60).map((el, i) => ({
                index: i,
                tag: el.tagName.toLowerCase(),
                text: (el.type === 'password' ? '' : (el.textContent || el.value || '')).trim().substring(0, 80),
                selector: uniqSel(el),
                visible: el.offsetParent !== null
            }));
        })()
        "#;

/// JS that collects deduped same-origin links (capped) for the PageState `links`
/// field. Scrapling LinkExtractor-level quality: canonicalize, filter non-http
/// schemes, strip fragments, drop content-obvious extensions (images, fonts,
/// pdf, archives, ico, css/js), dedupe in insertion order. Turns "crawl +
/// internal links" into a single navigate call.
/// ponytail: returns the array directly (like ELEMENTS_JS) so returnByValue gives
/// a JSON array — JSON.stringify would make from_value::<Vec<_>>() fail.
pub const LINKS_JS: &str = r#"
    (() => {
        try {
            const origin = location.origin;
            const skipExt = new Set(['pdf','zip','rar','7z','tar','gz','xz','bz2',
                'jpg','jpeg','png','gif','webp','avif','svg','ico','bmp','tif','tiff',
                'woff','woff2','ttf','otf','eot',
                'mp4','webm','mp3','ogg','wav','mov','avi','m4a',
                'css','js','json','xml','rss',
                'exe','dmg','iso','apk','msi']);
            const seen = new Set();
            const out = [];
            for (const el of document.querySelectorAll('a[href], area[href]')) {
                const raw = el.href || '';
                if (!raw) continue;
                // strip fragment + trailing slash; drop non-http
                let u = raw;
                const hash = u.indexOf('#');
                if (hash > -1) u = u.slice(0, hash);
                // drop non-http(s) schemes (mailto, javascript, tel, file, etc.)
                if (!u.startsWith('http://') && !u.startsWith('https://')) continue;
                // same-origin only
                if (!u.startsWith(origin)) continue;
                // trailing-slash normalise
                const qp = u.indexOf('?');
                let path = qp > -1 ? u.slice(0, qp) : u;
                if (path.endsWith('/')) u = u.slice(0, path.length - 1) + (qp > -1 ? u.slice(qp) : '');
                // drop known non-content extensions
                const seg = u.split('/').pop() || '';
                const dot = seg.lastIndexOf('.');
                if (dot > -1 && skipExt.has(seg.slice(dot + 1).toLowerCase())) continue;
                if (seen.has(u)) continue;
                seen.add(u);
                if (out.push(u) >= 200) break;
            }
            return out;
        } catch (e) { return []; }
    })()
    "#;

/// Abstraction over a browser backend.
/// ponytail: trait with async_trait when needed; dyn dispatch for now.
#[async_trait::async_trait]
pub trait BrowserBackend: Send + Sync {
    async fn navigate(&self, url: &str) -> anyhow::Result<PageState>;
    async fn screenshot(&self, full_page: bool) -> anyhow::Result<Vec<u8>>;
    async fn evaluate(&self, js: &str) -> anyhow::Result<serde_json::Value>;
    async fn click(&self, index: usize) -> anyhow::Result<()>;
    async fn type_text(&self, index: usize, text: &str) -> anyhow::Result<()>;
    /// Type text character-by-character with a per-keystroke delay (human-like
    /// pacing; Google flags a whole-string insertText). Backends without
    /// per-char input fall back to a single insertText.
    async fn type_text_delayed(
        &self,
        index: usize,
        text: &str,
        _delay_ms: u64,
    ) -> anyhow::Result<()> {
        self.type_text(index, text).await
    }
    async fn scroll(&self, direction: &str) -> anyhow::Result<()>;
    /// Press a key (Enter, Tab, ArrowDown...) in the focused element. Trusted
    /// CDP Input when supported, JS fallback (Enter -> form.submit) otherwise.
    async fn press(&self, key: &str) -> anyhow::Result<()>;
    /// Trusted click at raw viewport coords (cross-origin iframes / reCAPTCHA).
    async fn click_coords(&self, x: i64, y: i64) -> anyhow::Result<()>;
    /// Trace a human-like mouse path toward (x,y) (eased, jittered steps) before
    /// a click — a real pointer travels, it doesn't teleport. Backends without
    /// trusted input no-op (the click still lands).
    async fn mouse_move_human(&self, _x: i64, _y: i64) -> anyhow::Result<()> {
        Ok(())
    }
    /// Hard-reload the current page bypassing cache (Ctrl+Shift+R) — drops the
    /// anti-bot state Google set on a flagged request; the manual recipe that
    /// beats the /sorry wall. Default falls back to location.reload() (best-
    /// effort; CDP backends override with Page.reload ignoreCache:true — the
    /// boolean is non-standard and ignored by browsers).
    async fn reload_hard(&self) -> anyhow::Result<()> {
        self.evaluate("location.reload(); true").await.map(|_| ())
    }

    /// Trusted drag (press at x1,y1 → move with the button held → release at
    /// x2,y2) for drag-and-drop / slider CAPTCHAs. Crosses cross-origin iframes.
    async fn drag(&self, _x1: i64, _y1: i64, _x2: i64, _y2: i64) -> anyhow::Result<()> {
        anyhow::bail!("drag not supported by this backend")
    }
    async fn get_html(&self) -> anyhow::Result<String>;

    /// Register a JS init script (agent-browser `--init-script` borrow): runs
    /// before every FUTURE navigation via Page.addScriptToEvaluateOnNewDocument.
    async fn add_init_script(&self, _js: &str) -> anyhow::Result<()> {
        anyhow::bail!("add_init_script not supported by this backend")
    }

    /// Open a new tab at `url`; returns a tab id. Multi-page backends only.
    async fn open_tab(&self, _url: &str) -> anyhow::Result<String> {
        anyhow::bail!("open_tab not supported by this backend")
    }
    /// Switch the active tab by id.
    async fn activate_tab(&self, _id: &str) -> anyhow::Result<()> {
        anyhow::bail!("activate_tab not supported by this backend")
    }
    /// Close a tab by id.
    async fn close_tab(&self, _id: &str) -> anyhow::Result<()> {
        anyhow::bail!("close_tab not supported by this backend")
    }
    /// List open tabs: [{id, url, active}].
    async fn list_tabs(&self) -> anyhow::Result<serde_json::Value> {
        anyhow::bail!("list_tabs not supported by this backend")
    }
    /// Accessibility-tree snapshot of the current page (read-only; interact via elements[]).
    async fn a11y(&self) -> anyhow::Result<serde_json::Value> {
        anyhow::bail!("a11y not supported by this backend")
    }

    /// TRUSTED, evaluate-free element discovery: viewport center of the FIRST
    /// element matching `css` with a real layout rect, via the DOM domain
    /// (DOM.querySelectorAll → DOM.getContentQuads) — no page JS runs.
    /// None on backends without DOM/layout support (caller falls back).
    async fn element_center(&self, _css: &str) -> Option<(i64, i64)> {
        None
    }

    /// TRUSTED, evaluate-free consent-button discovery via the accessibility
    /// tree (Accessibility.getFullAXTree → backendDOMNodeId → getContentQuads):
    /// phase 1 = a button/link whose accessible name matches `patterns`;
    /// phase 2 = the last visible button/link. Returns (x, y, tag).
    async fn consent_button(&self, _patterns: &[&str]) -> Option<(i64, i64, String)> {
        None
    }

    /// TRUSTED, evaluate-free current URL (Page.getFrameTree — no page JS).
    async fn current_url(&self) -> Option<String> {
        None
    }

    /// Type into the FOCUSED element with real per-key Input.dispatchKeyEvent
    /// (no evaluate; the target field must already be focused/clicked).
    async fn type_focused(&self, _text: &str, _delay_ms: u64) -> anyhow::Result<()> {
        anyhow::bail!("type_focused not supported by this backend")
    }

    /// Prefetch / discovery-only navigation: visit `url` and return its outbound
    /// links, skipping full content extraction. Fast path for site-mapping
    /// (crawl4ai `prefetch=True`). Default impl falls back to navigate+evaluate.
    async fn discover_links(&self, url: &str) -> anyhow::Result<Vec<String>> {
        self.navigate(url).await?;
        let v = self
            .evaluate("Array.from(document.querySelectorAll('a[href]')).map(a => a.href)")
            .await?;
        let mut out = vec![];
        if let Some(arr) = v.as_array() {
            for item in arr {
                if let Some(s) = item.as_str() {
                    out.push(s.to_string());
                }
            }
        }
        Ok(out)
    }

    /// Capture a rectangular region of the page in CSS px (PixelRAG-style tiles).
    /// `scale` upscales the capture (>1 = higher-res output — the crop+upscale
    /// precision pass for small captcha tiles the 2B vision model misreads).
    async fn screenshot_clip(
        &self,
        _x: f64,
        _y: f64,
        _width: f64,
        _height: f64,
        _scale: f64,
    ) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("screenshot_clip not supported by this backend")
    }

    /// Evaluate JS inside a cross-origin iframe's isolated world (reCAPTCHA
    /// puzzle bframe, challenge frames) where parent-page JS can't reach.
    /// Returns None when the backend can't do it (non-CDP) or the frame isn't
    /// found — callers fall back to vision-locate heuristics.
    async fn eval_in_frame(
        &self,
        _url_contains: &str,
        _js: &str,
    ) -> anyhow::Result<Option<serde_json::Value>> {
        Ok(None)
    }

    /// Save the current page as PDF.
    async fn pdf(&self) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("pdf not supported by this backend")
    }

    /// Re-capture the current page state without navigating (D1: skip if unchanged).
    async fn snapshot(&self) -> anyhow::Result<PageState> {
        let url = self
            .evaluate("location.href")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();
        let title = self
            .evaluate("document.title")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();
        let text: String = self
            .evaluate("document.body ? document.body.innerText || '' : ''")
            .await?
            .as_str()
            .unwrap_or("")
            .to_string();
        let elements_val = self.evaluate(ELEMENTS_JS).await?;
        // An empty `elements` from a failed eval (Null / wrong shape) must not
        // mislabel a healthy page as a bot-limited "crippled" shell.
        let elements_ok = elements_val.is_array();
        let elements: Vec<InteractiveElement> =
            serde_json::from_value(elements_val).unwrap_or_default();
        let links: Vec<String> =
            serde_json::from_value(self.evaluate(LINKS_JS).await?).unwrap_or_default();
        let challenge = detect_antibot(&title, &text);
        let crippled = elements_ok && detect_crippled(&challenge, elements.len());
        let chrome_error = detect_chrome_error(&title, &text);
        Ok(PageState {
            url,
            title,
            text: text.chars().take(8000).collect(),
            elements,
            links,
            challenge,
            crippled,
            chrome_error,
        })
    }

    fn backend_name(&self) -> &'static str;
}
