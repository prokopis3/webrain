<div align="center">

<img src="assets/webrain-logo-rounded.png" alt="webrain" width="180" height="180">

# WebRain

**A portable, LLM-driven browser-automation & web-scraping MCP server — one binary, three engines, any OS.**

WebRain exposes 17 intent-based tools over the **Model Context Protocol**. Install it on any system, point any LLM client (GitHub Copilot, Claude, Codex, Cursor, …) at it, and the model decides everything — search, crawl, scrape, navigate, interact — from a plain-language prompt. No hardcoded intent detection, no daemon, no Node.js.

[![Latest Release](https://badgen.net/github/release/prokopis3/webrain?icon=github)](https://github.com/prokopis3/webrain/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20%7C%20macOS%20%7C%20Windows-blue?style=flat-square)](https://github.com/prokopis3/webrain/releases)
[![Last Commit](https://badgen.net/github/last-commit/prokopis3/webrain/main?icon=github)](https://github.com/prokopis3/webrain/commits/main)
[![GitHub Stars](https://badgen.net/github/stars/prokopis3/webrain?icon=github)](https://github.com/prokopis3/webrain/stargazers)
[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](Cargo.toml)

</div>

---

## Why webrain?

Web automation shouldn't mean wiring up a driver, a browser download, and your own tool wrappers before an LLM can touch a page. Webrain collapses that into **one binary + one install command** and speaks MCP, so any LLM client drives it directly.

| | **webrain** | Raw Playwright / Puppeteer |
|---|---|---|
| **Setup** | one binary + `webrain install` | runtime + driver + browser download + your own wrappers |
| **Browsers** | Chrome + lightpanda + obscura through one CDP backend | one engine, usually Chromium |
| **LLM-ready** | MCP server, 17 tools, built-in decision guide | you hand-roll tool functions |
| **Anti-bot** | challenge detection + persistent profile + native login | manual |
| **Extraction** | autoschema → JSON / regex / table / spider / batch | you write selectors |
| **Runs on** | any OS, any LLM client, or plain CLI | tied to your stack |

## What you can do with it

- **Scrape at scale** — batch pagination + spider with auto-throttle and checkpoint/resume; `webrain_sitemap` / `webrain_scan` to map a site first.
- **Structured data without hand-written selectors** — `webrain_autoschema` probes the DOM, then JSON / regex / table extractors read container-level structure.
- **Stealth login** — real-Chrome profiles with an encrypted local credential vault (AES-256-GCM + optional TOTP); transfer cookies across engines.
- **Get past challenges** — reads the `challenge` field on every navigate; protected sites use real Chrome + a persistent profile + session (native login). Native CAPTCHA/vision v2 (drag, eval_in_frame, vision ask) — no Python sidecar.
- **See the page** — a11y / semantic tree, snapshots, and vision tiles (screenshot → vector store) for tables and charts.
- **Structured search** — `webrain_serp` returns typed JSON (`position`/`title`/`url`/`domain`/`snippet`) across duckduckgo · bing · google · brave, deduped, paginated, region-pinned, with proxy + 2captcha + serpapi fallbacks.
- **Read anything** — PDFs (extract + render), JSON-LD, media, plus `fetch_http` for static pages 10–100× faster than a browser.

---

## Tech Stack

- **Language**: Rust (edition 2024, MSRV 1.85)
- **Workspace**: `webrain-core` (CDP client, engines, vault, launch, install) · `webrain-mcp` (MCP server + tool schemas) · `webrain-cli` (single binary)
- **Protocol**: Model Context Protocol (stdio + HTTP transports)
- **Browser automation**: Chrome DevTools Protocol over `tokio-tungstenite` (raw WebSocket)
- **Engines**: Chrome for Testing · Lightpanda (Zig) · Obscura (Rust, V8) · `fetch_http`
- **Crypto**: AES-256-GCM vault, SHA-256/HMAC/SHA-1 (TOTP)
- **PDF**: lopdf + pdf-inspector (pure Rust)
- **Deployment**: Docker (multi-arch), GitHub Actions CI/CD

## Prerequisites

- **OS**: Windows, macOS, or Linux (x86_64 / arm64).
- **A browser engine** — run `webrain install` once (downloads Chrome for Testing). Obscura and Lightpanda are optional extra engines.
- **Linux**: system libraries for Chrome (list below) before first run.
- **Docker** — only to run the obscura / lightpanda engines in containers.
- **An MCP-capable client** (VS Code + Copilot, Claude, Codex, Cursor, …) — optional; the CLI works standalone.

## Installation

> Install model mirrors [vercel-labs/agent-browser](https://github.com/vercel-labs/agent-browser): a native binary plus an `install` command that downloads engines into a cache dir.

### Global Installation (recommended)

One command installs `webrain` on your PATH:

**Linux / macOS:**

```bash
curl -fsSL https://raw.githubusercontent.com/prokopis3/webrain/main/scripts/install.sh | bash
```

**Windows (PowerShell):**

```powershell
irm https://raw.githubusercontent.com/prokopis3/webrain/main/scripts/install.ps1 | iex
```

Then, in any terminal:

```bash
webrain install          # Download Chrome for Testing (first time only)
webrain mcp --http 9223  # start the MCP server
```

> Prefer a manual download? The per-OS commands below do the same without a script.

**Windows (PowerShell):**

```powershell
$dir = "$env:LOCALAPPDATA\Programs\webrain"; New-Item -ItemType Directory -Force -Path $dir | Out-Null
Invoke-WebRequest "https://github.com/prokopis3/webrain/releases/latest/download/webrain-windows.exe" -OutFile "$dir\webrain.exe"
[Environment]::SetEnvironmentVariable("Path", "$([Environment]::GetEnvironmentVariable('Path','User'));$dir", "User")
# open a new terminal, then:
webrain install          # Download Chrome for Testing (first time only)
webrain mcp --http 9223  # start the MCP server
```

**Linux / macOS:**

```bash
curl -L -o ~/.local/bin/webrain https://github.com/prokopis3/webrain/releases/latest/download/webrain-linux
chmod +x ~/.local/bin/webrain
webrain install          # Download Chrome for Testing (first time only)
webrain mcp --http 9223  # start the MCP server
```

> macOS: swap `webrain-linux` for `webrain-macos` in the curl line.

### Project Installation

Pin a version as a local dependency:

```bash
cargo add webrain --git https://github.com/prokopis3/webrain
# or build from this repo (see From Source)
```

### Cargo (Rust)

```bash
cargo install --git https://github.com/prokopis3/webrain webrain-cli
webrain install   # Download Chrome (first time only)
```

### Homebrew (macOS)

```bash
brew tap prokopis3/webrain
brew install webrain
webrain install   # Download Chrome (first time only)
```

### Scoop (Windows)

From the official **extras** bucket (after [PR #18455](https://github.com/ScoopInstaller/Extras/pull/18455) merges):

```powershell
scoop install extras/webrain
```

Or from the project's own bucket:

```powershell
scoop bucket add webrain https://github.com/prokopis3/scoop-webrain
scoop install webrain
webrain install   # Download Chrome (first time only)
```

### Linux Dependencies

On Linux, Chrome needs system libraries. Install them with your package manager before first run:

```bash
# Debian/Ubuntu
sudo apt-get install -y libnss3 libnspr4 libxkbcommon0 libatk1.0-0 \
  libatk-bridge2.0-0 libxcomposite1 libxdamage1 libxrandr2 libxfixes3 \
  libxcursor1 libxi6 libxtst6 libxss1 libxext6 fonts-liberation

# Fedora
sudo dnf install -y nss nspr libxkbcommon atk at-spi2-atk at-spi2-core \
  libXcomposite libXdamage libXrandr libXfixes libXcursor libXi libXtst \
  libXScrnSaver libXext
```

If you see *"shared library"* errors when running Chrome, that's the missing-deps symptom — install the list above.

### From Source

```bash
git clone https://github.com/prokopis3/webrain
cd webrain
cargo build --release --bin webrain
./target/release/webrain install   # Download Chrome (first time only)
./target/release/webrain mcp --http 9223
```

### Updating

Upgrade to the latest version:

```bash
webrain upgrade
```

Detects your installation method (Homebrew, Scoop, or a manual install) and
updates automatically — `brew upgrade webrain`, `scoop update webrain`, or
self-updates the binary in place.

If installed via cargo:

```bash
cargo install --git https://github.com/prokopis3/webrain webrain-cli --force
```

Re-download engines after an update (cache dir stays, versions are additive):

```bash
webrain install
webrain install --engine obscura
```

### OS Compatibility

**Yes — the codebase targets all three desktop OSes.** Engines per OS:

| Engine | Windows | macOS | Linux |
|---|---|---|---|
| **Chrome for Testing** (`webrain install`) | ✅ | ✅ | ✅ |
| **Obscura** (`webrain install --engine obscura`) | ✅ x86_64 | ✅ x86_64 / arm64 | ✅ x86_64 / arm64 |
| **Lightpanda** (`webrain lightpanda`, binary needed) | ⚠️ (needs a Windows build on PATH) | ✅ | ✅ |
| **Docker** (`docker/Dockerfile`) | via Docker Desktop | via Docker Desktop | ✅ |

Binary discovery is automatic on every OS: env override → PATH → `~/.lightpanda`, `~/.obscura`, `~/.local/bin` → the webrain engine cache.

## Quick Start

```bash
# 1. Get a browser engine
webrain install

# 2. Start the MCP server (stdio for VS Code/Copilot, or HTTP)
webrain mcp                        # stdio
webrain mcp --http 9223            # HTTP transport on 127.0.0.1:9223

# 3. Point an LLM at it. Register the MCP server in your client (see "Marketplace / IDE Plugins"):
#    stdio:  {"servers": {"webrain": {"command": "webrain", "args": ["mcp"]}}}
#    HTTP:   webrain mcp --http 9223  →  {"servers": {"webrain": {"type": "http", "url": "http://127.0.0.1:9223/mcp"}}}

# 4. Ask the LLM to do something, e.g. "scrape all product titles + prices from URL X"
```

Prefer to drive it by hand? Every engine + tool has a CLI twin:

```bash
webrain launch scrapingcourse demo "https://example.com/login" --port 9222   # headed Chrome + login
webrain doctor                 # full diagnosis: engines, MCP, CDP, vault
webrain fetch <url>             # attach to CDP_URL and fetch
webrain screenshot <url>
webrain eval "document.title"
```

### Traditional Selectors (also supported)

The MCP tools take plain CSS selectors directly (e.g. `webrain_click` with
`selector: "#submit"`), and the CLI works on snapshot refs — the same elements
`snapshot` reports, 1-indexed:

```bash
webrain launch https://example.com
webrain snapshot          # prints interactive elements, 1-indexed
webrain click 3           # click element #3
webrain type 5 "text"     # type into element #5
webrain eval 'document.querySelector("#email").value'
```

## Commands

```bash
webrain mcp [--http <port>]                        # MCP server (stdio, or HTTP on a port)
webrain doctor                                     # diagnose the install (engines, MCP, CDP, vault)
webrain install [--engine chrome|obscura] [--stealth]  # download a browser engine
webrain upgrade                                    # update to the latest release
webrain launch <service> <profile> [url]           # stealth Chrome, persistent profile
webrain login <service> <profile> [url]            # interactive login into a profile
webrain vault set|list|user|rm                     # encrypted credential vault (AES-256-GCM + TOTP)
webrain cookies / setcookies <file>                # export / import cookies
webrain fetch <url>                                # attach to CDP_URL and fetch
webrain screenshot <url>                           # screenshot (single or full page)
webrain spider <url> [--depth N --pages N --respect-robots]  # crawl
webrain click <i> / type <i> <text> / eval <js>    # drive the CDP_URL backend
webrain obscura / lightpanda [--port N]            # spawn a CDP server
webrain watch <video-url-or-file> [--vision]       # transcript + frames (no browser)
webrain install watch|whisper|vision [--model]     # local AI stack (whisper + Qwen3-VL-2B)
webrain serp "<query>" [--engine …] [--limit N]   # structured JSON search (5 engines + auto)
```

The 17-tool MCP surface is discovered dynamically — `webrain_guide` lists
it for the LLM (see [MCP Tools](#mcp-tools)).

## Browser Engines

| Need | Engine | How to get it |
|---|---|---|
| Material / interactive SPA (Google Flights, calendars, dropdowns), interactive challenges | **real Chrome** | `webrain install` |
| Fast scraping of non-challenged JS pages, parallel tabs; v0.2.0+ render builds also screenshot + PDF | **obscura** | `webrain install --engine obscura [--stealth] [--no-render]`, then `webrain obscura` |
| Fastest/lightest, real a11y + semantic tree, no rendering | **lightpanda** | install the binary, then `webrain lightpanda` |
| Static HTML, no JS/auth | **fetch_http** (no browser) | built-in |

**Key rules (see [Agent Decision Guide](#agent-decision-guide)):**

- **Never** use obscura/lightpanda for Material/SPA interaction — they still lack the layout engine for complex Material UI widgets. Obscura v0.2.0+ **render builds** can screenshot and export PDFs (native Rust renderer). Lightpanda returns a *fake placeholder PNG*. Route interactive SPAs to real Chrome via `cdp_urls:["http://127.0.0.1:9222"]`.
- Read the `challenge` field after every `webrain_navigate`. If it's non-null, the page is gated — use real Chrome + a persistent profile + session (`webrain launch` / `webrain login`).
- Extract from container/card-level DOM, not bare `$` text nodes (Google Flights renders a spurious price grid).

## MCP Tools

The surface is **intent-based** (firecrawl-style) — each tool has a `what` / `action` / `op` / `mode` selector, so the LLM picks a boundary, not a primitive. Every capability is preserved as a selector value, and all 17 tools are discovered dynamically (`webrain_guide` lists them for the LLM):

| Tool | What it does |
|---|---|
| `webrain_navigate` | Go to a URL — page state + `challenge` / `crippled` detection. The entry point. |
| `webrain_observe` | Read the current page: `what` = state · a11y · semantic · html · images · console · flatten (Shadow DOM) · fit · clean · screenshot · pixel · page_info · annotate · media |
| `webrain_interact` | Drive the page: `action` = click · click_coords · type · press · scroll · nav · tab · select · hover · check · dialog · wait · upload · dismiss_overlays · add_init_script |
| `webrain_extract` | Structured data: `mode` = schema (CSS) · regex · jsonld · table · autoschema · bm25 |
| `webrain_scrape` | No-browser HTTP fetch of one URL (10–100× faster, static only) |
| `webrain_batch` | Same op across many URLs in parallel tabs: `op` = fetch · extract · interact · eval · screenshot (+ `cdp_urls` per-proxy fan-out) |
| `webrain_crawl` | Site traversal: `mode` = spider (BFS/DFS/best-first + autothrottle + checkpoint) · sitemap · scan · validate |
| `webrain_search` | Web search (duckduckgo · google · bing · brave) |
| `webrain_serp` | Structured JSON search: `position`/`title`/`url`/`domain`/`snippet` across duckduckgo · bing · google · brave · auto (dedupe · paginate · region-pin · proxy · 2captcha) |
| `webrain_pdf` | PDF work: `op` = page · extract (→ markdown) · render (→ vision tiles) · images |
| `webrain_download` | Files/media: `engine` = http (stream) · ytdlp (HLS/playlists) |
| `webrain_watch` | Video → timestamped transcript + frames (no browser; `vision:true` → text captions) |
| `webrain_session` | Browser/auth/session: `op` = open · close · list · cookies · setcookies · save_state · restore_state · profiles · login · close_launch |
| `webrain_vision` | Screenshot-tile vision index: `op` = index · retrieve |
| `webrain_eval` | Arbitrary JS in the page (escape hatch) |
| `webrain_eval_in_frame` | Run JS inside a cross-origin iframe (isolated world) |
| `webrain_guide` | Agent decision guide (browser/challenge/extraction/delegation) |

Legacy one-action tool names still dispatch (backward compatible via `map_surface()`).

Full per-tool reference: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Tool/browser/challenge decisions: [`docs/AGENT_DECISION_GUIDE.md`](docs/AGENT_DECISION_GUIDE.md).

## Watch videos (local AI, no browser)

`webrain_watch` (MCP) / `webrain watch` (CLI) turns **any video** — URL or
local file — into a timestamped transcript + frames, with **no browser**:

```bash
webrain install watch             # one command: bundles ffmpeg+ffprobe, yt-dlp,
                                  # whisper-cli + a GGUF model (self-contained, any OS)
webrain watch video.mp4           # transcript (captions → local whisper → cloud) + frames
webrain watch video.mp4 --vision  # + text captions of the frames via a vision LLM
webrain install vision            # local Qwen3-VL-2B (llama-server + GGUF + mmproj)
```

Vision chain (`--vision` / `vision:true`): **Groq `qwen3.6-27b` → OpenAI
`gpt-4o-mini` → local Qwen3-VL-2B** — when no key is set, video understanding
runs fully **offline** after `webrain install watch` + `webrain install vision`.
Cloud keys (`GROQ_API_KEY` / `OPENAI_API_KEY` / `FIREWORKS_API_KEY`) are optional
if you run the local stack. `webrain install watch` prints a `warnings` block if
anything's missing.

## CLI Reference

| Command | Description |
|---|---|
| `webrain mcp [--http <port>]` | Start the MCP server (stdio, or HTTP on a port) |
| `webrain install [--force] [--engine chrome\|obscura] [--stealth]` | Download a browser engine into the cache |
| `webrain obscura [--port N]` | Spawn the Obscura CDP server (default 9224) |
| `webrain lightpanda [--port N]` | Spawn the Lightpanda CDP server (default 9225) |
| `webrain launch <service> <profile> [url] [--headless] [--port N]` | Spawn a stealth Chrome with a persistent per-account profile |
| `webrain login <service> <profile> [url] [--port N]` | Launch + attach for interactive login |
| `webrain cookies [--port N] [--out file]` / `webrain setcookies <file>` | Export / import cookies |
| `webrain fetch <url>` · `webrain screenshot <url>` · `webrain spider <url>` · `webrain click <i>` · `webrain type <i> <text>` · `webrain eval <js>` | Drive the `CDP_URL` backend |
| `webrain vault set\|list\|user\|rm` | Manage encrypted credentials (hidden prompts) |
| `webrain doctor` | Full install diagnosis — version, MCP server, CDP ports (9222/9224/9225), engine discovery, vault, `recommend`. `--doctor` alias |

## Environment Variables

| Variable | Description | Default |
|---|---|---|
| `CDP_URL` | CDP endpoint to attach to | `http://127.0.0.1:9222` |
| `WEBRAIN_CHROME` | Explicit Chrome binary path | auto-discovered |
| `WEBRAIN_BROWSERS_DIR` | Engine cache dir (Chrome for Testing, Obscura) | `%LOCALAPPDATA%\webrain\browsers` / `~/.cache/webrain/browsers` |
| `WEBRAIN_LIGHTPANDA` / `WEBRAIN_OBSCURA` | Explicit lightpanda/obscura binary paths | auto-discovered |
| `WEBRAIN_PROFILES_DIR` | Per-account browser profile root | `%APPDATA%\webrain\profiles` / `~/.config/webrain/profiles` |
| `WEBRAIN_VAULT_DIR` | Encrypted credential vault dir | `%APPDATA%\webrain` / `~/.config/webrain` |
| `WEBRAIN_USER` / `WEBRAIN_PASS` | Login fallback credentials (env channel) | — |
| `GROQ_API_KEY` / `OPENAI_API_KEY` / `FIREWORKS_API_KEY` | Cloud STT (transcript) keys; `GROQ`/`OPENAI` also power the vision chain | — |
| `WEBRAIN_STT_MODEL` | Cloud STT model override (e.g. `whisper-large-v3`) | provider default |
| `WEBRAIN_WHISPER_BIN` / `WEBRAIN_WHISPER_MODEL` | Local whisper-cli binary / GGUF model paths | bundled cache |
| `RUST_LOG` | Log verbosity (`webrain=info,tungstenite=warn`) | as above |

## Marketplace / IDE Plugins

Webrain is an **MCP server**, so it plugs into any MCP-capable IDE. There is no separate extension to publish — you register the server, and the 17 tools appear.

Both transports work in every client: **stdio** (`webrain mcp`) or **HTTP**
(`webrain mcp --http 9223`, endpoint `http://127.0.0.1:9223/mcp`).

**VS Code (GitHub Copilot)**

Add to your user `settings.json` under `"mcp"`:

```jsonc
{
  "mcp": {
    "servers": {
      "webrain": {
        "type": "stdio",
        "command": "webrain",
        "args": ["mcp"],
        "env": { "CDP_URL": "http://127.0.0.1:9222" }
      }
    }
  }
}
```

**Claude Desktop** — `~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "webrain": { "command": "webrain", "args": ["mcp"] }
  }
}
```

**Cursor** — project `.mcp.json`:

```json
{
  "mcpServers": {
    "webrain": { "command": "webrain", "args": ["mcp"] }
  }
}
```

**Agent skills (optional)** — this repo ships agent guidance so the LLM drives it well:

- `skills/webrain/SKILL.md` — browser/challenge/extraction decision guide + auth-cookie transfer procedures.
- `.github/skills/` — repo-local skills (`credentials`, `update-changelog`, `git-cleanup`).

## Architecture

```
webrain (single binary)
├── webrain-core            CDP client + engines + vault + launch + install
│   ├── backends/cdp.rs     BrowserBackend over CDP (WS): navigate/eval/click/a11y/…  (+ STEALTH_JS)
│   ├── engines.rs          Tile (vision tiles), Spider (BFS/DFS/best-first + AutoThrottle), extract, BM25
│   ├── install.rs          engine download/discovery (Chrome for Testing, Obscura, Lightpanda)
│   ├── launch.rs           spawn Chrome/lightpanda/obscura, wait for CDP
│   ├── login.rs / vault.rs encrypted credential vault + CDP login injection (TOTP)
│   └── vision.rs           screenshot-tile embedding + vector store
├── webrain-mcp             MCP server: list_tools / call_tool, 15 consolidated tools (63 legacy via map_surface)
└── webrain-cli             subcommand dispatch (mcp | install | launch | login | …)
```

**How it works:**

1. `CdpBackend` connects to a CDP endpoint (Chrome, lightpanda, or obscura — all speak CDP) over a raw WebSocket.
2. On attach it applies stealth hardening (JS patches + fingerprint-noise evasions
   ON by default, no forged UA / `Emulation.setAutomationOverride` — patchright
   parity) and waits out Cloudflare/captcha interstitials natively — so it can
   log into real sites.
3. The MCP layer exposes every action as a tool. An LLM picks tools by intent; `webrain_guide` + `AGENT_DECISION_GUIDE.md` encode the *which-browser / which-tool* decisions so the LLM never guesses.
4. Extraction is generic — autoschema probes the DOM, JSON/regex/table extractors read container-level structure, spider/batch/sitemap crawl at scale, vision tiles give the model "eyes" for tables/charts.

**Data flow (LLM → browser):**

```
LLM prompt → tool call (webrain_navigate/extract/batch/…) → CdpBackend → CDP → engine (Chrome/lightpanda/obscura)
        ← JSON result (PageState, extracted rows, stats) ←
```

## Agent Decision Guide

**Read [`docs/AGENT_DECISION_GUIDE.md`](docs/AGENT_DECISION_GUIDE.md) before browser tasks.** It encodes, with live-verified results:

- **Browser selection** — Material/SPA → real Chrome via `cdp_urls`; Cloudflare/Turnstile → Chrome + persistent profile/session (native login); fast JS scraping → obscura; lightpanda for real a11y with minimal footprint; static → `fetch_http`.
- **Challenge handling** — read `challenge` after every navigate; obscura/lightpanda cannot pass interactive challenges.
- **Extraction matrix** — autoschema → extract_json → batch → regex → table → spider; never guess selectors.
- **a11y** — Google/Material widgets are `combobox`/`option`/`tab`, not `button`; `filter` matches name/value/css_path; if `role=<x>` returns `[]`, drop the role and filter by label.

## Testing

```bash
# run the self-check tests in webrain-core
cargo test --package webrain-core

# run everything
cargo test --workspace

# lint
cargo clippy --workspace --all-targets
cargo fmt --check
```

The repo favors **one runnable self-check per non-trivial module** (assert-based, no test framework sprawl) — see the `#[cfg(test)]` blocks in `webrain-core/src/engines.rs` and `install.rs`.

## CI/CD & Releases

GitHub Actions (`.github/workflows/`):

- **`ci.yml`** — `cargo fmt --check` + `clippy` + `test` on every push/PR to `main`.
- **`release.yml`** — on a `v*` tag push: builds the binary natively on Linux/Windows/macOS, generates the CHANGELOG entry for that version (if missing), and creates the GitHub Release with the changelog section as the body.
- **`pr-lint.yml`** — enforce conventional-commit PR titles (`<type>(<scope>): <description>`).
- **`changelog-enforce.yml`** — require the changelog to be updated when source changes (PRs and pushes to `main`).

Commit convention: `<type>(<scope>): <description>` — types `feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert`, scopes `core|engines|mcp|tools|cli|install|launch|login|vault|vision|pdf|download|cookies|batch|extraction|navigation|stealth|antibot|session|media|docs|build|ci|style|perf|dist|deps|config|test|skill|script|release`.

## Deployment (Docker)

A self-contained image with Chromium bundled lives in [`docker/`](docker/) with a `docker-compose.yml`:

```bash
# build (context is the repo root, so the workspace crates are visible)
docker build -f docker/Dockerfile -t webrain .

# run (HTTP MCP on 9223)
docker run -p 9223:9223 webrain mcp --http 9223

# or via compose — also mounts persistent volumes for the vault + profiles + engine cache
docker compose -f docker/docker-compose.yml up -d

# multi-arch
docker buildx build --platform=linux/amd64,linux/arm64 -t ghcr.io/prokopis3/webrain -f docker/Dockerfile .
```

## Troubleshooting

**`failed to spawn Chrome`** → Chrome isn't installed or `WEBRAIN_CHROME` is wrong. Run `webrain install` to download Chrome for Testing.

**`port X already has a CDP endpoint`** → a browser is already running there. Stop it, or pick another port (`--port N`).

**Screenshots fail / blank on obscura or lightpanda** → obscura v0.2.0+ render builds support screenshots; older builds and lightpanda have no paint engine (lightpanda returns a *fake placeholder PNG*). Use real Chrome.

**Empty a11y tree** → the page likely never rendered (consent/challenge page) or the control needs interaction. Check `webrain_navigate`'s `challenge`; try real Chrome; drop the `role` filter and use `filter` on the label.

**`lightpanda not found` / `obscura not found`** → install the binary (see [Browser Engines](#browser-engines)) or set `WEBRAIN_LIGHTPANDA` / `WEBRAIN_OBSCURA`.

**`webrain doctor` shows MCP down** → start `webrain mcp --http 9223`; if CDP ports are down, `webrain install` then start an engine.

**Cloudflare/Turnstile blocks the scrape** → read the `challenge` field and use real Chrome with a persistent profile + session: `webrain launch <service> <profile> <url>` then `webrain login <service> <profile>`, or re-attach an already-authenticated Chrome via `CDP_URL`.

## Changelog

See [`CHANGELOG.md`](CHANGELOG.md). The changelog follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) + [SemVer](https://semver.org/spec/v2.0.0.html): unreleased changes live under `## [Unreleased]`, grouped by `### Added / Changed / Fixed / …`, with per-entry `**scope**:` prefixes matching the commit scopes. CI enforces it (`changelog-enforce.yml`). Releases are cut by the release workflow, which versions from conventional commits and regenerates the changelog entry.

## Contributing

1. Read `AGENTS.md` and `docs/AGENT_DECISION_GUIDE.md` first.
2. For source changes, add a `CHANGELOG.md` entry under `[Unreleased]` and use conventional commits.
3. Keep the ponytail contract: delete over add, stdlib before deps, one self-check per non-trivial module. Run `cargo test --workspace` and `cargo clippy` before pushing.
4. Open a PR — `pr-lint.yml` validates the title, `ci.yml` runs lint + tests.

## License

[MIT](LICENSE)
