# webrain — Next-Gen AI Browser Tool Architecture

> **Superseded.** The authoritative, researched next-gen architecture and project
> structure live in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). This file is kept
> for history. (Formerly **Webrain**; renamed **webrain**.)

## Overview

Webrain is a Rust-native CDP browser agent tool that gives Claude, Gemini, and all AI models the power to fetch web pages, interact with pages, scrape content, batch process URLs, take screenshots, and spider websites — all through a stealth, anti-bot browser layer that bypasses Cloudflare and bot detection.

**Drop-in Puppeteer/Playwright replacement** — same patterns (Browser → Context → Page), different engine (Lightpanda + Obscura instead of Chromium).

## Why Rust

| Criterion | Rust | Python |
|-----------|------|--------|
| Browser engine integration | Obscura is Rust-native (V8 via deno_core) | Python FFI overhead |
| Stealth TLS fingerprinting | `wreq` in obscura-net: real Chrome TLS | No native TLS control |
| MCP server | obscura-mcp already exists | Reimplementation needed |
| CDP server (WebSocket) | obscura-cdp with 12+ domains | Would need full rebuild |
| Performance | Zero-copy, no GIL, sub-ms latency | GIL-bound |
| Python consumers | Via MCP protocol (language agnostic) | Native |
| Binary size | ~15MB single binary | 100MB+ with deps |

## Crate Dependency Graph

```
┌──────────────────────────────────────────────────────┐
│                 AI Models (Claude, Gemini, etc.)      │
│                       │  MCP stdio                   │
├──────────────────────────────────────────────────────┤
│                    webrain-cli                          │
│  webrain mcp │ webrain fetch │ webrain screenshot          │
│  webrain spider │ webrain batch-screenshot               │
├──────────────────────┬───────────────────────────────┤
│     webrain-mcp        │                               │
│  11 MCP tools        │                               │
│  stdio JSON-RPC      │                               │
├──────────────────────┤                               │
│     webrain-core       │     (future: webrain-python)    │
│  UnifiedBrowser      │     (PyO3 bindings)           │
│  BatchEngine         │                               │
│  ScreenshotEngine    │                               │
│  SpiderEngine        │                               │
│  SessionPool         │                               │
│  AntiBotConfig       │                               │
├──────────┬───────────┤                               │
│ Obscura  │ Lightpanda│                               │
│ (Rust/V8)│ (Zig/V8)  │                               │
│ native   │ CDP WS    │                               │
└──────────┴───────────┘                               │
```

## Crate Details

### webrain-core (`webrain-core/`)

The core library. Zero binary — just types, traits, and engines.

| Module | Purpose | Key Types |
|--------|---------|-----------|
| `browser.rs` | `BrowserBackend` trait + `UnifiedBrowser` | `PageState`, `PageResult`, `InteractiveElement` |
| `engines.rs` | Parallel batch operations + spider/crawler | `BatchEngine`, `ScreenshotEngine`, `SpiderEngine` |
| `session.rs` | Browser session lifecycle | `SessionPool` |
| `stealth.rs` | Anti-bot configuration | `AntiBotConfig` |

#### BrowserBackend trait

```rust
#[async_trait::async_trait]
pub trait BrowserBackend: Send + Sync {
    async fn navigate(&self, url: &str) -> anyhow::Result<PageState>;
    async fn screenshot(&self, full_page: bool) -> anyhow::Result<Vec<u8>>;
    async fn evaluate(&self, js: &str) -> anyhow::Result<serde_json::Value>;
    async fn click(&self, index: usize) -> anyhow::Result<()>;
    async fn type_text(&self, index: usize, text: &str) -> anyhow::Result<()>;
    async fn scroll(&self, direction: &str) -> anyhow::Result<()>;
    async fn extract_content(&self, query: &str) -> anyhow::Result<String>;
    async fn get_html(&self) -> anyhow::Result<String>;
    fn backend_name(&self) -> &'static str;
}
```

Two planned implementations:
1. **ObscuraBackend** — wraps `obscura::Browser` natively (Rust)
2. **LightpandaBackend** — CDP WebSocket client to `lightpanda serve`

### webrain-mcp (`webrain-mcp/`)

MCP stdio server. Registered as:

```json
{
  "mcpServers": {
    "webrain": {
      "command": "webrain",
      "args": ["mcp"]
    }
  }
}
```

#### MCP Tools

| Tool | Input | Output |
|------|-------|--------|
| `webrain_navigate` | `url: string` | `PageState` (title, text, elements) |
| `webrain_screenshot` | `full_page?: bool` | `{screenshot_b64: string}` |
| `webrain_click` | `index: integer` | Element clicked confirmation |
| `webrain_type` | `index: integer, text: string` | Text entered confirmation |
| `webrain_scroll` | `direction: "up" \| "down"` | Scroll status |
| `webrain_extract` | `query: string` | Structured extraction results |
| `webrain_get_html` | `selector?: string` | HTML content |
| `webrain_batch_fetch` | `urls: string[], concurrency?: int` | `PageResult[]` |
| `webrain_batch_screenshot` | `urls: string[], full_page?: bool` | `{url, screenshot_b64}[]` |
| `webrain_spider` | `seed_url: string, max_depth?, max_pages?` | `SpiderResult[]` with nested links |

### webrain-cli (`webrain-cli/`)

Single binary with subcommands:

```
webrain mcp                    # Start MCP server (default)
webrain fetch <url>            # Fetch a page
webrain screenshot <url>       # Screenshot a page
webrain spider <seed_url>      # Crawl from a URL
```

### webrain-python (deferred)

Python bindings via PyO3. Deferred because:
- MCP is the primary AI interface (Claude Desktop, Cursor, etc.)
- Python 3.14 isn't supported by current PyO3
- `uvx webrain` or `pip install webrain` when needed

Planned API:
```python
from webrain import WebrainBrowser

async with WebrainBrowser(stealth=True) as browser:
    page = await browser.new_page()
    await page.goto("https://example.com")
    print(await page.content())
    screenshot = await page.screenshot()
```

## Anti-Bot Strategy

Three layers, borrowed from obscura and camoufox research:

1. **TLS Fingerprint** (obscura-net `StealthHttpClient`):
   - Real Chrome TLS ClientHello, ALPN, cipher order
   - Defeats JA3/JA4 bot management
   - Consistent with User-Agent

2. **Browser Fingerprint** (at JS level):
   - `navigator.webdriver` → `false`
   - Consistent `navigator.platform`, `screen`, `WebGL`
   - Native function `.toString()` returns `[native code]`

3. **Tracker Blocking** (obscura-net blocklist):
   - 3,520 known tracker domains blocked
   - Request-level blocklist, not DOM-level

## Request Flow

```
1. AI Model calls webrain_navigate("https://example.com")
2. webrain-mcp receives JSON-RPC message on stdin
3. tools::call_tool() dispatches to webrain_navigate handler
4. SessionPool.get_or_create() returns UnifiedBrowser
5. UnifiedBrowser.navigate() calls BrowserBackend.navigate()
6. ObscuraBackend (Rust) or LightpandaBackend (CDP WS)
   a. Creates isolated BrowserContext
   b. Navigates to URL
   c. Waits for load event
   d. Captures PageState (title, text, elements)
7. Result serialized and sent back to AI model via stdout
```

## Session Isolation

Following obscura's `BrowserContext::isolated_copy()` pattern:
- Each MCP session gets its own browser context
- Cookie jars are independent (one session cannot read another's cookies)
- Proxy + fingerprint can differ per context

## Integration with obscura (TODO)

When the obscura crate is available as a git dependency:

```rust
// webrain-core/src/backends/obscura.rs
pub struct ObscuraBackend {
    browser: obscura::Browser,
}

impl BrowserBackend for ObscuraBackend {
    async fn navigate(&self, url: &str) -> anyhow::Result<PageState> {
        let page = self.browser.new_page().await?;
        page.goto(url).await?;
        // Capture state via CDP Runtime.evaluate
        // ...
    }
}
```

## Integration with Lightpanda (TODO)

Lightpanda communicates via CDP WebSocket (like Chrome):

```rust
// webrain-core/src/backends/lightpanda.rs
pub struct LightpandaBackend {
    ws_url: String,  // ws://127.0.0.1:9222/devtools/browser
}

impl BrowserBackend for LightpandaBackend {
    // All methods -> CDP WebSocket messages
    // Page.navigate, Runtime.evaluate, Page.captureScreenshot, etc.
}
```

## Project Structure

```
webrain/
├── Cargo.toml              # Workspace root
├── ARCHITECTURE.md          # This file
├── webrain-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs           # Re-exports
│       ├── browser.rs       # BrowserBackend trait + UnifiedBrowser
│       ├── engines.rs       # BatchEngine, ScreenshotEngine, SpiderEngine
│       ├── session.rs       # SessionPool
│       └── stealth.rs       # AntiBotConfig
├── webrain-mcp/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs           # MCP stdio server (run_stdio)
│       └── tools.rs         # Tool definitions + dispatch
├── webrain-cli/
│   ├── Cargo.toml
│   └── src/
│       └── main.rs          # webrain binary entry point
└── target/                  # Build artifacts
```

## Decisions

| Decision | Rationale |
|----------|-----------|
| **Rust over Python** | Obscura already has CDP+MCP+stealth in Rust; no rebuild needed |
| **MCP as primary AI interface** | Claude/Gemini/Cursor all speak MCP natively; no custom protocol |
| **Obscura as primary backend** | Native Rust integration, no subprocess. Lightpanda secondary via CDP |
| **Deferred Python bindings** | MCP covers AI use case; PyO3 0.24 doesn't support Python 3.14 |
| **No trait abstraction framework** | One trait, two backends — no middleware, no factories |
| **BFS spider, no priority queue** | Simple visited set + queue; add BestFirst when needed |
| **Single SessionPool** | HashMap-based; add TTL/eviction when multi-user needed |

## References

- [Obscura](https://github.com/h4ckf0r0day/obscura) — Rust headless browser with CDP + MCP + stealth
- [Lightpanda](https://github.com/lightpanda-io/browser) — Zig headless browser with CDP + MCP
- [Camoufox](https://github.com/daijro/camoufox) — Firefox fork, fingerprint injection at C++ level
- [Crawl4AI](https://github.com/unclecode/crawl4ai) — Python crawling strategies (extraction patterns reference)
- [Browser-Use](https://github.com/browser-use/browser-use) — MCP server tool pattern reference
- [Browser-Harness](https://github.com/browser-use/browser-harness) — Thin CDP daemon pattern reference
- [PixelRAG](https://github.com/StarTrail-org/PixelRAG) — RAG approach for browser content
