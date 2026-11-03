# Plan: fast tiled page parsing (and can webrain answer "screenshot-tiles \<url\>")?

Status: plan only. No code written.
Scope: `webrain-core/src/engines.rs`, `webrain-core/src/backends/cdp.rs`,
`webrain-core/src/vision.rs`, `webrain-mcp/src/tools.rs`, `skills/webrain/`.

## 1. The question, answered first

Prompt A: `screenshot-tiles https://news.ycombinator.com and summarize the top stories`
Prompt B: `screenshot-tiles https://arxiv.org/abs/2404.12387 and explain the key findings`

**Today: no, not as a single call, and badly even as a sequence.**

There is no `screenshot-tiles` tool. The 18 advertised tools (`tools.rs` `list_tools`)
do not include `webrain_pixel`; it exists only as a legacy alias dispatched through
`map_surface`, plus `webrain_observe {what:"pixel"}`. So a model hearing
"screenshot-tiles" has to *invent* the sequence, and the sequence it lands on is:

| Step | Call | Cost today |
|---|---|---|
| 1 | `webrain_eval "[scrollWidth, scrollHeight]"` | geometry discovery, by hand |
| 2 | `webrain_observe {what:"pixel", tile_width, tile_height, max_tiles}` | **N sequential CDP captures**, every tile base64 in the reply |
| 3 | `webrain_vision {op:"ask", tiles:[...], prompt}` | **the same N captures again**, then one model call |

Three failures that matter:

1. **N captures for one page.** `TileEngine::tile` (`engines.rs:41`) loops
   `screenshot_clip(x, y, w, h, 1.0)` once per tile. Each `Page.captureScreenshot`
   with `captureBeyondViewport: true` (`cdp.rs:2836`) re-rasterizes the whole page
   surface. 16 tiles = 16 full-page rasterizations to produce 16 crops of the *same*
   pixels. "Parallel tabs" does not help: one CDP connection, one WebSocket, and the
   captures still serialize behind the renderer.
2. **The tiles travel through the context window.** `observe(what:pixel)` returns
   `png_b64` per tile — a 935x1210 PNG page is ~660 KB, so 16 tiles is ~10 MB into
   the tool result. This is the same spill-to-file situation measured on a single
   PDF page earlier (864 KB payload for one page).
3. **Two coordinate conventions.** `TileEngine` clips in **document** space (offsets
   from `scrollHeight`); `vision(op=ask, tiles)` treats its rects as **viewport**
   space and adds `scrollX/scrollY` (`vision.rs`, the `off` probe). Tile rects from
   step 2 are therefore wrong for step 3 on any scrolled page — silently, with a
   confident answer about the wrong band.

Prompt B is worse: `arxiv.org/abs/...` is an HTML abstract page with a text layer.
Tiling it burns 16 tiles of VLM to read text that `webrain_extract` /
`observe(what:fit)` returns in ~2 KB. Tiles are the right tool there only for the
figure, not the page.

**After this plan:** prompt A is 2 calls (`navigate` -> `observe(what:pixel)`) with
one capture and zero base64 in the reply, and prompt B is routed to text first. The
recipe lives in a skill so the model does not have to invent it.

## 2. What PixelRAG does that is worth copying (and what is not)

Verified against `StarTrail-org/PixelRAG`:

| Their approach | Copy? |
|---|---|
| Capture one tall surface (`tile_height` 8192) and crop strips **client-side** | **Yes** — this is the whole fix |
| Chunk to 1024 px strips before embedding so the VLM never gets an oversized image | Yes, for the index path (`vision op=index`), not for reading |
| 48 Chrome workers, raw BGRA -> `/dev/shm`, process-pool JPEG | **No** — 28M-tile offline ingest; we render one page per call |
| Scroll + wait for `document.images` per tile | **No, not yet** — see §5 |
| FAISS IVF over 214 GB, Qwen3-VL-Embedding-2B | No — our index is a JSONL linear scan by design |

## 3. Ranked changes

Each item states its self-check; a change without one is not done.

### T1 — capture once, crop locally (the only change that matters for speed)
`TileEngine::tile`: one `backend.screenshot(true)` (full page), decode with the
`image` crate (already a dependency, used by `pdf_render`), crop each tile, encode
PNG. 1 rasterization + N cheap crops, replacing N rasterizations.
- Also fixes lazy images *partially*: a full-page capture forces layout of the whole
  document, though not off-screen image fetches.
- Self-check: unit test that crops a synthetic 200x600 image at 200x200 into 3 tiles
  with the expected offsets and sizes; assert the capture count is 1 by counting
  backend calls with a stub `BrowserBackend`.
- Fallback: if the full-page PNG decode fails (huge page, OOM), fall back to the
  current per-clip loop — keep the old path behind that branch rather than deleting
  the capability.

### T2 — let vision consume tiles without re-capturing them
`vision(op:ask)` gains `images: [b64]` as an alternative to `tiles`. Then
`observe(what:pixel)` -> `vision(op:ask, images:[...])` needs no second capture and
no coordinate convention at all.
- Keeps `tiles` for the caption/coords use cases (captcha geometry) exactly as is.
- Removes the document-vs-viewport trap for the new path by deleting the rect, not
  by documenting it.
- Self-check: assert the request builder emits text-first then images in order, and
  that a call with `images` performs zero `screenshot_clip` calls.

### T3 — stop shipping base64 when nobody asked
`tiles_dir` on `observe{what:pixel}` writes the tiles and returns paths; `png_b64`
stays empty. **Moved to the top of the list after measuring**: base64 is the *context*
cost (1.87 MB -> 226 bytes on HN), capture count is only CPU. PixelRAG's lesson
holds — images belong on disk, never in the transport.
- Implemented; self-check is the `crop_png` + `tile_rects` unit tests plus a live run.

### T4 — one alias so the literal prompt resolves
Add `screenshot_tiles` -> `observe(what=pixel)` to `map_surface`, and mention
"screenshot tiles" in the `webrain_observe` description. One line each. This is
cheap insurance, not the fix — the skill (T6) is the fix.

### T5 — parallel across pages, not within a page (defer)
`webrain_batch` already parallelizes tabs per URL. A `op:"pixel"` arm would give
"tile these 20 pages", which is the only place parallelism beats the single-capture
win. Defer until T1 lands and a real N-page task appears: within one page, parallel
`captureScreenshot` calls only queue behind the same renderer.

## 4. The two prompt paths after T1/T2

```
A) navigate https://news.ycombinator.com
   observe {what:"pixel", tile_width:900, tile_height:1200}   # 1 capture, 2-3 tiles
   vision  {op:"ask", images:[...], prompt:"Summarize the top stories, one line each."}
   => 3 calls, 1 page rasterization

B) navigate https://arxiv.org/abs/2404.12387
   observe {what:"fit"}            # abstract + authors as text, ~2 KB
   observe {what:"pixel", tile_width:900, tile_height:1200}   # only if the figure matters
   vision  {op:"ask", images:[...], prompt:"Describe the main figure."}
   => text first, tiles only for the figure
```

## 5. Explicitly NOT building

- **A scroll engine.** `captureBeyondViewport: true` already captures below the fold;
  scrolling is needed only for `loading="lazy"` galleries and infinite scroll. Add
  `scroll_wait: true` only when a real page returns blank tiles, and then as a single
  pre-pass (scroll to bottom once, wait for `document.images`, scroll back), not the
  per-tile scroll loop PixelRAG runs.
- **Per-tile parallel capture.** Same renderer, same surface; it queues.
- **A second vision model.** Unlimited-OCR is a 3B PyTorch/CUDA transcriber — it
  needs a Python stack this repo deliberately does not have. If it is ever worth it,
  it enters as `OCR_URL` (an optional HTTP endpoint), the same shape as `EMBED_URL`.
- **Replacing the JSONL index.** Fine below a few thousand vectors; the local
  threshold is already documented in `vision.rs`.

## 6. Skill (do this, it is the actual answer to the prompt)

New `skills/webrain/page-tiles/SKILL.md`, referenced from the router `SKILL.md`:

- **Trigger**: "screenshot-tiles", "look at this page", "read the chart", "summarize
  what is visible", "the text extract is empty/garbled".
- **Decision rule (first line)**: text layer beats pixels. `observe(what:fit)` /
  `extract` first; tiles only for charts, tables, figures, image-only pages, and
  captcha geometry.
- **The 3-call recipe** from §4, with the tile size rule (900x1200 full width, not
  800x800 squares that cut columns in half) and `max_tiles` guidance.
- **Payload rule**: never paste `png_b64` back to the user or into a later prompt;
  hand the tiles to `vision(op:ask, images:[...])` and keep the answer.
- **Trap list**: vision reads browser pixels, not files (already learned in
  `pdf-extraction`); one or two fields per ask, never a count; a `crippled:true`
  image page is normal.
- Self-check: `webrain install skills` warns on unknown `webrain_*` names — the new
  skill must produce no warning (the existing guard in `skills.rs` tests covers it).

## 7. Order of work

| # | Step | Reaches | Gate |
|---|---|---|---|
| 1 | T1 capture-once-crop + test | `engines.rs`, maybe `cdp.rs` | `cargo test -p webrain-core`, plus a manual HN tile run |
| 2 | T2 `images` on vision ask | `vision.rs`, `tools.rs` schema | `cargo test`, one forced-local run |
| 3 | T6 skill + router link | `skills/webrain/**` | `cargo test -p webrain-core skill` |
| 4 | T3 `write_dir` | `tools.rs`, `engines.rs` | tile round-trip test |
| 5 | T4 alias + description | `tools.rs` | `list_tools` diff |
| 6 | T5 batch `op:"pixel"` | `tools.rs` | only if an N-page task appears |
| 7 | CHANGELOG `[Unreleased]` + `docs/reference/tools.mdx` + decision guide | docs | `mintlify validate` |

## 8. Risks
- **Full-page capture on a pathological page** (100k px tall, 40 MB PNG) can blow
  memory. Mitigation: the T1 fallback branch, plus a `max_tiles`-driven early exit
  when `scrollHeight` exceeds a cap (e.g. 40 000 px) — capture in vertical bands
  instead of one surface.
- **Changing `TileEngine` output shape** would break `observe(what:pixel)` consumers
  and the PixelRAG-style index path. Keep the `TileShot` shape; only the capture
  strategy changes.
- **T2 adds a second way to feed vision.** Two ways is a smell — the plan is to route
  every new caller through `images`, and mark `tiles` as the coordinate/captcha path
  in its description so the split stays legible.

## 9. Measured 2026-11-03 (implemented, HN front page, Chrome CDP 9333, local 2B)

Page 804x1246 -> 2 tiles at 900x1200.

| Metric | Value |
|---|---|
| `navigate` | 1031 ms, `challenge: null` |
| one whole-page capture | 103 ms, 145 KB |
| 2 tiles: capture + crop + write | 499 ms, 339 KB on disk |
| tool reply | **226 bytes** (vs ~1.87 MB if base64) |
| `vision{op:ask, images:[2 paths]}` | 17.0 s, correct HN story list |

**Capture-once is not a CPU win at small tile counts.** PNG decode + re-encode of a
~1 MP tile costs ~150-200 ms, more than the 103 ms clip it replaces; at 2 tiles the
new path is ~2x slower in wall time while producing 8000x less context. It wins on
round trips (1 CDP call instead of N) and on context (the actual constraint), and
should win on CPU only for tall pages where N is large. If CPU ever matters:
encode JPEG q85 for tiles (~10x cheaper encode, ~5x smaller) and derive the vision
data-URL mime from the file extension — deliberately NOT done here, because q85 on
600 px captions costs OCR accuracy and accuracy was the stated priority.

Also measured: the local 2B emits a preamble ("The user wants a summary of the
provided screenshots.") before the answer at `max_tokens: 2048`, so prompts must ask
for the output directly and callers must tolerate one leading line.

