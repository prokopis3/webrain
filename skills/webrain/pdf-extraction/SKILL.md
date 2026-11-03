---
name: pdf-extraction
description: 'Extract text, tables, figures and page images from PDFs — one file, many files, or URLs — with webrain_pdf and webrain_batch op=pdf. Use when asked to read/summarise a paper or report, convert PDF to Markdown, pull tables out of a PDF, OCR a scanned PDF, describe figures/charts, or extract a batch of PDF links found by a crawl. Covers arxiv.org/pdf, ?download= links, login-gated PDFs, and when to switch from text extraction to render + vision.'
---

# PDF extraction (single file, batch, and URL)

Zero system dependencies for text. The parse is **pure Rust** (`pdf-inspector` on
`lopdf`), so it needs no browser, no Python, no poppler — which also means a PDF
never needs a CDP tab and never hits anti-bot. A URL is fetched and parsed
in-memory: you never have to save the file yourself.

## Text first — measured on a 9-page paper

| path | time | output |
|---|---|---|
| `op:extract` | **5.2 s** | 48 KB markdown, 32,401 chars, 10 table rows |
| `op:render` + tiles + local vision | 157.9 s | 17,330 chars (53% of the text) |
| `op:images dir:…` | **< 1 s** | 29 PNGs at native size, 3 KB reply (arxiv 2206.01062) |

Vision costs ~16 s **per page** (2048-token cap at ~12 tok/s) and the tiling step is
1.4% of that — so never parallelize tiles, route instead: extract, then send only the
pages whose `texts[n].text` is empty or short to `op:render` + vision. That is where
tiles earn their keep: on the measured paper a figure-only page returned **0 chars**
from the text layer, and vision recovered its appendix heading and Figure caption
(along with a chain-of-thought preamble — take the last paragraph, not the first).

`layout` now answers "which pages" for you, so you do not loop the whole document:

| field | meaning | trust it for |
|---|---|---|
| `pages_with_raster_images` | pages holding an image XObject | fact — `op:images` can decode these |
| `pages_with_captions` | pages whose text has a `Figure N` / `Table N` caption | hint — the figure may be **vector**, invisible to both the text layer and `op:images` |
| `pages_needing_vision` | empty-text pages + captioned pages with no raster | the routing list (measured: 6 of 9 on a figure-heavy paper) |

A caption is a hint, not a gate: it over-triggers on table pages whose table the text
layer already recovered (10 pipe rows on the measured report), and it misses a figure
whose caption sits on the *previous* page. Start from `pages_needing_vision`, then
subtract any page whose `texts[n].text` already contains the content you need.

A figure page is *not* a transcription job: ask vision to **describe the figure / read
the table**, not to reproduce the page. Full-page transcription on a dense figure page
burns the 2048-token budget on the preamble and truncates at ~1000 chars.

## "Everything in one file" — the whole-document recipe

When the ask is *all of it* (text + tables + figures + what the figures show), three
calls do the job, and only the first one is big:

```
1. webrain_pdf {op:"extract", path}                      -> markdown + pipe tables + layout
2. webrain_pdf {op:"images",  path, dir:"out/img"}       -> native-size figure PNGs
   webrain_pdf {op:"render",  path, dpi:150, dir:"out/pg"} -> page PNGs for the pages vision owns
3. webrain_vision {op:"ask", images:[...], prompt:"…"}    -> per-figure reads
```

Then assemble: text layer as the body, the vision reads for the figures and for any
page whose `texts[n].text` came back empty, images linked by relative path. Keep the
two layers labelled — a 2B model's table transcription and the text layer's pipe table
are different evidence, and on the measured paper they agreed cell-for-cell while the
model truncated after ~3 rows.

Measured on arXiv 2206.01062 (9 pages, 4.3 MB): extract 53,758 chars / 56 pipe rows in
5 s, 29 images + 9 page tiles on disk in ~3 s, 6 vision reads. The only page the text
layer lost was page 1 — title, five authors and the abstract exist solely as pixels.

## When to use
- "extract / read / summarise this PDF", "convert this paper to markdown"
- "get the tables out of this report", "OCR this scanned document"
- "here are 200 PDF links from a crawl — extract them all"
- "describe the figures/diagrams in this paper"

## The two entry points

| Goal | Call |
|---|---|
| 1..N PDFs (local paths **or** URLs) | `webrain_pdf` → `op:extract`, `path:` or `paths:[...]` |
| Same, from the batch tool | `webrain_batch` → `op:pdf`, `urls:[...]` (browserless) |
| Embedded raster images | `webrain_pdf` → `op:images`, `path:`, `pages:[]`, **`dir:`** |
| Page PNGs / tiles for vision | `webrain_pdf` → `op:render`, `path:`, `pages:[]`, `dpi:`, `tile_size:`, **`dir:`** |
| Print the **open browser page** to PDF | `webrain_pdf` → `op:page` (base64 PDF) |

**Always pass `dir`.** Without it `op:render` returns base64 in the tool result — a
9-page 150-dpi render is **15 MB** (the entire context budget for one document), and 29
extracted figures are ~20 MB. With `dir` the same calls answer in 500 B and 3 KB:
render writes `p001.png` (or `p001_t0x0.png` with `tile_size`), images writes
`img_p001_000.png` at the embedded image's **native** size (no downscale, no re-encode).
Those paths then go straight into `webrain_vision` `{op:"ask", images:[...]}`.

`op:extract` and `op:pdf` run concurrently (one thread per core) and preserve
input order. A `paths:[]`/`urls:[]` call is the *batch* path — do not loop N
single calls.

## Procedure — textual PDF

1. Call `webrain_pdf` `{op:"extract", path:"<url-or-path>"}` (or `paths:[...]`).
2. Check `results[i].error` first; a bad path/URL reports there, not as a throw.
3. Read `pdf_type` and `texts[page].needs_ocr`:
   - `TextBased` + `needs_ocr:false` everywhere → done. `markdown` has headings,
     lists, bold/italic and pipe tables.
   - anything else, or any `needs_ocr:true` → this is a scan / image page. Go to
     **Procedure — scanned page** below. Do not summarise from empty text.
4. Trust but verify tables: the extractor emits pipe tables from cell geometry.
   Borderless/merged-header tables lose row alignment (real case: a 4-row perf
   table came back with the memory column shifted onto its own line). If a
   number matters, re-read that page with `op:render` + `webrain_vision op:ask`.
5. `layout.is_complex` / `pages_with_tables` / `pages_with_columns` tell you
   *where* to look before you render anything.

## Procedure — batch / URLs

1. **Best:** `webrain_batch` `{op:"pdf", urls:["https://…/a.pdf", …]}`.
   Downloads and extracts concurrently, no browser needed. Returns
   `{count, results[]}` — the same per-document objects as `op:extract`, input
   order preserved, and a dead link fails only its own `results[i].error`.
2. Equivalent: `webrain_pdf` `{op:"extract", paths:[…]}` — identical engine;
   local paths and URLs can be mixed in one call.
3. Local files you already downloaded: pass the paths straight to `paths:[]`.
4. Never use `op:markdown|fetch|screenshot` on a PDF URL: Chrome renders PDFs in
   a viewer plugin with **no DOM**, so those ops return empty text (or a
   placeholder PNG) and look like a failed scrape.
5. Downloaded with `webrain_download` first? Its `results[].title` **is** the
   saved path — feed that to `paths:[]`. Downloads keep a `.pdf` extension even
   when the link has none, but `filter_extension:"pdf"` still matches the **URL**,
   so it silently rejects `arxiv.org/pdf/2408.09869`; drop the filter for those.

## Procedure — scanned page / figures

1. `webrain_pdf` `{op:"render", path, pages:[3], dpi:150, dir:"out/pages"}` → the paths
   to `out/pages/p003.png`. Requires a `--features pdfium` build with
   `pdfium.dll`/`libpdfium.so` on PATH. If it returns the "needs a build with
   --features pdfium" error, that is a capability limit, not a broken PDF: fall back to
   `op:images dir:…`, or open the PDF in real Chrome and screenshot it.
2. **Ask vision about the files, not the screen.** `webrain_vision`
   `{op:"ask", images:["out/pages/p003.png"], prompt:"…"}` reads the PNGs from disk —
   no CDP, no tab, no `file://` navigation, no second capture. (The old
   navigate-to-the-PNG dance is gone; `images:` is checked before any browser branch.)
   Use the pixel path only when the target is a live page: there a clip larger than
   the viewport plus `scale:3` still matters, because CDP captures from the surface so
   the clip need not fit on screen (measured on 5 arXiv papers: 5/5 correct with the
   clip + scale, 0/5 without).
3. Resolution comes from `dpi`, not from `scale`: 150 dpi is a 1275×1650 page and reads
   body text and captions; drop to 110 only when the model needs the whole sheet at
   once. Native-size `op:images` output (1025×1025 page renders, in the measured paper)
   is sharper than any page render — prefer it when the figure exists as a raster.
4. Prompt shape decides whether it answers or thinks aloud. A terse
   "TITLE and AUTHORS only. UNREADABLE if illegible." returns two clean lines;
   "give 3 short lines / the first 20 words" made the 2B model dump its whole
   chain of thought into `answer` **and** name the wrong paper. Ask for one or
   two fields, and never ask a small VLM to count.
5. For figure-by-figure descriptions, render the pages that
   `layout.pages_with_tables`/caption text point at — never render all pages of a
   long report "just in case".
6. Pixel retrieval (PixelRAG-style, index → cosine top-k → VLM reads the tile):
   `webrain_vision` `op:index` → `op:retrieve`. Indexes persist per name; the
   embedder is whatever `EMBED_URL`/`EMBED_MODEL` points at, so point it at a
   vision-embedding model (e.g. `Qwen/Qwen3-VL-Embedding-2B`) for pixel search.

## Anti-bot / access
- A public PDF (`arxiv.org/pdf/…`, `…?download=1`) needs no identity: use the
  URL directly, or `webrain_download` `{engine:"http"}` to keep a local copy.
- `403`/`401`/consent wall on the PDF means the *document* is gated, not the
  text layer. Establish a session first (`webrain launch` / `webrain_session`
  `op:login`), then either fetch the URL with that browser or download the file
  from the authenticated tab and extract the local path.
- A challenge/interstitial body is not a PDF: the extractor will error. Do not
  report its text as the document.

## Pitfalls that actually bite
- `has_encoding_issues: true` → missing ToUnicode maps. Expect ligature damage
  (`ff`/`fi`/`fl` become `U+FFFF`, e.g. `o￿ces`). Quote around it or render that
  page; a pdfium build fixes the text layer.
- `images: []` is **not** proof the PDF has no figures. Vector art lives in `Form`
  XObjects with no `/Subtype /Image` at all (measured: 3 of 5 arXiv papers have
  zero rasters, only vector figures), and JPEG2000/CCITT/JBIG2 are still skipped.
  Use `op:render` when you need to *see* them.
- `images: []` on a paper whose appendix is made of *included pages* used to be a bug,
  not a fact: images one level down inside a `Form` XObject's own `/Resources` were
  never visited, so arXiv 2206.01062 reported **0** while holding **37**. Fixed — that
  file now yields 29 PNGs, and the count matching each figure's panel count (8 for
  Figure 4, 6 for Figure 6) is the sanity check that you got them all.
- `page` in an `op:images` entry means *the page whose `/Resources` expose this
  XObject*, not *the page that draws it*. A resource dictionary can be shared, so a
  group can be attributed a page where it is not visible — verify against the rendered
  page before writing "figure on page N".
- Appendix pages that are images of pages extract as interleaved garbage (two
  columns spliced mid-sentence). Detect it (`markdown` reads like two texts
  shuffled together) and switch to `op:render` + vision.
- `markdown` is the whole document (a 9-page report ≈ 48 KB). Extract once, then
  read the section you need — do not call `op:extract` per page.
- `op:images` and `op:render` inline base64 PNGs when `dir` is absent. Always pass
  `dir`; without it a 29-image paper is ~20 MB of tool result and a 9-page render is
  15 MB. Also pass `pages:[]` rather than pulling a 100-page scan into context.
- A per-provider vision error is worth reading in full: the chain now reports **every**
  failure, not just the last. `openrouter: 402 (out of credits)` and `groq:
  model_not_found` in one message means two different fixes; a single 404 used to hide
  both. No local model in the list means `webrain install vision` has not been run.

## Done check
- `results[].error` absent for every input.
- `pdf_type`/`needs_ocr` justify the method you used.
- Every number or figure you quote traces to a page you actually read
  (`texts[n]` or a rendered page), not to a garbled table row.
- For a batch: `count` matches the number of inputs and no result is `{error}`.
