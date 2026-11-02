# webrain — Architecture

> This is the map: what the pieces are, where they live, and how a call flows.
> For the researched rationale, backend detail and the decision log, read
> [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md); for *how to drive the tools*,
> read [`AGENTS.md`](AGENTS.md) and [`docs/AGENT_DECISION_GUIDE.md`](docs/AGENT_DECISION_GUIDE.md).

webrain is a **portable, LLM-agnostic browser-automation and scraping MCP server**
in Rust. The model decides everything — search, crawl, scrape, navigate, interact —
from the user's prompt. There is **no intent detection in the code**, and **no LLM
in the extraction path**: extraction is JSON / regex / JSON-LD / table / autoschema /
BM25, all zero-LLM, so replies are typed JSON rather than raw HTML.

## Three crates, one binary

```
webrain (bin)  ───────────────►  webrain-mcp  ──────────────►  webrain-core  ───────────►  CDP
CLI subcommands                  stdio + HTTP JSON-RPC          BrowserBackend + engines    Chrome / Obscura / Lightpanda
`webrain mcp|fetch|serp|…`       one dispatch, intent tools      vault, proxy, serp, vision  (all speak CDP)
```

| Crate | Kind | Owns |
|---|---|---|
| `webrain-core` | lib, no binary | `BrowserBackend` trait, `CdpBackend`, engines (spider/tile/extract/batch), vault, proxy pool, SERP providers, local vision/whisper, install + launch, operator skills |
| `webrain-mcp` | lib | stdio JSON-RPC and the HTTP transport; `tools.rs` holds the CDP connection and the entire tool surface behind one `call_tool` dispatcher |
| `webrain-cli` | bin | `match`-based subcommands. Deliberately **no clap** — one binary, no dependency for a handful of subcommands |

## `webrain-core/src`

| File | Responsibility |
|---|---|
| `browser.rs` | `BrowserBackend` trait; `PageState`, `PageResult`, `InteractiveElement` |
| `backends/cdp.rs` | `CdpBackend` — **the only backend**. Chrome, Obscura and Lightpanda all speak CDP, so one client drives all three over a shared WebSocket, with per-tab ops |
| `engines.rs` | `SpiderEngine` (BFS/DFS/best-first + AutoThrottle), `TileEngine` (vision tiles), `BatchResult`, extraction, BM25 |
| `install.rs` | engine and model download/discovery (Chrome for Testing, Obscura, whisper, vision) |
| `launch.rs` | spawn an engine and wait for its CDP endpoint |
| `vault.rs` | encrypted credentials + TOTP |
| `login.rs` | fill a login form over CDP, status-only reply |
| `proxy.rs` | rotation pool (`WEBRAIN_PROXIES`), per-call override |
| `serp.rs` | SERP providers (duckduckgo, bing, google, brave, yahoo, baidu, yandex, auto) |
| `httpcache.rs` | revalidation-only HTTP cache |
| `captcha.rs` | image/grid captcha helpers |
| `video.rs`, `vision.rs` | media frames + local vision model |
| `skills.rs` | operator skills: `BUNDLED` (`include_str!`), `dir`, `list`, `resolve`, `install_bundled`, `install_one`, `remove` |

## The tool surface

`list_tools()` in `webrain-mcp/src/tools.rs` is the source of truth for the surface:
eighteen `webrain_*` entries today, described as 17 intents in the guide text because
`eval` and `eval_in_frame` are one intent with two entry points. `call_tool` executes
them and `map_surface` folds consolidated arguments onto the executor arms. The
agent-facing contract is **intent**, not the ~66-method `CdpBackend` surface — see
[`docs/adr/0002-intent-tool-surface-and-central-dispatch.md`](docs/adr/0002-intent-tool-surface-and-central-dispatch.md).

| Group | Tools |
|---|---|
| Navigate & read | `webrain_navigate`, `webrain_eval`, `webrain_eval_in_frame`, `webrain_observe` (a11y tree via `what=a11y`) |
| Interact | `webrain_interact` |
| Extract (zero-LLM) | `webrain_extract`, `webrain_scrape`, `webrain_pdf`, `webrain_download` |
| Scale out | `webrain_batch`, `webrain_crawl` (whole-site BFS/DFS/best-first via `mode=spider`) |
| Search | `webrain_search`, `webrain_serp` |
| Media & vision | `webrain_vision`, `webrain_watch` |
| State & guidance | `webrain_session`, `webrain_stats`, `webrain_guide` |

A capability folded into a selector is **not** a separate entry: `webrain_a11y`,
`webrain_spider`, `webrain_extract_json`, `webrain_cookies` and `webrain_login` still
exist as `map_surface` routing names, but an agent reaches them as
`observe(what=a11y)`, `crawl(mode=spider)`, `extract(mode=json)`,
`session(op=cookies)` and `session(op=login)`.

`webrain_guide` is also how an operator skill reaches the model: `topic="skills"`
lists what is installed, `topic="skill:<name>"` returns one.

## Per-call flow

```
1. RESOLVE   args → intent (map_surface) → handler
2. CONNECT   CdpBackend: attach to real Chrome / Obscura / Lightpanda (all CDP)
3. ACT       navigate | eval | interact | extract | batch | crawl …
4. REPORT    typed JSON back over MCP — never raw HTML
```

Browser identity is **state**: a profile, a session and a solved challenge are
runtime state, so the answer to a protected page is real Chrome + a persistent
profile + a session (`webrain launch` / `webrain login`) — never "retry anonymously
with a fresh browser".

| Need | Engine |
|---|---|
| Material / interactive SPA (dropdowns, calendars, segmented controls) | **real Chrome** — never obscura/lightpanda |
| Protected, authenticated, Cloudflare/CAPTCHA/Turnstile, real screenshots | **real Chrome + persistent profile + session** |
| Fast scraping of non-challenged JS pages | obscura |
| Minimal footprint, real accessibility tree, no rendering | lightpanda |
| Static HTML, no JS/auth | `fetch_http` (no browser at all) |

Read the `challenge` field on every navigate. A challenge, login or consent page is
**not** successful navigation and must not be extracted as target content.

## Skills

Two trees, one canonical — and they must not be copies of each other:

```
skills/webrain/                    ← the shipped bundle: compiled into the binary
├── SKILL.md                       ←   umbrella: rules, browser choice, extraction matrix
├── credentials/SKILL.md           ← 4 product skills, each with frontmatter `name:`
├── auth-media-vision/SKILL.md
├── web-research/SKILL.md
├── product-market-research/SKILL.md
├── references/*.md                ← chapters the umbrella links to (no `name:`)
├── workflows/*.md                 ← end-to-end procedures (no `name:`)
└── prompts/  evals/               ← supporting material, not skills

.github/skills/                    ← repo-local only: contributor-facing skills
```

`webrain-core/src/skills.rs` compiles `skills/webrain/**` with `include_str!`, and
`webrain install skills` writes that tree to `<engine cache>/skills`, so this is what
users receive. Anything describing the **tool surface** belongs here and nowhere else:
a second copy under `.github/skills/` starts drifting the moment a tool is renamed —
which is exactly what had happened to the `credentials` copy. `.github/skills/` is for
skills that serve *this repository* (changelog upkeep, git cleanup, design work), not
for the product's operators.

## Where the depth lives

| Doc | Contents |
|---|---|
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | researched rationale, backend detail, request flow, roadmap |
| [`docs/adr/`](docs/adr/) | Architecture Decision Records (source of truth) |
| [`AGENTS.md`](AGENTS.md) | hard rules for agents: identity is state, no Python, capability-driven claims |
| [`docs/AGENT_DECISION_GUIDE.md`](docs/AGENT_DECISION_GUIDE.md) | browser / challenge / extraction decision tree |
| [`docs/reference/tools.mdx`](docs/reference/tools.mdx) | the tool reference |
| [`docs/FEATURE_GAPS.md`](docs/FEATURE_GAPS.md) | ranked gap plan and build order |
| [`skills/webrain/SKILL.md`](skills/webrain/SKILL.md) | the shipped operator skill |
