---
name: pdf-extraction
description: 'Extract text, tables, figures and page images from PDFs — one file, many files, or URLs — with webrain_pdf and webrain_batch op=pdf. Use when asked to read/summarise a paper or report, convert PDF to Markdown, pull tables out of a PDF, OCR a scanned PDF, describe figures/charts, or extract a batch of PDF links found by a crawl. Covers arxiv.org/pdf, ?download= links, login-gated PDFs, and when to switch from text extraction to render + vision.'
---

# PDF extraction (single file, batch, and URL)

Zero system dependencies for text. The parse is **pure Rust** (`pdf-inspector` on
`lopdf`), so it needs no browser, no Python, no poppler — which also means a PDF
never needs a CDP tab and never hits anti-bot.

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
| Embedded raster images | `webrain_pdf` → `op:images`, `path:`, optional `pages:[]` |
| Page PNGs / tiles for vision | `webrain_pdf` → `op:render`, `path:`, `pages:[]`, `dpi:`, `tile_size:` |
| Print the **open browser page** to PDF | `webrain_pdf` → `op:page` (base64 PDF) |

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

1. `webrain_pdf` `{op:"render", path, pages:[3], dpi:110}` → base64 PNG(s).
   Requires a `--features pdfium` build with `pdfium.dll`/`libpdfium.so` on PATH.
   If it returns the "needs a build with --features pdfium" error, that is a
   capability limit, not a broken PDF: fall back to `op:images`, or open the PDF
   in real Chrome and screenshot it.
2. **`webrain_vision` reads browser pixels, not files.** A rendered page becomes
   readable only after you decode it to a PNG and navigate to it
   (`webrain_navigate` → `file:///…/p1.png`; the tool then reports
   `crippled:true` and empty `text` — that is normal, the pixels are there).
   Never aim vision at the PDF itself: Chrome's PDF viewer at a 764×485 headless
   viewport is so small the model invents a completely different document.
3. Ask with a clip **larger than the viewport**, plus `scale:3`:
   `{op:"ask", x:0, y:0, w:850, h:420, scale:3, prompt:"Transcribe: TITLE and
   AUTHORS only. UNREADABLE if illegible."}`. CDP captures from the surface, so
   the clip need not fit on screen. Measured on 5 arXiv papers: 5/5 correct with
   the clip + scale, 0/5 without. Past page 1, clip the **whole sheet**
   (`w:935, h:1210` at `dpi:110`) at `scale:2` — that read page 2's three section
   headings and page 3's `Figure 1` caption exactly, so one ask covers a page.
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
- Appendix pages that are images of pages extract as interleaved garbage (two
  columns spliced mid-sentence). Detect it (`markdown` reads like two texts
  shuffled together) and switch to `op:render` + vision.
- `markdown` is the whole document (a 9-page report ≈ 48 KB). Extract once, then
  read the section you need — do not call `op:extract` per page.
- `op:images` inlines base64 PNGs into the tool result; ask for `pages:[]` rather
  than pulling a 100-page scan into context.

## Done check
- `results[].error` absent for every input.
- `pdf_type`/`needs_ocr` justify the method you used.
- Every number or figure you quote traces to a page you actually read
  (`texts[n]` or a rendered page), not to a garbled table row.
- For a batch: `count` matches the number of inputs and no result is `{error}`.
