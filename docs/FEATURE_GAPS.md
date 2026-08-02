# webrain → Next-Gen Stealth Spider / Web-Brain: Cross-Reference Gap Plan

> Researched against 11 reference projects, `2026-08-01`.
> **Refs:** daijro/camoufox · h4ckf0r0day/obscura · jo-inc/camofox-browser ·
> unclecode/crawl4ai · lightpanda-io/browser · StarTrail-org/PixelRAG ·
> browser-use/browser-use · browser-use/browser-harness · alibaba/page-agent ·
> NousResearch/hermes-agent · Panniantong/Agent-Reach
>
> **Method:** steal only features with clear native power. YAGNI the rest.
> Correctness/security/perf out of scope (separate review passes).

## Decision: stay Rust

| Axis | Python (browser-use/hermes/crawl4ai) | webrain (Rust) |
|---|---|---|
| Network stealth | Can't touch TLS handshake — Playwright/requests ride a distinct TLS stack, a permanent bot tell | `rustls`/BoringSSL custom ClientHello → JA3/JA4 impersonation in-config (obscura IS Rust: proof) |
| Runtime | Interpreter + GIL per step; 100+ MB | single ~15 MB binary, no GIL, zero deps |
| Per-step overhead | DOM serialize + AX + vision under GIL | in-process, no serialization boundary |
| Proof of concept | — | Agent-Reach ships a **Rust headless-browser MCP** backend — the approach is proven |
| The one Python edge | LLM brain + embeddings | That layer lives in the **MCP client** (Copilot), not webrain. Don't pay for it here. |

Python wins only for the agent-brain/embeddings layer — which is client-side.
Keep webrain a lean native tool server. **Rust.**

## Ranked steal-list (biggest power first)

### T1 — Network-level stealth (the thing JS overrides can't fake)
| # | Steal | From | Why it's powerful | Effort |
|---|---|---|---|---|
| 1 | **TLS/HTTP2 ClientHello impersonation** (JA3/JA4, Chrome 145 cipher order/ALPN/extensions, consistent not random) | obscura | JS-only stealth dies on TLS-fingerprint checks (AWS WAF, Cloudflare TLS exams). Rust-native = rustls config. The #1 gap. | High (~250 ln + rustls dep) |
| 2 | **GREASE-correct `sec-ch-ua` / `sec-ch-ua-platform`** derived from UA (Chromium per-major GREASE algorithm) so HTTP layer matches `navigator.userAgentData` | obscura | Removes a cheap, common header-vs-JS tell. | Small (~30 ln) |
| 3 | **Consistent profile pool** — platform/UA/userAgentData/WebGL renderer (ANGLE vs Metal) internally aligned; pin or rotate per context | obscura | One internally-consistent identity beats random per-surface spoofing. | Small (~40 ln) |
| 4 | **Canvas + audio fingerprint noise** (per-context seed → `toDataURL`/`getImageData`/`AnalyserNode` variance) | camoufox (C++) → webrain does it in STEALTH_JS | Unique canvas/audio hash per context. JS approximation is weaker than C++ but near-free. | Small (~40 ln) |
| 5 | **Tracker blocklist** (3.5k-domain analytics/ads/fp scripts aborted before load) | obscura | Kills fingerprinting surfaces AND speeds pages. | Small (~30 ln) |
| 6 | **Identity alignment**: process TZ + locale + geolocation match proxy exit region; `Accept-Language` consistent | obscura / camofox-browser | Rotated identity with mismatched TZ/region is itself a bot signal. | Medium (~80 ln) |

### T2 — Crawl intelligence (crawl4ai)
| # | Steal | Why | Effort |
|---|---|---|---|
| 7 | **URL Seeder / DomainMapper** — discover URLs *before* crawling: sitemap (+index, parallel sub-sitemaps) + Common Crawl CDX + Wayback + crt.sh + RSS; BM25 pre-rank; live-check | Stealth win (never touches the target site to find pages) + efficiency. Pure `ureq` HTTP — no browser. | Medium (~200 ln) |
| 8 | **HTTP cache with 304 revalidation** — ETag/Last-Modified/head-fingerprint; `cache: enabled/disabled/read_only/write_only/bypass`; status `hit/validated/miss` | Biggest spider token-cost win (re-crawls cost the same prompt tokens) + speed. | Medium (~150 ln) |
| 9 | **Rate limiter + sticky proxy** — exp-backoff on 429/503 (`base/max_delay/max_retries`); pin one IP to a deep-crawl session (`get_proxy_for_session(ttl)`) | Survives rate-limit walls; deep crawls keep a consistent exit IP. | Medium (~120 ln) |
| 10 | **Hooks lifecycle** — `on_browser_created`, `on_page_context_created` (cookies/localStorage/route-block), `before_goto` (headers), `after_goto` (wait-selector), `on_user_agent_updated`, `before_retrieve_html` (scroll) | The single biggest unlock for auth-gated scraping (login → persist storage_state). | Medium (~150 ln) |
| 11 | **Memory-adaptive dispatcher** — upgrade `webrain_batch` from fixed `concurrency=N` to memory-threshold throttle + requeue + starved-URL fairness | Fixed concurrency OOMs on big jobs; adaptive keeps tabs alive. | Medium (~120 ln) |
| 12 | **Filter-chain + composite scorers** — `Domain/URLPattern/ContentType/SEO` filters + `KeywordRelevance/PathDepth/Freshness` scores + `score_threshold`; resume/crash recovery state | Turns BFS/DFS/BestFirst into a tunable deep-crawl pipeline with resumability. | Medium (~180 ln) |
| 13 | **Fit-prune output** — text/link-density DOM prune (drop nav/footer/ads/scripts) → token-cheap `fit_text` for the LLM | Direct prompt-token savings; complements `semantic_tree`. | Small (~50 ln) |
| 14 | **Schema-validation loop** — `webrain_autoschema` gains: run a candidate CSS schema against live HTML → return match-score/refine hints | The stealable primitive of crawl4ai's self-validating LLM schema (the LLM loop stays client-side). | Small (~60 ln) |

### T3 — Web-brain primitives
| # | Steal | Why | Effort |
|---|---|---|---|
| 15 | **LP.* CDP extensions**: `getInteractiveElements`, `detectForms`, `getContentSignal` | webrain has `semantic_tree`+`a11y`; these are the cheap agent-facing views lightpanda ships. | Small (~90 ln) |
| 16 | **Vision embedding retrieval** — embed tiles (Qwen3-VL-2B / ONNX / GGML) → cosine vector index + multi-modal query + tile-geometry filter (`min_tile_height`/aspect) + context-aware `max_tiles` budget | The real "web-brain" vision search. Heaviest dep — behind an optional flag. | High (~300 ln + model) |

### Not webrain's job (client owns it — Copilot is the brain)
`AgentOutput {thinking/evaluation_previous_goal/memory/next_goal}` + inline plan state machine
+ loop/stagnation detection (action hashing) + pluggable memory (browser-use, page-agent,
hermes) — all **client-side loop logic**. Document the contract, don't build it in.
`agent_helpers.py` self-extension (browser-harness) is a Python-client pattern.

## Skip-list (ponytail YAGNI)

| Idea | From | Why skip |
|---|---|---|
| C++-level engine patching (full Firefox fork) | camoufox | A platform change, not a feature. Upgrade path if ever needed: swap engine to camoufox/libtreefox. |
| Headful Xvfb + Mesa llvmpipe real-WebGL | camofox-browser | Container-deploy concern; CDP can't add real GPU anyway. |
| LLMExtractionStrategy / LLMConfig chunking | crawl4ai | The agent IS the LLM — writes schemas, webrain runs them zero-LLM. |
| Cosine text embedding (schema/LLM chunk) | crawl4ai | Embeddings live in the vision layer (T3-16), not text. |
| Markdown generation | lightpanda | Deliberate: `innerText` + vision tiles. Not a gap. |
| Streaming results over MCP | crawl4ai | MCP req/resp; client batches. Revisit only if a client needs it. |
| xCAPTCHA solving | camofox-browser | Stealth minimizes, never solves. Out of lane. |
| Pluggable memory providers / MoA / holographic reasoning | hermes | Client-side brain. |
| Multi-backend router + SKILL.md intent routing | Agent-Reach | MCP already does discovery; Copilot owns routing. |
| Content-signal/selector caching internals | lightpanda | Micro-optimization; add when profiling says so. |

## Build order

```
P1  ✅ DONE — canvas/audio noise in STEALTH_JS · tracker blocklist
    (`Network.setBlockedURLs`, 28 patterns, one call in shared attach_and_init).
    Skipped: profile rotation + identity-alignment hooks (config nobody sets),
    JS-level GREASE (an HTTP-header concern → handled in P2 layer).
P2  ◐ P2-lite DONE — Chrome-identical HTTP headers on the no-browser fast path
    (`webrain_fetch_http`/`download`/`validate_urls`): Chrome 145 UA, GREASE
    sec-ch-ua, sec-ch-ua-platform, sec-fetch-*. Verified live vs httpbingo echo
    (Chrome/145.0.0.0, no ureq tell).
    REMAINING: TLS/HTTP2 JA3/JA4 ClientHello byte parity — rustls can't emit
    Chrome's extensions/GREASE; needs a BoringSSL fork (obscura/wreq).
    Deferred; add only when a WAF starts failing past the HTTP layer.
P3  Crawl intel (~470 ln, engines.rs + tools.rs): URL seeder · 304 cache ·
    rate limiter + sticky proxy · hooks lifecycle · memory-adaptive batch ·
    filter-chain scorers + resume
P4  Brain primitives (~140 ln): interactive-elements/forms/content-signal ·
    fit-prune · schema-validate loop · vision embeddings (optional flag)
                                        ──────────────────────────
                                        ~1,030 ln total, 1 new dep (rustls)
```

## Architecture impact

Everything stays in `webrain-core/src/backends/cdp.rs` (STEALTH_JS, network
layer), `webrain-core/src/engines.rs` (seeder, cache, rate-limit, scorers),
`webrain-mcp/src/tools.rs` (new args/tools). One new optional crate (rustls)
for the TLS-impersonation client. `BrowserBackend` trait untouched except a
`stealth_http` method. Vision embeddings gated behind a feature flag.

→ skipped: engine fork, LLM extraction, markdown, streaming, captcha, client-brain,
memory providers; add only if a concrete client asks for it.
