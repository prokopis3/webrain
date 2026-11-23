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

### 1. Three functions do the work of twenty (backend)

`webrain-mcp/src/tools.rs:926-2322` (`call_tool`, ~1400 lines),
`webrain-mcp/src/lib.rs:154-985` (`handle_rpc`, 831), `:1255-1525`
(`handle_http_conn`, 270).

`call_tool` is one `match` covering every tool family: no arm can be tested without
the whole dispatcher, and any edit risks an unrelated arm. The serp and vision paths
are well tested only because they were extracted into `serp.rs` / `vision.rs`.

**Fix:** split by tool family into small functions returning `Value` — mechanical, no
behaviour change. **Why it is still open:** ~1400 moved lines across the two most
load-bearing files, zero functional gain, and a review surface that would hide the
real diffs if it landed beside a fix. It is a readability project with its own
change window.

### 2. The chat page is still one 7.5k-line component (frontend, readability)

`webrain-cli/ui/src/App.svelte`, 7,540 lines. The palette, the snackbar and the
tools chooser are out (see `CHANGELOG.md`); each took its markup, styles and
keyframes with it, because Svelte scopes all three to the defining component, and
the chooser also had to redeclare the dialog shell (`#modal` backdrop, `.card`,
primary button) because `App`'s rules never reach a child's DOM.

**This is not a performance item, and the measurement says so** (see item 3):
opening the settings panel and stepping through all nine tabs produces **zero**
long tasks. The reason to split is that 7,540 lines is hard to review.

**A boundary is not free — measured, not assumed:** palette `+0.8 KB` gzipped,
snackbar `+0.2`, tools chooser `+2.8` (it copied the dialog shell), and a
`Dialog.svelte` shared shell written to remove that duplication cost **`+0.73`
with only one consumer** — it was reverted rather than shipped. Call it ~0.7 KB
per component boundary, so a full split of the remaining 7,540 lines lands
somewhere near `+7 KB`.

**Decision (recorded, so it stops being re-discovered):** the 80 KB ceiling is a
*bug detector*, not a decomposition budget. If the page is split further, the
ceiling moves to 88 KB in the same commit that crosses 80, with this paragraph as
the reason — and the split goes **per tab**, not per panel: the settings card is
nine tabs in one 355-line block over ~40 shared values (`manage`, `toggles`,
`skillNew`, `mcpNew`, `providers`, …), so a whole-panel move would need a
40-prop component, which is worse code than what it replaces. One tab per
component, each with a handful of props, measured after each.

The transcript rows stay last: one `{#each}` with a dozen locals, and the
measurement below shows they are already cheap to scroll.

### 3. Painting is now measured, and the transcript is not the problem

Measured 2026-11-23 on a 55-message thread (`c1794890501696`: 33 `.msg`, 82
`content-visibility` rows, 9,972 px of transcript in a 603 px viewport), debug
build, headless Chrome, via `PerformanceObserver` on `longtask` plus a
`requestAnimationFrame` delta loop while scrolling 30 steps top→bottom:

| Interaction | Long tasks | Longest | TBT | Frames | P50 / P95 | Slow (>34 ms) |
|---|---:|---:|---:|---:|---|---:|
| First scroll after opening the thread | 1 | **134 ms** | 84 ms | 67 | 16.7 / 16.8 ms | 1 |
| Second scroll | 0 | — | 0 | 30 | 16.7 ms | 0 |
| Third scroll | 0 | — | 0 | 30 | 16.7 ms | 0 |
| Settings open + all 9 tabs | 0 | — | 0 | — | — | — |

**Reading:** the 134 ms task is the first layout of the rows that
`content-visibility: auto` had been skipping — it happens once per thread, and
never again on the same thread. Steady-state scrolling is a locked 60 fps, and
the settings panel does not block the main thread at all, which is the evidence
that item 2 is readability work rather than a performance fix.

**Declined on the record: virtualising the transcript.** One 134 ms task on the
first view of a 55-message thread, paid once, is under the ~200 ms a deliberate
navigation can absorb, and a virtual list would add scroll restoration, anchoring
and `find in page` regressions to buy back 84 ms of blocking time. Revisit if a
thread reaches the hundreds of messages, or if the first-view task crosses 200 ms.

**Still unmeasured:** a *streaming* turn (per-token DOM insertion plus markdown
re-parse) — the provider was out of credits, so no live turn could be recorded.
That is the one place a chat can stutter, and it needs a funded key, not code.

**Reproducing** (paste in the served page's console, DevTools open):

```js
const t = [], po = new PerformanceObserver((l) => l.getEntries().forEach((e) => t.push(e.duration)));
po.observe({ entryTypes: ["longtask"] });
const sc = [...document.querySelectorAll("*")].find((e) => e.scrollHeight > e.clientHeight + 300 && !e.classList.contains("rail"));
for (let i = 0; i <= 30; i++) { sc.scrollTop = ((sc.scrollHeight - sc.clientHeight) * i) / 30; await new Promise(requestAnimationFrame); }
console.table(t);  // then scroll again — the tasks should be empty
```

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
