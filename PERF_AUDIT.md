# Memory & Performance Audit — webrain

**Originally:** 2026-10-23 (whole repo). **Pruned:** 2026-11-23 — shipped findings
are removed from this file and live in git history; what remains is what is still
open or deliberately declined. Re-measure before trusting any number here.

**Method:** `check_index_coverage` first (the graph is best-effort: `App.svelte`
currently reports `parse_unusable` for the whole file, so read it directly), then
`search_graph` / `query_graph` / `search_code`, then the cited lines re-read, then
measure in the running page before calling anything a performance problem.

---

## Open

### 1. `handle_tools_call` is one 1,069-line function (backend)

`webrain-mcp/src/lib.rs:280-1348`. It is out of `handle_rpc` — that was the fix
for "one `match` answers four methods" — but it still does four jobs in sequence:
fold the intent surface, run the no-state / no-browser guards, connect (and retry)
a backend, then call and post-process whatever came back.

**Fix:** cut at those seams — `guards`, `connect`, `execute`, `postprocess`, each
returning `Value` — so a change to the retry policy stops being a change to the
same function as a change to the guards.

**Why open:** the body is byte-identical to the arm it came from, this is the path
every transport ends in, and the payoff is reviewability rather than speed. It
needs its own change window, the way the first two cuts did.

### 2. `handle_http_conn` is a 724-line router that writes responses inline (backend)

`webrain-mcp/src/lib.rs:1780-2503`. Unlike `call_tool` and `handle_rpc` this is not
a `match`: it parses the request line and headers, answers `/health`, enforces the
optional bearer token, reads the body, dispatches, and formats each response in
place — so a route's logic and its HTTP framing are the same twenty lines.

**Fix:** separate routing (`path + method → Option<route>`) from response building
(`fn reply(status, body) -> String`, `fn sse_chunk(..)`).

**Why open:** it is the transport every HTTP client depends on; a byte-level slip
here is a client outage, not a bad line number.

### 3. The chat page is still one 7.5k-line component (frontend, readability)

`webrain-cli/ui/src/App.svelte`, 7,523 lines. The palette, the snackbar and the
tools chooser are out (see `CHANGELOG.md`); each took its markup, styles and
keyframes with it, because Svelte scopes all three to the defining component, and
the chooser also had to redeclare the dialog shell (`#modal` backdrop, `.card`,
primary button) because `App`'s rules never reach a child's DOM.

**This is not a performance item, and the measurement says so:** opening the
settings panel and stepping through all nine tabs produces **zero** long tasks,
and scrolling a 55-message thread holds 16.7 ms frames. The reason to split is
that 7,523 lines is hard to review.

**A boundary is not free — measured, not assumed:** palette `+0.8 KB` gzipped,
snackbar `+0.2`, tools chooser `+2.8` (it copied the dialog shell), and a
`Dialog.svelte` shared shell written to remove that duplication cost **`+0.73`
with only one consumer** — it was reverted rather than shipped. Call it ~0.7 KB
per component boundary, so a full split of the remaining 7,523 lines lands
somewhere near `+7 KB`.

**Decision (recorded, so it stops being re-discovered):** the 80 KB ceiling is a
*bug detector*, not a decomposition budget. If the page is split further, the
ceiling moves to 88 KB in the same commit that crosses 80, with this paragraph as
the reason — and the split goes **per tab**, not per panel: the settings card is
nine tabs in one 353-line block over ~40 shared values (`manage`, `toggles`,
`skillNew`, `mcpNew`, `providers`, …), so a whole-panel move would need a
40-prop component, which is worse code than what it replaces. One tab per
component, each with a handful of props, measured after each. The shared atoms
those tabs draw are in `ui/src/app.css`, so a tab costs its markup and no form CSS.

The transcript rows stay last: one `{#each}` with a dozen locals, and the
measurement shows they are already cheap to scroll.

---

## Declined on purpose

### Long-lived caching of the UI assets

SvelteKit's guide says hash the filename and cache forever. Here the assets are
`include_str!`-embedded in the binary and served `Cache-Control: no-store`, because a
rebuilt binary must never hand a browser the previous UI — that exact failure already
cost a debugging round. Correctness beats a 0-byte cache hit on loopback.

**The upgrade, if this is ever served over a tunnel:** hash the embedded bytes once at
startup, answer `ETag` + `Cache-Control: no-cache` (revalidate, not `no-store`), so a
reload of an unchanged binary is a `304` instead of ~220 KB. About ten lines in the
asset route of `chat-server`.

### Virtualising the transcript

Measured, then declined: the first view of a 55-message thread pays **one 134 ms
long task** (84 ms of blocking time) laying out the rows `content-visibility: auto`
had been skipping, and never pays it again. A virtual list would add scroll
restoration, anchoring and `find in page` regressions to buy back 84 ms of a
once-per-thread cost. Revisit at hundreds of messages, or if that first-view task
crosses 200 ms.

### `rollup-plugin-visualizer`

A dependency for a report. The budget gate answers the same question with an
assertion, which is the half that keeps it true.

---

## The SvelteKit performance guide, and why most of it is not ours

<https://svelte.dev/docs/kit/performance>, walked item by item on 2026-11-23. **This
page is not SvelteKit** — no router, `load` function, adapter, `handle` hook or
prerender — so six of its ten "out of the box" bullets describe machinery we do not
run. Their absence is not a gap.

| Guide item | Here |
|---|---|
| Code-splitting | Partial, by Vite: app + CSS + deferred markdown chunk |
| Asset preloading, hashing, coalescing, data inlining, prerendering, link preloading | N/A — one route, no SSR, one served document |
| Parallel loading | Already true: six mount fetches fire together; only `loadSessions().then(seed)` is a real dependency |
| Conservative invalidation | Already true: one SSE stream, routes re-read only when a frame says so |
| Images / video | None shipped; attachments are fetched from the server on demand |
| Fonts | **Zero webfonts** — `@font-face` / `fonts.googleapis` / `.woff` = 0 hits, so the JS→CSS→font waterfall it warns about cannot occur |
| External scripts | None — `index.html` is one inline module |
| Selective loading | Done where it matters: `marked` + `DOMPurify` in a deferred, idle-warmed chunk |
| Svelte version | 5.57.3 |

---

## Baseline worth keeping: the memory contract (2026-11-19)

The source for `PER_TAB_MB` in `webrain-core/src/backends/cdp.rs`. Windows, 32 GB,
headless Chrome, one 4 KB fixture page, release build.

| tabs | urls | wall ms | rows/s | driver peak MB | browser delta MB |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 100 | 9194 | 10.9 | 11 | 85 |
| 4 | 100 | 3094 | 32.3 | 12 | 271 |
| 8 | 100 | 2994 | 33.4 | 12 | 514 |
| 16 | 100 | 2985 | 33.5 | 13 | 986 |
| 32 | 100 | 4304 | 23.2 | 13 | 1841 |

1. Throughput saturates at 8 tabs; 32 is *slower* than 16 (23.2 vs 33.5 rows/s) for
   1.8 GB instead of 1.0 GB. Past saturation, tabs buy memory and latency.
2. About 56 MB of browser per tab on this fixture; seed `tabs_for()` with it.
3. The driver process is flat at 11–13 MB, which is the evidence that batch rows are
   streamed rather than accumulated per page.

Extraction, no browser (`cargo bench -p webrain-core --bench extract`): 10,000
records in 92 ms (autoschema), 125 ms (4-field schema), ~14 ms (BM25 top-20) — about
80–110 K records/s, so a batch's time is the browser, not the parser.

---

## Re-measuring

```powershell
cd webrain-cli/ui
bun run build     # per-asset sizes
bun run check     # ends with the gzip budget (80 / 15 / 29 KB)
```

Backend: `cargo clippy --workspace --all-targets`, `cargo test --workspace`, the
`soak` example for the tab table above. After adding or moving a source file, run
`index_repository(mode="fast")` **before** trusting a coverage signal — a new file
reads `not_tracked` until it is indexed, which is the index being old, not the code
being absent.

Painting, in the served page's console with DevTools open:

```js
const t = [], po = new PerformanceObserver((l) => l.getEntries().forEach((e) => t.push(e.duration)));
po.observe({ entryTypes: ["longtask"] });
const sc = [...document.querySelectorAll("*")].find((e) => e.scrollHeight > e.clientHeight + 300 && !e.classList.contains("rail"));
for (let i = 0; i <= 30; i++) { sc.scrollTop = ((sc.scrollHeight - sc.clientHeight) * i) / 30; await new Promise(requestAnimationFrame); }
console.table(t);  // the first scroll may show one task; later scrolls should be empty
```

Streaming, which is the path a chat can actually stutter on.

**Real provider turn, measured 2026-11-23** (`webrain chat --serve`, provider
`deepseek` / `deepseek-flash`, 1,201 output tokens, the page's own footer:
`129.9 tok/s · 9.3s`). A `MutationObserver` on the transcript recorded **62 DOM
updates for a 1,410-char reply — median 23 chars per update, p90 39, biggest 113
(the "webrain copy" label), median gap 17 ms — and zero long tasks.** Reasoning
tokens arrive before any answer text: the first content update lands ~8.2 s in,
the answer then streams in ~1.1 s. Rendering cost is therefore bounded by chunk
size (≤ ~40 chars), not by token count, and stays far below one frame.

**Synthetic stress baseline, same day:** 420 tokens 18 ms apart (≈55 tok/s)
through the page's own `render()` from `chunk-markdown.js`, inserted into a real
transcript row — 430 frames over 8,131 ms, P50 16.7 ms, P95 33.4 ms, max 33.6 ms,
no frame over 34 ms, zero long tasks, TBT 0. The 33.4 ms P95 is the harness: the
loop yields 18 ms between paints, so every few frames get two intervals. This is
the harsher profile (finer chunks than any provider sends) and still passes.

Two traps when reproducing, both of which produced wrong numbers first:

- **In-page `setTimeout` is throttled when the window lacks OS focus.** A
  `hasFocus: false` tab stretched a 12.1 s turn into 93 s of wall clock, froze
  `requestAnimationFrame` for 91 s and coalesced the whole reply into one
  1,427-char update — which reads exactly like "the provider sends one giant
  chunk". Poll from outside the page (`page.evaluate` in a loop) and record with a
  `MutationObserver`, which fires on the DOM change itself. Check
  `document.hasFocus()` before trusting any in-page timing loop.
- **The first turn navigates** (to the conversation URL), destroying the evaluate
  context. Establish the thread with a throwaway turn, or re-arm on each poll.

`output/mock-stream.mjs` serves the same OpenAI-shaped SSE chunks locally when the
full transport path needs recording without spending provider credit; point a
provider row at `http://127.0.0.1:8811/v1` for the duration and delete it after.
