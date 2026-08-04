<div align="center">

<img src="assets/webrain-logo.png" alt="webrain" width="180" height="180">

# webrain

**A portable, LLM-driven browser-automation & web-scraping MCP server — one binary, three engines, any OS.**

Webrain exposes ~45 browser/scraping tools over the **Model Context Protocol**. It is meant to be installed on any system and driven by **any LLM** (GitHub Copilot, Claude, Codex, Cursor, …). The LLM decides everything — search, crawl, scrape, browser-navigate, browser-interact — from a plain-language prompt, with no hardcoded intent detection.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.85+-orange.svg)](Cargo.toml)
![Platforms](https://img.shields.io/badge/platforms-Windows%20%7C%20macOS%20%7C%20Linux-lightgrey)

</div>

---

## Table of Contents

- [webrain](#webrain)
  - [Table of Contents](#table-of-contents)
  - [Key Features](#key-features)
  - [Tech Stack](#tech-stack)
  - [Prerequisites](#prerequisites)
  - [Installation](#installation)
    - [Global Installation (recommended)](#global-installation-recommended)
    - [Project Installation](#project-installation)
    - [Cargo (Rust)](#cargo-rust)
    - [Homebrew (macOS)](#homebrew-macos)
    - [Scoop (Windows)](#scoop-windows)
    - [Linux Dependencies](#linux-dependencies)
    - [From Source](#from-source)
    - [Updating](#updating)
    - [OS Compatibility](#os-compatibility)
  - [Quick Start](#quick-start)
  - [Browser Engines](#browser-engines)
  - [MCP Tools](#mcp-tools)
  - [CLI Reference](#cli-reference)
  - [Environment Variables](#environment-variables)
  - [Marketplace / IDE Plugins](#marketplace--ide-plugins)
  - [Architecture](#architecture)
  - [Agent Decision Guide](#agent-decision-guide)
  - [Testing](#testing)
  - [CI/CD \& Releases](#cicd--releases)
  - [Deployment (Docker)](#deployment-docker)
  - [Troubleshooting](#troubleshooting)
  - [Changelog](#changelog)
  - [Contributing](#contributing)
  - [License](#license)

---

## Key Features

- **Portable MCP server** — a single `webrain` binary. MCP over stdio *or* HTTP (`webrain mcp --http 9223`). No daemon, no Node.js.
- **Three browser engines, one CDP backend** — real **Chrome** (full rendering, Material/SPA, screenshots), **lightpanda** (fast, real a11y tree), **obscura** (stealth, parallel tabs). All speak CDP; `CdpBackend` drives any of them. Plus `fetch_http` for static HTML (10–100× faster than a browser).
- **Engine install like agent-browser** — `webrain install` downloads Chrome for Testing; `webrain install --engine obscura` downloads the latest Obscura release; `webrain lightpanda` / `webrain obscura` spawn the CDP servers.
- **~45 scraping tools** — navigate, snapshot, a11y/semantic tree, autoschema + JSON/regex extraction, batch pagination, spider, sitemap, tables, JSON-LD, PDF extract/render, vision (screenshot tiles → vector store), cookie transfer, stealth login with an encrypted local vault (AES-256-GCM + optional TOTP).
- **Anti-bot aware** — reads the `challenge` field on every navigate; real-Chrome stealth sidecar (`scripts/stealth_solve.py`) for Cloudflare/Turnstile/CAPTCHA.
- **LLM-first decisions** — `webrain_guide` + `docs/AGENT_DECISION_GUIDE.md` encode when to use which engine/tool, so an agent knows exactly what to do.
- **Cross-OS** — Windows, macOS, Linux. Docker image included.

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

- **A browser engine.** Run `webrain install` to download Chrome for Testing (recommended), or point webrain at an existing Chrome/Edge/Chromium. Lightpanda and Obscura are optional engines (see [Browser Engines](#browser-engines)).
- **Rust** — only to build from source (`cargo`). Download from [rustup.rs](https://rustup.rs).
- **An LLM client** — VS Code + Copilot, Claude Desktop, Codex, Cursor, etc. (any MCP client).
- Nothing else is required for the server itself. No Node.js, no Playwright, no daemon.

## Installation

> Install model mirrors [vercel-labs/agent-browser](https://github.com/vercel-labs/agent-browser): a native binary plus an `install` command that downloads engines into a cache dir.

### Global Installation (recommended)

One command per OS — downloads the release binary and puts `webrain` on PATH, then install a browser engine:

**Windows (PowerShell):**

```powershell
$dir = "$env:LOCALAPPDATA\Programs\webrain"; New-Item -ItemType Directory -Force -Path $dir | Out-Null
Invoke-WebRequest "https://github.com/prokopis3/webrain/releases/download/v0.1.0/webrain-windows.exe" -OutFile "$dir\webrain.exe"
[Environment]::SetEnvironmentVariable("Path", "$([Environment]::GetEnvironmentVariable('Path','User'));$dir", "User")
# open a new terminal, then:
webrain install          # Download Chrome for Testing (first time only)
webrain mcp --http 9223  # start the MCP server
```

**Linux / macOS:**

```bash
curl -L -o ~/.local/bin/webrain https://github.com/prokopis3/webrain/releases/download/v0.1.0/webrain-linux
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

Coming soon — use the macOS curl install above for now.

### Scoop (Windows)

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

```bash
# If installed via cargo:
cargo install --git https://github.com/prokopis3/webrain webrain-cli --force

# Re-download engines after an update (cache dir stays, versions are additive):
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
| **Docker** (`Dockerfile`) | via Docker Desktop | via Docker Desktop | ✅ |

Binary discovery is automatic on every OS: env override → PATH → `~/.lightpanda`, `~/.obscura`, `~/.local/bin` → the webrain engine cache.

## Quick Start

```bash
# 1. Get a browser engine
webrain install

# 2. Start the MCP server (stdio for VS Code/Copilot, or HTTP)
webrain mcp                        # stdio
webrain mcp --http 9223            # HTTP transport on 127.0.0.1:9223

# 3. Point an LLM at it. In VS Code, add to settings.json "mcp" (see Marketplace):
#    {"servers": {"webrain": {"command": "webrain", "args": ["mcp"]}}}

# 4. Ask the LLM to do something, e.g. "scrape all product titles + prices from URL X"
```

Prefer to drive it by hand? Every engine + tool has a CLI twin:

```bash
webrain launch scrapingcourse demo "https://example.com/login" --port 9222   # headed Chrome + login
webrain doctor                 # full diagnosis: engines, MCP, CDP, vault, sidecar
webrain fetch <url>             # attach to CDP_URL and fetch
webrain screenshot <url>
webrain eval "document.title"
```

## Browser Engines

| Need | Engine | How to get it |
|---|---|---|
| Material / interactive SPA (Google Flights, calendars, dropdowns), real screenshots, Cloudflare/Turnstile | **real Chrome** | `webrain install` |
| Fast scraping of non-challenged JS pages, parallel tabs | **obscura** (stealth) | `webrain install --engine obscura [--stealth]`, then `webrain obscura` |
| Fastest/lightest, real a11y + semantic tree, no rendering | **lightpanda** | install the binary, then `webrain lightpanda` |
| Static HTML, no JS/auth | **fetch_http** (no browser) | built-in |

**Key rules (see [Agent Decision Guide](#agent-decision-guide)):**

- **Never** use obscura/lightpanda for Material/SPA interaction or screenshots — they have **no layout/paint engine**. Obscura errors loudly on screenshots; lightpanda returns a *fake placeholder PNG*. Route interactive SPAs to real Chrome via `cdp_urls:["http://127.0.0.1:9222"]`.
- Read the `challenge` field after every `webrain_navigate`. If it's non-null, the page is gated — use the real-Chrome stealth sidecar.
- Extract from container/card-level DOM, not bare `$` text nodes (Google Flights renders a spurious price grid).

## MCP Tools

All tools are discovered dynamically (`webrain_guide` lists them for the LLM). Highlights:

| Category | Tools |
|---|---|
| **Navigate / observe** | `webrain_navigate`, `webrain_snapshot`, `webrain_a11y`, `webrain_semantic_tree`, `webrain_get_html`, `webrain_console` |
| **Interact** | `webrain_click`, `webrain_type`, `webrain_press`, `webrain_scroll`, `webrain_nav`, `webrain_tab`, `webrain_dismiss_overlays` |
| **Extract** | `webrain_autoschema`, `webrain_extract_json`, `webrain_extract_regex`, `webrain_table`, `webrain_get_jsonld`, `webrain_pdf_extract`, `webrain_pdf_images` |
| **Crawl** | `webrain_batch`, `webrain_spider` (AutoThrottle + checkpoint/resume), `webrain_sitemap`, `webrain_scan` |
| **Vision** | `webrain_pixel`, `webrain_vision_index`, `webrain_vision_retrieve`, `webrain_screenshot` |
| **Auth / state** | `webrain_login`, `webrain_profiles`, `webrain_cookies`, `webrain_setcookies`, `webrain_open_session`, `webrain_close_session` |
| **Utility** | `webrain_fetch_http`, `webrain_download`, `webrain_search`, `webrain_validate_urls`, `webrain_clean`, `webrain_media`, `webrain_get_images`, `webrain_guide`, `webrain_eval` |

Full per-tool reference: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md). Tool/browser/challenge decisions: [`docs/AGENT_DECISION_GUIDE.md`](docs/AGENT_DECISION_GUIDE.md).

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
| `webrain doctor` | Full install diagnosis — version, MCP server, CDP ports (9222/9224/9225), engine discovery, vault, Python sidecar, `recommend`. `--doctor` alias |

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
| `RUST_LOG` | Log verbosity (`webrain=info,tungstenite=warn`) | as above |

## Marketplace / IDE Plugins

Webrain is an **MCP server**, so it plugs into any MCP-capable IDE. There is no separate extension to publish — you register the server, and the ~45 tools appear.

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
├── webrain-mcp             MCP server: list_tools / call_tool, ~45 tool schemas
└── webrain-cli             subcommand dispatch (mcp | install | launch | login | …)
```

**How it works:**

1. `CdpBackend` connects to a CDP endpoint (Chrome, lightpanda, or obscura — all speak CDP) over a raw WebSocket.
2. On attach it applies stealth hardening (UA override, `Emulation.setAutomationOverride`, JS patches) — so it can log into real sites.
3. The MCP layer exposes every action as a tool. An LLM picks tools by intent; `webrain_guide` + `AGENT_DECISION_GUIDE.md` encode the *which-browser / which-tool* decisions so the LLM never guesses.
4. Extraction is generic — autoschema probes the DOM, JSON/regex/table extractors read container-level structure, spider/batch/sitemap crawl at scale, vision tiles give the model "eyes" for tables/charts.

**Data flow (LLM → browser):**

```
LLM prompt → tool call (webrain_navigate/extract/batch/…) → CdpBackend → CDP → engine (Chrome/lightpanda/obscura)
        ← JSON result (PageState, extracted rows, stats) ←
```

## Agent Decision Guide

**Read [`docs/AGENT_DECISION_GUIDE.md`](docs/AGENT_DECISION_GUIDE.md) before browser tasks.** It encodes, with live-verified results:

- **Browser selection** — Material/SPA → real Chrome via `cdp_urls`; Cloudflare/Turnstile → Chrome + stealth sidecar; fast JS scraping → obscura; lightpanda for real a11y with minimal footprint; static → `fetch_http`.
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

- **`ci.yml`** — lint (ruff), tests, formatting on every push/PR to `main`.
- **`release.yml`** — on merge to `main`: bump SemVer from conventional commits, update `CHANGELOG.md`, create a GitHub Release.
- **`pr-lint.yml`** — enforce conventional-commit PR titles (`<type>(<scope>): <description>`).
- **`changelog-enforce.yml`** — require the changelog to be updated when source changes.

Commit convention: `<type>(<scope>): <description>` — types `feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert`, scopes `core|api|cli|handlers|tools|plugin|memory|cache|pipeline|extraction|login|navigation|antibot|llm|token|browser|serp|search|config|deps|docs|tests|ci`.

## Deployment (Docker)

A self-contained image with Chromium bundled:

```bash
# build
docker build -t webrain .

# run (HTTP MCP on 9223)
docker run -p 9223:9223 webrain mcp --http 9223

# multi-arch
docker buildx build --platform=linux/amd64,linux/arm64 -t ghcr.io/prokopis3/webrain .
```

## Troubleshooting

**`failed to spawn Chrome`** → Chrome isn't installed or `WEBRAIN_CHROME` is wrong. Run `webrain install` to download Chrome for Testing.

**`port X already has a CDP endpoint`** → a browser is already running there. Stop it, or pick another port (`--port N`).

**Screenshots fail / blank on obscura or lightpanda** → they have no paint engine. Obscura errors loudly; lightpanda returns a *fake placeholder PNG*. Use real Chrome.

**Empty a11y tree** → the page likely never rendered (consent/challenge page) or the control needs interaction. Check `webrain_navigate`'s `challenge`; try real Chrome; drop the `role` filter and use `filter` on the label.

**`lightpanda not found` / `obscura not found`** → install the binary (see [Browser Engines](#browser-engines)) or set `WEBRAIN_LIGHTPANDA` / `WEBRAIN_OBSCURA`.

**`webrain doctor` shows MCP down** → start `webrain mcp --http 9223`; if CDP ports are down, `webrain install` then start an engine.

**Cloudflare/Turnstile blocks the scrape** → read the `challenge` field and use the real-Chrome stealth sidecar (`python scripts/stealth_solve.py <url> --cdp-port 9222 --headed`), then re-attach webrain to that CDP port.

## Changelog

See [`CHANGELOG.md`](CHANGELOG.md). The changelog follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) + [SemVer](https://semver.org/spec/v2.0.0.html): unreleased changes live under `## [Unreleased]`, grouped by `### Added / Changed / Fixed / …`, with per-entry `**scope**:` prefixes matching the commit scopes. CI enforces it (`changelog-enforce.yml`). Releases are cut by the release workflow, which versions from conventional commits and regenerates the changelog entry.

## Contributing

1. Read `AGENTS.md` and `docs/AGENT_DECISION_GUIDE.md` first.
2. For source changes, add a `CHANGELOG.md` entry under `[Unreleased]` and use conventional commits.
3. Keep the ponytail contract: delete over add, stdlib before deps, one self-check per non-trivial module. Run `cargo test --workspace` and `cargo clippy` before pushing.
4. Open a PR — `pr-lint.yml` validates the title, `ci.yml` runs lint + tests.

## License

[MIT](LICENSE)
