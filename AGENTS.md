# AGENTS.md

Guidance for AI agents working in this repo. Read before making changes or
using the `mcp_webrain-*` tools.

## Webrain = portable MCP scraping tool

`webrain-mcp` exposes browser/scraping tools over MCP. It is meant to be
installed on any system and driven by **any LLM** (Copilot, Claude Code, Codex,
Cursor, …). Follow `docs/AGENT_DECISION_GUIDE.md` — or just call the
`webrain_guide` MCP tool — before browser tasks.

### Browser selection (set `CDP_URL`)
| Need | Browser |
|---|---|
| Cloudflare / CAPTCHA / Turnstile challenges, screenshots, rendering | real Chrome + `scripts/stealth_solve.py` |
| Fast scraping of non-challenged JS pages | obscura (docker, `--stealth`) |
| Lightweight / minimal | lightpanda |
| Static HTML, no JS/auth | `webrain_fetch_http` |

### Challenge rule (read `challenge` on every navigate)
`webrain_navigate` returns `challenge`. If non-null (`cloudflare_challenge` /
`blocked` / `captcha`), the page is gated: run the real-Chrome sidecar
(`python scripts/stealth_solve.py <url> --cdp-port 9222 --headed`) to solve +
login, then re-attach webrain to that CDP port. obscura/lightpanda cannot pass
interactive challenges.

### Extraction
Prefer `webrain_extract_json` (schema discovered via `webrain_autoschema` +
`webrain_eval`), `webrain_batch` for pagination 1..N, `webrain_spider` for
whole sites. Never guess selectors from memory — discover them.

Full detail: `docs/AGENT_DECISION_GUIDE.md`.
