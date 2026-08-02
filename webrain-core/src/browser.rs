/// A single page operation result: navigation, interaction, or extraction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PageResult {
    pub url: String,
    pub title: Option<String>,
    pub content: Option<String>,
    /// Base64-encoded PNG when screenshot was requested.
    pub screenshot_b64: Option<String>,
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
    /// Anti-bot challenge/block kind (`cloudflare_challenge`, `blocked`, `captcha`) when detected.
    pub challenge: Option<String>,
}

/// Classify a page as an anti-bot challenge/block page from its title+visible text.
/// crawl4ai `antibot_detector`, trimmed to the markers we actually hit. No HTML/network.
/// ponytail: tiny marker list; add structural HTML markers if sites evade it.
pub fn detect_antibot(title: &str, text: &str) -> Option<String> {
    let hay = format!("{title}\n{text}").to_ascii_lowercase();
    const MARKERS: &[(&str, &str)] = &[
        ("cloudflare_challenge", "checking your browser"),
        ("cloudflare_challenge", "just a moment"),
        ("cloudflare_challenge", "verify you are human"),
        ("cloudflare_challenge", "__cf_chl"),
        ("cloudflare_challenge", "challenge-platform"),
        ("blocked", "access denied"),
        ("blocked", "forbidden"),
        ("blocked", "cf-error-code"),
        ("captcha", "captcha"),
        ("captcha", "h-captcha"),
        ("captcha", "g-recaptcha"),
    ];
    MARKERS
        .iter()
        .find(|(_, m)| hay.contains(m))
        .map(|(kind, _)| kind.to_string())
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InteractiveElement {
    pub index: usize,
    pub tag: String,
    pub text: String,
    pub selector: String,
    pub visible: bool,
}

/// Abstraction over a browser backend.
/// ponytail: trait with async_trait when needed; dyn dispatch for now.
#[async_trait::async_trait]
pub trait BrowserBackend: Send + Sync {
    async fn navigate(&self, url: &str) -> anyhow::Result<PageState>;
    async fn screenshot(&self, full_page: bool) -> anyhow::Result<Vec<u8>>;
    async fn evaluate(&self, js: &str) -> anyhow::Result<serde_json::Value>;
    async fn click(&self, index: usize) -> anyhow::Result<()>;
    async fn type_text(&self, index: usize, text: &str) -> anyhow::Result<()>;
    async fn scroll(&self, direction: &str) -> anyhow::Result<()>;
    async fn get_html(&self) -> anyhow::Result<String>;

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
    async fn screenshot_clip(
        &self,
        _x: f64,
        _y: f64,
        _width: f64,
        _height: f64,
    ) -> anyhow::Result<Vec<u8>> {
        anyhow::bail!("screenshot_clip not supported by this backend")
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
        let elements: Vec<InteractiveElement> = serde_json::from_value(
            self.evaluate(crate::backends::cdp::ELEMENTS_JS).await?,
        )
        .unwrap_or_default();
        let challenge = detect_antibot(&title, &text);
        Ok(PageState {
            url,
            title,
            text: text.chars().take(8000).collect(),
            elements,
            challenge,
        })
    }

    fn backend_name(&self) -> &'static str;
}


