# Memory & Performance Audit — webrain

**Date:** 2026-10-23
**Scope:** whole repo, focused on memory use and speed. No feature changes proposed.
**Method:** code graph only (`query_graph`, `search_graph`, `search_code`,
`get_code_snippet`, `get_file_outline`, `check_index_coverage`) against
`D-Windows-Documents-Programming-Projects-Rust-webrain` (2566 nodes / 6182 edges).
Every finding below was re-read at the cited lines after the first pass.

**Assumed load:** one long-lived process per user (an editor or agent holds the MCP
server open) plus one attached Chrome; `webrain_batch` fanning out 4–8 tabs on that
one browser; optionally a shared `--http` instance serving several clients.

---

## Must fix

### 1. Every browser command blocks every other tab
`webrain-core/src/backends/cdp.rs:585-650` (`send_cmd_with`), setup at `:358-427` (`connect`)

- **What this is:** One WebSocket per browser. `send_cmd_with` takes the lock on the
  shared connection, writes the command, and keeps the lock while waiting for that
  command's reply — 30 s normally, 120 s for screenshot/PDF.
- **Problem:** `connect` spawns two tasks, but they only copy WebSocket frames into a
  channel; matching a reply to its command happens *inside* the lock. So
  `webrain_batch(concurrency: 8)` issues its 8 tabs' commands one at a time. One slow
  `Page.captureScreenshot`, one `awaitPromise` eval, or one paused renderer makes the
  other 7 tabs wait. A single 120 s PDF freezes the whole browser.
- **Fix:** one dispatcher task reads the channel and resolves
  `HashMap<u64, oneshot::Sender<Value>>`; `send_cmd_with` holds the lock only to write.
  About 60 lines in this file, no API change.
- **If we skip it:** batch throughput equals the slowest page, and one stuck command
  hangs every tab.

### 2. Every reply is tokenized, including multi-megabyte images
`webrain-mcp/src/lib.rs:1055-1078` (`with_token_cost`)

- **What this is:** Exact BPE (byte-pair encoding — the tokenizer a language model
  uses) runs over the whole reply just to report `tokens.est_tokens`.
- **Problem:** screenshot, vision and PDF replies carry megabytes of base64. cl100k
  emits one `u32` per ~4 characters, so a 4 MB reply produces ~1M tokens: hundreds of
  ms of CPU plus ~4 MB of transient memory, for a number nobody acts on. The parsed
  vocabulary also sits in RAM (a `OnceLock`) for the life of the process.
- **Fix:** above a small cap use the estimate the field name already promises:
  `if text.len() > 32*1024 { chars/4 } else { exact }`. One branch. That also makes
  `tiktoken-rs` removable.
- **If we skip it:** the largest replies pay the most CPU and memory for a cosmetic
  metric.

## Should fix

### 3. Network-capture buffer has no cap and is copied
`webrain-core/src/backends/cdp.rs:632` (push), `:1333-1390` (`capture_media`, clone)

- **What this is:** While capture is on, every `Network.requestWillBeSent` URL is
  pushed into `net_urls` with no length guard. `capture_media` then clones the whole
  vector and leaves it in the field.
- **Problem:** a video page requests thousands of segments. `webrain_media(wait_ms:
  60000)` can buffer tens of thousands of strings, clone them (double peak), and hold
  them until the next capture — then report only 100.
- **Fix:** cap the push (`if v.len() < 2048`) and use
  `std::mem::take(&mut *self.net_urls.lock().await)` instead of `.clone()`.
- **If we skip it:** long captures on streaming pages are where this process spikes
  memory.

### 4. Vision store reloads and rewrites whole, and re-measures every vector per query
`webrain-core/src/vision.rs:550-631` (`index_current_page`), `:255-265` (`search`)

- **What this is:** `index_current_page` loads the entire index for its tag, adds the
  new tiles, and saves it all back. `search` computes each stored vector's norm (its
  length — one square root) on every query. `add` (`:199-206`) stores the vector
  unchanged — verified, it does not normalize.
- **Problem:** the store is cumulative, so call #100 loads and rewrites 100 pages of
  vectors. A vector never changes after `add`, yet every query recomputes its norm:
  17k tiles means 17k square roots per search.
- **Fix:** normalize inside `add` so stored vectors are unit length, then `search` is a
  plain dot product.
- **If we skip it:** index and retrieve get slower with every page ever indexed.

### 5. Tile images are cloned two or three times
`webrain-core/src/vision.rs:566-601` (`index_current_page`)

- **What this is:** `inputs` clones every tile's base64 PNG, then the
  describe/caption branch collects `t.png_b64.clone()` for the same tiles again.
- **Problem:** a 20-tile page of ~300 KB tiles is 6 MB held as ~18 MB across three
  copies. This is the tool's largest single memory spike.
- **Fix:** build the PNG `Vec<String>` once and pass slices to embed and to
  describe/caption; keep only the single clone `EmbedInput::Image` requires.
- **If we skip it:** vision indexing stays the easiest way to exhaust memory.

### 6. Three functions do the work of twenty
`webrain-mcp/src/tools.rs:926-2322` (`call_tool`, 1396 lines),
`webrain-mcp/src/lib.rs:154-985` (`handle_rpc`, 831), `:1255-1525` (`handle_http_conn`, 270)

- **What this is:** `call_tool` is one `match` covering every tool family.
- **Problem:** no arm can be tested without the whole dispatcher, and any edit risks an
  unrelated arm. The serp and vision paths are well tested only because they were
  extracted into `serp.rs` / `vision.rs`.
- **Fix:** split by tool family into small functions returning `Value` — a mechanical
  move, no behaviour change.
- **If we skip it:** each new tool makes the file harder to change safely.

### 7. Session eviction picks an arbitrary victim
`webrain-mcp/src/lib.rs:1354-1366`

- **What this is:** at 64 sessions the map drops `map.keys().next()` — hash order, not
  age.
- **Problem:** the 65th concurrent client evicts a session that may be mid-crawl; that
  client's next call reconnects to a fresh backend and loses its tab, cookies and
  session. (The auth token compare at `:97-106` is fine — constant-time and
  length-gated.)
- **Fix:** keep `last_used: Instant` in `SessionMeta` and evict the oldest, or refuse
  with a clear error instead of dropping silently.
- **If we skip it:** a busy shared instance randomly drops one user's browser state.

### 8. SERP re-filters and re-parses already-processed results (`auto` only)
`webrain-core/src/serp.rs:781-839` (`http_search`), `:1445-1664` (`serp_search` merge),
`:676-692` (`filter_relevant`), `:622-637` (`dedupe_cap`)

- **What this is:** `http_search` already calls `filter_relevant` per page and
  `dedupe_key` (a full URL parse) per result. The `auto` arm then runs
  `dedupe_cap(filter_relevant(merged, &query), limit)` over that merged output — so
  relevance filtering and URL parsing each happen twice. `brave_api_search`
  (`:1146-1203`) also filters internally before joining the same merge.
- **Problem:** with `limit=100` across three providers the inner loop does roughly
  twice the work needed. `specific_engine` is unaffected — it returns `http_search`
  output directly.
- **Fix:** have `http_search` / `brave_api_search` return already-filtered, deduped
  results and have `auto` merge with dedupe only (`dedupe_cap`), dropping the second
  `filter_relevant`.
- **If we skip it:** milliseconds per call, but paid on every search.

## Nice to have

### 9. Media capture pumps with a fake command
`webrain-core/src/backends/cdp.rs:1345-1350`

- **What this is:** every 200 ms `capture_media` runs `eval_js("1")` to keep commands
  flowing so the reader keeps draining events. Because of finding 1, each of those
  holds the connection lock.
- **Problem:** a 60 s capture issues ~300 pointless round-trips and blocks every other
  tab for the whole capture.
- **Fix:** one `tokio::time::sleep(wait_ms)` — the pump only exists because replies are
  read under the lock (fixed properly by finding 1).
- **If we skip it:** media capture is the most disruptive single call for other tabs.

### 10. Screenshot keeps the PNG twice
`webrain-mcp/src/tools.rs:1001-1031` (`webrain_screenshot` arm)

- **What this is:** the raw `png` bytes, the base64 string (1.33× bigger) and the file
  on disk are all produced; the base64 is returned even when `path` is.
- **Problem:** a tall full-page capture means bytes + base64 in RAM at once, then both
  are shipped back inside a JSON reply that finding 2 tokenizes.
- **Fix:** return `path` alone when a directory was given (keep `screenshot_b64` only
  when no path was written).
- **Status 2026-11-19 — the fix as written was wrong, and `b64: false` is what landed.**
  The base64 is not a duplicate: `hoist_screenshot` (`webrain-mcp/src/lib.rs:1389`) moves
  it into the MCP `image` content block, which is the only way a client can display the
  picture. Dropping it whenever a path exists would have removed the image from every
  screenshot call, since `dir` defaults to `screenshots`. `b64: false` now returns the
  file path alone for callers that only want the file; batch screenshot rows already
  shipped paths only.
- **If we skip it:** screenshots stay the second-largest reply after vision tiles.

### 11. "Renumber positions 1..N" is written five times
`webrain-core/src/serp.rs:689`, `:634`, `:835`, `:1015`, `:1199`

- **What this is:** the same three-line loop in `filter_relevant`, `dedupe_cap`,
  `http_search`, `browser_search`, `brave_api_search`.
- **Problem:** five copies that must change together if positions ever change meaning.
- **Fix:** one `fn renumber(&mut [SerpResult])`.
- **If we skip it:** low risk today, one more copy tomorrow.

---

## Verification pass (what was re-read)

| # | Claim re-checked | Result |
|---|---|---|
| 1 | Is there a reader task that dispatches replies? | No — `connect` spawns **transport-only** frame pumps; reply matching is inside the lock. Claim holds. |
| 2 | Is tokenization conditional or capped? | No branch; runs on every reply. Holds. |
| 3 | Is `net_urls` capped anywhere? | No `MAX_*` or length guard exists in `cdp.rs`. Holds. |
| 4 | Does `add` normalize vectors? | **No** — stores the `Vec<f32>` unchanged, so per-query `norm` is real waste. Holds. |
| 5 | Are there really 2+ tile clones? | Yes — `inputs` clone plus a second `collect` for describe/caption. Holds. |
| 6 | Function sizes | `call_tool` 926–2322, `handle_rpc` 154–985, `handle_http_conn` 1255–1525. Holds. |
| 7 | Eviction policy | `map.keys().next()` at `:1354-1366`, no age tracking. Holds. |
| 8 | Is the double work everywhere? | **Corrected:** only on the `auto` merge path; single-engine returns `http_search` output directly. |
| 9 | Pump cadence | 200 ms `eval_js("1")` loop confirmed at `:1345-1350`. Holds. |
| 10 | Is base64 still returned with a path? | **Answered:** yes by default and deliberately — it is the MCP image block. `b64: false` opts out. |
| 11 | How many renumber loops? | **Corrected:** five sites, not four. |

Two claims were wrong on the first pass and are corrected above (8 and 11).

---

**Verdict:** healthy core — bounded caches, good parser tests, sensible caps. Fix 1 and
2 first; they are the only two that hurt under normal batch load.

**Lean:** about −150 lines possible (findings 6, 8, 11), and −1 dependency
(`tiktoken-rs`, with its ~2 MB compiled vocabulary) once finding 2 becomes an estimate.

**Not checked:** internals of `pdf.rs`, `video.rs`, `login.rs`, `launch.rs`,
`proxy.rs`, `install.rs`, `browsers.rs`; the docs-site JavaScript; CLI argument parsing
beyond `serp`. `check_index_coverage` reported no gaps for the three crates, but a clean
coverage signal is not proof of completeness.

---

## Status — applied (2026-10-23)

Nine of the eleven findings are implemented. `cargo clippy --workspace
--all-targets` clean, `cargo test --workspace` 92 passed (75 core + 17 mcp),
`cargo fmt --all -- --check` clean, graph re-indexed to 2589 nodes / 6200 edges.

| # | Finding | State | Where |
|---|---|---|---|
| 1 | Lock held across the reply wait | **applied** | `spawn_cdp_reader` + `CdpBackend.pending`; `send_cmd_with` locks only to write |
| 2 | Exact BPE on every reply | **applied** | `with_token_cost`: `chars/4` above 32 KiB |
| 3 | Uncapped, cloned `net_urls` | **applied** | 2048 cap in the reader; `mem::take` in `capture_media` |
| 4 | Per-candidate `norm` in `search` | **applied** | `vision::unit` on `add`/`load`; `search` does a plain dot product |
| 5 | Two tile-PNG copies | **applied** | one `pngs: Vec<String>` reused by embed/describe/caption |
| 6 | 1396/831/270-line functions | **deferred** | see below |
| 7 | Arbitrary session eviction | **applied** | `SessionMeta.used` + `min_by_key` (true LRU) |
| 8 | Double relevance filtering | **applied** | `auto` merge is `dedupe_cap` only |
| 9 | 200 ms fake-eval pump | **applied** | plain sleep; reader drains with no command in flight |
| 10 | `screenshot_b64` alongside `path` | **deferred** | see below |
| 11 | Five renumber loops | **applied** | one `serp::renumber` helper |

**Why 6 is deferred.** Splitting `call_tool` / `handle_rpc` / `handle_http_conn`
is pure churn: ~1400 moved lines across the two most load-bearing files in the
repo, zero change in behaviour, and a large review surface for no functional
gain. It is a readability project, not a performance one, and it belongs in its
own change window — not inside a performance fix where it would make the real
diffs unreviewable.

**Why 10 is deferred.** Returning `path` *instead of* `screenshot_b64` removes a
field a client may already read. That is a feature loss, and the constraint for
this pass was "without losing features". The memory saving is real but the
compatibility decision is the user's, not this audit's.

**Follow-ups discovered while applying.** `send_cmd_with` now fails with
`CDP connection closed` when the reply channel drops instead of surfacing the
underlying parse error; and because the reader task outlives every command, a
`Network.requestWillBeSent` burst during a 120 s PDF can no longer stall the
socket by filling the 64-slot inbound channel — both are strict improvements but
change failure wording no test currently pins.

---

# Phase 1 baseline — 2026-11-19

First measured numbers for the memory contract (56 MB per tab, saturation at 8
tabs). Nothing below changed code; this is the line every later change is judged
against, and the source for the `PER_TAB_MB` constant in
`webrain-core/src/backends/cdp.rs`.

**Machine:** Windows, 32 GB RAM (32,578 MB total / 17,346 MB free at start), headless
Chrome (`--headless=new`, port 9222), fixture served from a local `TcpListener`
(one 4 KB HTML page, distinct `?i=` per row). Release build.

## Extraction, no browser

`cargo bench -p webrain-core --bench extract`

| Benchmark | 1,000 records | 10,000 records |
| --- | --- | --- |
| `autoschema_candidates` | 8.83 ms | 92.1 ms |
| `extract_fields` (4-field schema) | 11.65 ms | 124.9 ms |
| `bm25_filter` (top_k 20) | ~1.3 ms | ~14 ms |

The zero-LLM path runs at roughly 80–110 K records/s, so 10,000 rows cost about a
tenth of a second of CPU. Extraction is not where a 1,000 URL batch spends its
time; the browser is.

## Batch memory and throughput

```
$env:CDP_URL='http://127.0.0.1:9222'; $env:SOAK_URLS='100'
cargo run --release --example soak -p webrain-core
```

| tabs | urls | ok | err | wall ms | rows/s | driver peak MB | browser delta MB | delta/row KB |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 100 | 100 | 0 | 9194 | 10.9 | 11 | 85 | 25.7 |
| 4 | 100 | 100 | 0 | 3094 | 32.3 | 12 | 271 | 30.4 |
| 8 | 100 | 100 | 0 | 2994 | 33.4 | 12 | 514 | 32.6 |
| 16 | 100 | 100 | 0 | 2985 | 33.5 | 13 | 986 | 38.0 |
| 32 | 100 | 100 | 0 | 4304 | 23.2 | 13 | 1841 | 46.2 |

`browser delta MB` is the summed RSS of browser processes at peak minus the same
sum before the first level, so it is attributable to these tabs on a machine that
already has a browser open.

Three things the roadmap can stop guessing:

1. **Throughput saturates at 8 tabs** (33.4 rows/s), and 32 is *slower* than 16
   (23.2 vs 33.5) while costing 1.8 GB instead of 1.0 GB. Past saturation, tabs
   buy memory and latency, not speed.
2. **The browser costs about 56 MB per tab** on this fixture (85 MB at one tab
   including shared browser overhead, then +55 to +60 MB per added tab). The
   roadmap's 96 MiB per-tab estimate was too high; phase 2 should seed
   `tabs_for()` with this measurement and keep it as a tunable constant.
3. **The driver process is flat** at 11 to 13 MB from 1 to 32 tabs, which is the
   evidence that batch rows are streamed rather than accumulated per page. The
   memory that scales is Chrome's, so the tab budget is the only lever worth
   pulling, and `open_tab` is the right place to enforce it.

Reproduce both tables before and after any phase 2 change.


---

# Addendum — the SvelteKit performance guide, against this app

**Date:** 2026-11-23
**Source:** <https://svelte.dev/docs/kit/performance> (fetched, not recalled)
**Method:** the guide's own checklist walked against the real tree. Coverage first,
per this file's usual method — `check_index_coverage` on
`webrain-cli/ui/src/App.svelte` reports **`parse_unusable`, ranges 1–7197**, so the
graph cannot answer for the component and every claim below is read from source.

**The premise that decides most of it:** this page is **not SvelteKit**. There is
no router, no `+page.svelte`, no `load` function, no adapter, no `handle` hook, no
prerender step. It is a plain Svelte 5 + Vite bundle (`webrain-cli/ui`) whose output
(`chat.js`, `chat.css`, `chunk-markdown.js`) is `include_str!`-embedded in the Rust
binary and served by `chat --serve`. Six of the guide's ten "out of the box"
bullets describe machinery we do not run, so their absence is not a gap.

## Item by item

| Guide item | Here | Evidence |
|---|---|---|
| Code-splitting | **Partial, by Vite** — 3 files: app, CSS, markdown chunk | `vite build` output; the chunk is loaded by `import("./markdown.js")` |
| Asset preloading (`modulepreload`) | **N/A** — one JS + one CSS, both in the served HTML | `index.html` is a single inline module that mounts `App` |
| File hashing / cache forever | **Deliberately not done** (see below) | assets served `Cache-Control: no-store`, `chat_server.rs` |
| Request coalescing | **N/A** — no `load` layer | — |
| Parallel loading | **Already true** | mount fires `/settings`, `/permission-rules`, `/pending`, `/sessions`, `/catalog`, `/providers` concurrently; only `loadSessions().then(seed)` is sequenced, and that is a real dependency |
| Data inlining (replay `fetch`) | **N/A** — no SSR | — |
| Conservative invalidation | **N/A** — the page holds one live SSE stream (`/events`) and re-reads a route only when a frame says so | |
| Prerendering | **N/A** — one route, and it is the app | |
| Link preloading | **N/A** — no client-side navigation | |
| Svelte version latest | **Met** | `svelte: ^5` in `webrain-cli/ui/package.json` |
| Images / video | **N/A** — the page ships no images or video; attachments are rendered from the server on demand | `kindOf()` in `App.svelte` |
| Fonts: preload, subset | **N/A, and the waterfall it warns about cannot happen** — zero webfonts | `@font-face` / `fonts.googleapis` / `.woff` = **0 hits**; `--mono` and `--display` are system stacks |
| External scripts | **None** | `index.html` has one inline module and no third-party tag |
| `rollup-plugin-visualizer` | **Not added** — a dependency for a report | `check-budget.mjs` already fails the build when an asset crosses its gzip ceiling, which is the part that keeps it true |
| Selective loading | **Already done** where it matters | two static imports (`svelte`, `./Palette.svelte`), the 84 KB `marked`+`DOMPurify` pair deferred and warmed on idle |
| Instrumenting (`Server-Timing`, OpenTelemetry) | **Not done** | a local single-process server; the browser's own Network panel answers the same question |

## The one contested item: file hashing and long-lived caching

The guide's "hash the filename, cache forever" assumes a static host that can emit
new names per deploy. Here the assets live **inside the binary** and are served
`no-store`, because a rebuilt binary must never hand a browser the previous UI —
that exact failure has already cost one debugging round (screenshots showing a UI
that no longer existed). Correctness beats a 0-byte cache hit on localhost.

The honest upgrade, if this is ever served over a tunnel or on a shared host:
compute one hash of the embedded bytes at startup, answer `ETag` +
`Cache-Control: no-cache` (revalidate, not `no-store`), so a reload of an unchanged
binary is a `304` instead of 220 KB. About ten lines in the asset route of
`chat_server.rs`; not worth it while every client is on loopback.

## What the guide's spirit does ask for, and where it stands

- **A budget instead of a feeling** — done: `check-budget.mjs` fails `bun run check`
  past 80 / 15 / 29 KB gzip (measured 75.0 / 13.1 / 27.3).
- **Do not lay out what nobody sees** — done: `content-visibility: auto` on the
  card-like transcript rows.
- **Small reactive scopes** — started: the composer's palette is its own component;
  ~7.6k lines remain in `App.svelte`.
- **Divide the page before dividing the box** — the transcript's own rows are the
  next slice, and they are the coupled ones (one `{#each}` with a dozen locals).

## Reproducing

```powershell
cd webrain-cli/ui
bun run build            # sizes printed per asset
bun run check            # ends with the gzip budget
```
