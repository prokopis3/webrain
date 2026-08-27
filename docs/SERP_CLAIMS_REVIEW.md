# SERP claims review — is the `webrain_serp` demo real?

Date: 2026-06 (live-verified against the actual binary)

## Question

The marketing demo claims a `webrain_serp` tool that:

> `engine=auto · limit=5` → "duckduckgo + bing + google in parallel" → "merged + deduped
> · 5 unique results · 2.1s · request_id serp-…" and shows clean tokio/rust results
> (rust-lang.org, Wikipedia, doc.rust-lang.org) with `{position, title, url, domain, snippet}`.
> Bottom line: "5 engines behind one tool · 3 engines no browser at all · 50 max results per call".

## Verdict: the machinery is real, the example output is not what `auto` produces

The tool **exists and works** (MCP tool `webrain_serp` + CLI `webrain serp`). Every
structural claim checks out against the code and a live run. But the **specific demo
output shown for `engine=auto` is misleading** — in a real run `auto` returns garbage,
because Bing (first in the merge order) returns unrelated junk on flagged IPs and no
relevance check exists to stop it. DuckDuckGo alone produces exactly the results the
demo shows; `auto` does not.

---

## 1. What is TRUE (verified in code + live runs)

| Claim | Status | Evidence |
|---|---|---|
| `webrain_serp` tool exists | ✅ | MCP tool registered `webrain-mcp/src/tools.rs:395`; CLI `webrain serp` `webrain-cli/src/main.rs:191` |
| 5 engines behind one tool | ✅ | `duckduckgo | bing | google | brave | auto` — tool schema `tools.rs:399`, core `webrain-core/src/serp.rs:98` |
| 50 max results per call | ✅ | `limit` clamped `1..=50` in MCP (`tools.rs:2371`) and CLI (`main.rs:217`); `http_search` caps pages at 5×10 (`serp.rs:624`) |
| `request_id serp-…` | ✅ | `format!("serp-{millis}-{seq}")` `serp.rs:578-586`; live: `serp-1786801552431-0` |
| `ms` timing in reply | ✅ | `SerpResponse.ms` `serp.rs:1144`; live: 2810ms (demo says 2.1s — same ballpark) |
| parallel fetch | ✅ | `auto` fires all engines via `join_all` `serp.rs:1110-1111` |
| merged + deduped | ✅ | `dedupe_cap` on URL `serp.rs:560-575`; positions renumbered 1-based |
| result JSON shape | ✅ | `{position,title,url,domain,snippet}` `serp.rs:29-38`; live output matches |
| ddg returns the demo's class of results | ✅ | Live `engine=duckduckgo "tokio rust"` → tokio.rs, github tokio-rs, docs.rs in 869ms |
| `engine=brave` per-query | ✅ | second demo line exists; routes to attached CDP engine (`serp.rs:1122-1130`) |

## 2. What is FALSE or EXAGGERATED

### 2.1 `auto` does NOT merge "duckduckgo + bing + google" — and the shown results are not what `auto` returns

- `auto` fires **all four** HTTP engines (`bing, duckduckgo, brave, google` — `serp.rs:98,1110`) over **plain HTTP**.
- **google over HTTP is a JS-gated consent shell → ~zero results.** Google only returns real
  results through the browser path (`browser_search`), which **`auto` never calls**
  (`serp.rs:1107-1121` — no backend, no browser). The `AGENT_DECISION_GUIDE.md:103` claim that
  "google joins via the browser path" for `auto` is **not implemented**.
- **brave over HTTP is a JS SPA shell → ~zero results.**
- So `auto` is effectively **bing + ddg over HTTP**, in merge order **bing first**.
- **Live proof** (this machine): `engine=duckduckgo "tokio rust"` → perfect tokio results
  (869ms). `engine=bing "tokio rust"` → **identity-theft articles**. `engine=auto "tokio rust"`
  → **"The Fire Factory" fireplace reviews** (2810ms). `auto` run #2 → Hotmail/Exchange news.
  The demo's 5 tokio results only appear with `engine=duckduckgo`.

Root cause: no **relevance gate**. Bing serves unrelated junk (flagged/GeoIP'd IP, no-JS
fallback page), `parse_bing` accepts any `li.b_algo` (`serp.rs:237-289`), and the merge
keeps whatever comes first (`dedupe_cap` stops at `limit`). Nothing checks that a result
actually matches the query, so garbage-in → garbage-out, and it beats the good engine.

### 2.2 "3 engines, no browser at all" is misleading

- TRUE for **duckduckgo and bing** (pure HTTP) — and `auto` itself (HTTP-only path).
- NOT true for **google and brave**: without a browser they return zero useful results
  (google = consent/JS wall; brave = SPA shell). The tool's own description says so
  (`tools.rs:399`: "google|brave are JS-gated… guest-browser flow"). So really:
  **2 engines + auto work browserless; 2 engines need a browser.**

### 2.3 "2.1s · 5 unique" is plausible but unstable

Live auto runs: 2810ms, 3666ms, 4069ms. `ms` depends on network/IP state. Fine as a demo
number, not a guarantee.

## 3. Optimal improvement plan

### P0 — make `auto` trustworthy (fixes the demo bug)

1. **Relevance gate (hard correctness fix).** In `webrain-core/src/serp.rs`, before a result
   is accepted (per-engine in `http_search` and/or at merge in the `auto` arm), drop results
   whose `title + domain` share **zero significant tokens** (len ≥ 3, stopwords removed) with
   the query. "The Fire Factory" vs "tokio rust" → 0 overlap → dropped. This single check
   turns the demo from garbage to correct regardless of engine order.
2. **Re-order the merge by engine quality.** Merge order `duckduckgo → bing` (DDG is the
   reliable HTML engine; bing is GeoIP-fragile). Keep `HTTP_ENGINES` for fallback but define
   a separate `AUTO_ORDER`.
3. **Stop HTTP-fetching google/brave in `auto`.** Split `HTTP_ENGINES` (`bing, duckduckgo`)
   from browser engines (`google, brave`). In `auto`, HTTP-fetch only `bing, ddg`; if a
   backend is attached, additionally `browser_search` google/brave **concurrently** and merge
   those in — this makes the documented "google joins via the browser path" true.
4. **Per-page sanity check** in `http_search_page` (`serp.rs:590`): if the fetched page has no
   results containers **and** shares no query tokens, treat as empty → contributes nothing,
   report in `skipped`.

### P1 — capability-driven docs (AGENTS.md rule 5)

- `docs/AGENT_DECISION_GUIDE.md:103` — reword "google joins via the browser path" for `auto`
  (only true after P0.3) or implement P0.3.
- `docs/index.mdx` + `docs/styles/landing-anim.js:640` — "duckduckgo, bing, google fetched in
  parallel" and the shown 5-result set are DDG results; label the demo as `engine=duckduckgo`
  or re-run it after P0. `webrain serp "tokio rust" --engine duckduckgo` reproduces the demo
  today.
- `webrain-cli/src/main.rs:194` — comment "duckduckgo/bing/google/auto need no browser" is
  wrong (google needs a browser for real results); fix the comment.

### P2 — observability

- Add `per_engine: [{engine, count, status}]` to `SerpResponse` for `auto`, so callers can
  see bing contributed 5 junk / ddg 0, instead of a flat `skipped[]`. Touch `serp.rs:1139`
  + `tools.rs:2401`.

### P3 — tests

- Unit test for the relevance gate (junk vs query token overlap).
- Live integration test (gated behind env var / `#[ignore]`): `auto "tokio rust"` must return
  ≥1 result from `tokio.rs|github.com|docs.rs`. This test fails today — it would have caught
  the demo discrepancy.

### P4 (optional, only if the "ebrain · serp · live" TUI is the product vision)

- Add a terminal UI mode to `webrain serp` (per-engine live status lines, merged list).
  Not needed to make the claims true.

## 4. One-line summary

`webrain_serp` is real: 5 engines, parallel fetch, merge+dedupe, `request_id`, `1..=50`
limit, exact JSON shape — all verified. The demo's specific `auto` output is **not**
reproducible today because `auto` merges unvalidated Bing junk ahead of DuckDuckGo's good
results; a query-relevance gate + engine re-ordering (P0) fixes it in a few hours.

---

## 5. Implementation status (P0–P2 applied)

- **P0.1 relevance gate** — `filter_relevant` in `webrain-core/src/serp.rs`: a result
  survives only when its title/domain/URL shares a significant query token; applied
  per-page in `http_search` and `browser_search` (junk pages stop pagination early) and
  again at the `auto` merge. Unit-tested (`relevance_gate_*`, `query_tokens_*`).
- **P0.2 merge order** — `auto` now merges `AUTO_HTTP_ENGINES = [duckduckgo, bing]`
  (ddg first; bing's GeoIP junk can no longer dominate the top of the list).
- **P0.3 engine split** — `auto` HTTP-fetches only ddg+bing; google/brave are
  `AUTO_BROWSER_ENGINES`, joined concurrently via `browser_search` (20s cap per engine)
  **only when a backend is attached** (CLI probes `connect_default().ok()`, MCP passes an
  attached backend through; `WEBRAIN_NO_STEALTH`/`set_no_stealth` honored for auto).
  Without a browser they're reported `skipped`, never HTTP-polled pointlessly.
- **P0.4 page sanity** — folded into P0.1 (an all-junk page parses to 0 after the gate →
  `fresh == 0` → stop → fallback chain / skipped).
- **P2 per_engine breakdown** — new `SerpEngineReport` + `SerpResponse.per_engine`
  (`status: ok|empty|skipped`, `count`), serialized in the MCP envelope and printed by the
  CLI (`engines: duckduckgo=ok(5), bing=skipped(0), ...`). `specific_engine` /
  `fallback_chain` now return the winning engine so single-engine replies report the true
  source after a fallback.
- **P1 docs/comments** — `AGENT_DECISION_GUIDE.md`, `docs/index.mdx`, CLI usage/comment,
  MCP tool description + AGENT_GUIDE updated to "duckduckgo|bing|auto pure HTTP;
  google|brave browser-rendered; auto = ddg+bing (+google/brave when attached),
  relevance-filtered".

Verify: `cargo test -p webrain-core serp` (pure unit tests) and
`webrain serp "tokio rust" --engine auto --limit 5` (should now return tokio/rust
results, not fireplaces).
