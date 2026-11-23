# Memory & Performance Audit — webrain

**Originally:** 2026-10-23 (whole repo). **Pruned:** 2026-11-23 — shipped findings
are removed from this file and live in git history; what remains is what is still
open or deliberately declined. Re-measure before trusting any number here.

**Method:** `check_index_coverage` first (the graph is best-effort: `App.svelte` is
`partial`, one unusable range at 4469 — read that region directly), then
`search_graph` / `query_graph` / `search_code`, then the cited lines re-read.

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

### 2. The chat page is still one 7.6k-line component (frontend)

`webrain-cli/ui/src/App.svelte` (7,863 lines). Two slices are out so far — the
composer's palette (`Palette.svelte`) and the snackbar (`Toasts.svelte`) — which
proved the pattern: each takes its markup *and* its styles, and its keyframes with
it, because Svelte scopes both to the defining component.

**Fix:** continue with the remaining presentational blocks, in this order:
the tools chooser (`#tools-modal`), the settings panel, the model dialog, then the
transcript rows. The rows are last and hardest: one `{#each}` with a dozen locals
(`previews`, `editArgs`, `toggles`, `item`), so they need a real prop contract rather
than a move.

**Cost so far, measured:** the two component boundaries cost +0.2 KB gzipped each
(74.2 → 75.2 KB for `chat.js`), inside the 80 KB ceiling.

### 3. Nothing yet measures *painting*

Every frontend number so far is either a bundle size or a JS timing measured in the
page (a 9 KB draft costs 0.03 ms per keystroke, so the composer is not a hot path).
No frame timeline of a streaming turn exists, which is the one place a chat can stutter
— per-token DOM insertion plus markdown re-parse. **Next measurement, not next code:**
record one turn in the Performance panel and report the longest task and the
dropped-frame count before touching the render path.

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
