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

- **Scrape at scale** — `webrain_batch` runs one op across many URLs in parallel tabs; `webrain_crawl` handles spider / sitemap / infinite-scroll / URL-validation, both with auto-throttle and checkpoint-resume.
- **Structured data without hand-written selectors** — `webrain_extract` mode `autoschema` proposes the repeated container, then `schema` / `regex` / `table` / `jsonld` read it. All zero-LLM.
- **Stealth login** — real-Chrome profiles with an encrypted local credential vault (AES-256-GCM + optional TOTP), plus cookie transfer across engines.
- **Get past challenges** — every `webrain_navigate` reports a `challenge` field; protected sites clear with real Chrome + a persistent profile + native login. Native CAPTCHA/vision v2 (drag, `eval_in_frame`, vision ask) — no Python sidecar.
- **See the page** — `webrain_observe` (state / a11y / semantic tree) plus vision tiles (screenshot → vector store) for tables and charts.
- **Structured search** — `webrain_serp` returns typed JSON (`position`/`title`/`url`/`domain`/`snippet`), deduped and paginated, across provider tiers: free HTTP engines → Brave API key → browser render → serpapi.
- **Read anything** — `webrain_pdf` (extract + render), JSON-LD, media, plus `webrain_scrape` for static pages over plain HTTP — 10–100× faster than a browser.

---

## Prerequisites

- **OS**: Windows, macOS, or Linux (x86_64 / arm64).
- **A browser engine** — run `webrain install` once (downloads Chrome for Testing). Obscura and Lightpanda are optional extra engines.
- **Linux**: system libraries for Chrome — see [installation](docs/installation.mdx#linux-dependencies).
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

Then download a browser engine once — `webrain install` — and follow [Run it](#run-it).

### Other install paths

Homebrew, Scoop, `cargo install`, a manual binary download, and a from-source
build are all covered in [docs/installation.mdx](docs/installation.mdx),
together with the Linux system libraries Chrome needs.

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

Every engine, per OS:

| Engine | Windows | macOS | Linux |
|---|---|---|---|
| **Chrome for Testing** (`webrain install`) | ✅ | ✅ | ✅ |
| **Obscura** (`webrain install --engine obscura`) | ✅ x86_64 | ✅ x86_64 / arm64 | ✅ x86_64 / arm64 |
| **Lightpanda** (`webrain lightpanda`, binary needed) | ⚠️ (needs a Windows build on PATH) | ✅ | ✅ |
| **Docker** (`docker/Dockerfile`) | via Docker Desktop | via Docker Desktop | ✅ |

Binary discovery is automatic on every OS: env override → PATH → `~/.lightpanda`, `~/.obscura`, `~/.local/bin` → the webrain engine cache.

## Run it

```bash
webrain install          # 1. one browser engine (Chrome for Testing)
webrain doctor           # 2. verify: engines, CDP ports, vault, MCP
webrain mcp              # 3. MCP over stdio (VS Code, Claude, Cursor, …)
webrain mcp --http 9223  #    …or HTTP on http://127.0.0.1:9223/mcp
```

Register that server in your client (see *Marketplace / IDE Plugins* below), then
just ask: **“scrape every product title and price from <url>”**.

Prefer the CLI? `webrain launch` gives you headed Chrome with a persistent profile,
and every MCP tool has a command twin — see [Commands](#commands).

### Selecting elements

The MCP tools drive elements by **index**, not by CSS selector: `webrain_navigate` /
`webrain_observe` return the interactive elements (`a, button, input, select, textarea,
[role=button]`) and `webrain_interact` takes that index — `{"action": "click", "index": 3}`.
The CLI uses the same 1-indexed refs:

```bash
webrain launch                     # guest-mode Chrome (no profile state)
webrain fetch https://example.com  # prints the page text + interactive elements
webrain click 3                    # click element #3
webrain type 5 "text"              # type into element #5
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
webrain screenshot <url>                           # full-page screenshot (always full page)
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
- **Parallelism differs per engine.** Chrome and obscura open N tabs on one connection, so `concurrency` is the lever. **lightpanda is single-target per *connection*** (its 2nd `Target.createTarget` errors `TargetAlreadyLoaded`), so one endpoint runs URLs serially and the reply says so (`single_target: true, effective_concurrency: 1`); its lever is *connections* — repeat the same endpoint in `cdp_urls` (each connection gets its own worker/page, capped by `--cdp-max-connections`, default 16). Measured on 8 URLs: 1 connection 6.15 s vs 4 connections 1.17 s. Hosted equivalents work the same way: a `wss://` endpoint (e.g. Lightpanda Cloud) is accepted directly.
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
| `webrain_batch` | Same op across many URLs in parallel tabs: `op` = fetch · markdown · extract · interact · eval · screenshot (+ `cdp_urls` per-proxy fan-out) |
| `webrain_crawl` | Site traversal: `mode` = spider (BFS/DFS/best-first + autothrottle + checkpoint) · sitemap · scan · validate |
| `webrain_search` | Web search (duckduckgo · google · bing · brave) |
| `webrain_serp` | Structured JSON search: `position`/`title`/`url`/`domain`/`snippet` across duckduckgo · bing · yahoo · baidu · yandex · google · brave · auto (dedupe · paginate · region-pin · proxy · Brave API key · serpapi) |
| `webrain_pdf` | PDF work: `op` = page · extract (→ markdown) · render (→ vision tiles) · images |
| `webrain_download` | Files/media: `engine` = http (stream) · ytdlp (HLS/playlists) |
| `webrain_watch` | Video → timestamped transcript + frames (no browser; `vision:true` → text captions) |
| `webrain_session` | Browser/auth/session: `op` = open · close · list · cookies · setcookies · save_state · restore_state · profiles · login · close_launch |
| `webrain_vision` | Screenshot-tile vision: `op` = index · retrieve · ask (local Qwen3-VL question answering over a viewport or clip region) |
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

## Environment Variables

The ones you are most likely to set. **Every variable is read once, at process
start or at CDP attach** — change one and restart the MCP server; a *stealth*
change also needs a fresh tab, because injected scripts live as long as the tab.
The complete list, with defaults and the allow/deny rules, is in
[docs/reference/environment.mdx](docs/reference/environment.mdx).

| Variable | Description | Default |
|---|---|---|
| `CDP_URL` | CDP endpoint to attach to | `http://127.0.0.1:9222` |
| `WEBRAIN_HTTP_TOKEN` | Bearer token the HTTP transport requires, compared in constant time. Unset is only allowed on a loopback bind | unset |
| `WEBRAIN_ALLOW_UNAUTHENTICATED` | Permit a **non-loopback** HTTP bind with no token — without it webrain refuses to start that way rather than serving as an open engine | off |
| `WEBRAIN_NO_STATE` | Disable every stateful/session tool (shared-instance mode) and cap tab fan-out at 6 | off |
| `WEBRAIN_STEALTH_NOISE` | Set `0` to drop the canvas/audio fingerprint-noise block. **This is the fix when a page renders but never mounts** — a Next.js/Angular SPA whose grid stays empty while a hand-opened tab of the same URL works | on |
| `WEBRAIN_NO_STEALTH` | Set `1` to skip *all* injected stealth JS + the UA override. A diagnostic for "is our shim breaking this page?" — **not** a workaround for hostile sites | off |
| `WEBRAIN_BROWSERS_DIR` | Engine cache dir (Chrome for Testing, Obscura) | `%LOCALAPPDATA%\webrain\browsers` / `~/.cache/webrain/browsers` |
| `WEBRAIN_CHROME` / `WEBRAIN_LIGHTPANDA` / `WEBRAIN_OBSCURA` | Explicit engine binary paths — `engine not found` means one of these is wrong or nothing is installed | auto-discovered |
| `WEBRAIN_PROFILES_DIR` | Persistent browser-profile root. Profiles are what keeps a login or a solved challenge across calls; pointing this somewhere new looks exactly like being logged out | `%APPDATA%\webrain\profiles` / `~/.local/share/webrain/profiles` |
| `WEBRAIN_PROXIES` | Proxy rotation pool (comma/newline list) for the HTTP engines and the google/brave guest Chrome; a per-call `proxy` overrides it | unset |
| `WEBRAIN_SKILLS_DIR` | Directory of operator `*.md` skills the agent reads through `webrain_guide(topic="skill:<name>")` | unset (off) |
| `BRAVE_API_KEY` | Enables the keyed Brave SERP provider — `--engine brave` then needs no browser | unset |
| `GROQ_API_KEY` / `OPENAI_API_KEY` / `FIREWORKS_API_KEY` | Optional cloud STT and vision; the bundled local stack needs no keys | unset |
| `RUST_LOG` | Log verbosity, e.g. `webrain=info,tungstenite=warn` | — |

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
├── webrain-mcp             MCP server: list_tools / call_tool, 17 intent-based tools (63 legacy via map_surface)
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

## Documentation

| Page | What's in it |
|---|---|
| [Quickstart](docs/quickstart.mdx) | Engine → MCP server → a live scrape in minutes |
| [Installation](docs/installation.mdx) | Every install path, Linux libraries, updating |
| [Agent Decision Guide](docs/AGENT_DECISION_GUIDE.md) | Which engine, which tool, how to handle a challenge |
| [Tools reference](docs/reference/tools.mdx) | The 17-tool surface, per tool and per selector |
| [CLI reference](docs/reference/cli.mdx) | Every subcommand and flag |
| [Environment variables](docs/reference/environment.mdx) | Every knob, with defaults |
| [Architecture](docs/ARCHITECTURE.md) · [ADRs](docs/adr/) | How the layers fit, and why |
| [Browser engines](docs/concepts/browsers.mdx) | Chrome vs Obscura vs Lightpanda |
| [Comparisons](docs/comparisons/) | vs Playwright, Puppeteer, Firecrawl, crawl4ai, Browser Use |
| [Changelog](CHANGELOG.md) | What shipped, per release |

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
- **`changelog-enforce.yml`** — require the changelog to be updated when source changes (PRs and pushes to `main`). Entries follow [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) + [SemVer](https://semver.org/spec/v2.0.0.html), grouped under `[Unreleased]` with per-entry `**scope**:` prefixes that match the commit scopes.

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

## Contributing

1. Read `AGENTS.md` and `docs/AGENT_DECISION_GUIDE.md` first.
2. For source changes, add a `CHANGELOG.md` entry under `[Unreleased]` and use conventional commits.
3. Keep the ponytail contract: delete over add, stdlib before deps, one self-check per non-trivial module. Run `cargo test --workspace` and `cargo clippy` before pushing.
4. Open a PR — `pr-lint.yml` validates the title, `ci.yml` runs lint + tests.

## License

[MIT](LICENSE)
