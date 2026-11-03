---
name: page-tiles
description: 'Read a page as pixels when text extraction is not enough — "screenshot-tiles <url> and summarise", "look at this page", "what does the chart/table show", "the extract is empty or garbled". Covers tiling (observe what=pixel), saving tiles to disk, and reading them with the local vision model in one batched call. Use when content is visual (charts, figures, CAPTCHA geometry, image-only pages) — not for ordinary text.'
---

# Page tiles (screenshot -> disk -> one vision call)

Three tools, one order. The expensive thing in a tile workflow is not the browser,
it is **bytes in the tool result**: a 660 KB PNG page becomes ~10 MB of base64 across
16 tiles. So the rule is: **tiles go to disk, the model gets paths, the answer comes back.**

## Decision rule — text before pixels

| Ask | Do this |
|---|---|
| "summarise this page / article / thread" | `webrain_extract` or `observe{what:"fit"}`. Text is ~2 KB; tiles are ~10 MB. |
| "what does the chart / table / figure show" | tiles |
| "the extract is empty / garbled / interleaved" | tiles (image-only page, or two columns spliced) |
| CAPTCHA / grid geometry | `webrain_vision{op:"ask", tiles:[...]}` (rects, not files) |
| PDF page | `webrain_pdf{op:"render"}` — see the pdf-extraction skill. Text first: a 9-page paper is 48 KB of markdown in ~5 s, while vision costs ~16 s **per page** and returns ~half the text. Tiles earn their keep only on the pages the text layer returns empty (measured: page 7 gave 0 chars, vision read 2133). |

`news.ycombinator.com` and `arxiv.org/abs/...` are **text** pages. Tiling them wastes
a VLM pass to re-read text that `observe{what:"fit"}` already returns.

## The recipe

```
1. webrain_navigate {url}
   -> check `challenge` is null first; a block page is not content.

2. webrain_observe {what:"pixel", tile_width:900, tile_height:1200,
                    max_tiles:8, tiles_dir:"output/tiles/<name>"}
   -> {count, tiles:[{index,x,y,width,height,path}]}   # NO base64 in the reply

3. webrain_vision {op:"ask", images:["output/tiles/<name>/tile_000.png", ...],
                   prompt:"..."}
   -> {answer}                                        # one llama call, all tiles
```

Pass **paths**, never `png_b64`. `images` reads from disk: no second capture, and the
tool call stays a few hundred bytes.

## Tile geometry

- Full width, tall strips: `tile_width:900, tile_height:1200`. Squares such as
  800x800 cut table columns and chart axes in half.
- `max_tiles` caps the run. 900x1200 covers ~2400 px of page per 2 tiles; a long
  article needs 4-6, a dashboard 2-3.
- Need the geometry before deciding? One `webrain_observe{what:"page_info"}` gives
  scroll/viewport size — do not `eval` for it.

## Prompt shape

- One or two fields per ask: `"Transcribe the title and authors only. UNREADABLE if illegible."`
- Never ask a small VLM to **count** — it answers with chain-of-thought and a wrong
  number. Ask for content, then count in your own code from the answer.
- Numbered prompts work in batch mode: tell the model "images are numbered 1..N".

## Traps

- **Vision reads browser pixels, not files.** `op:"ask"` cannot be pointed at a PDF or
  a URL; capture first (`observe what=pixel`, or `webrain_pdf{op:"render"}` + navigate
  to the PNG) and pass the pixels.
- **A `crippled:true` reply with empty `text` is normal** on an image document — the
  pixels are there, which is why tiles exist.
- **`tiles` (rects) is viewport-space, `what:"pixel"` writes document-space tiles.**
  Tiles produced by `observe` are for `images:[paths]`; only hand-written rects go to
  `tiles:[...]`.
- Nothing below the fold is missing: the capture reads the whole document surface, so
  there is no scroll step. A truly blank tile means the image never loaded
  (`loading="lazy"`), not that the tile is out of range.

## Done check

- `count` matches the page size you expected (not `max_tiles` on a page that is shorter).
- Every tile you pass to vision exists on disk (`path` returned by step 2).
- The summary quotes only content the model returned — no invented page text.
