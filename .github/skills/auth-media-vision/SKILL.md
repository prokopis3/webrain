---
name: auth-media-vision
version: "1.0.0"
description: Capture authenticated social-feed media (videos, images, audio) from a logged-in browser and actually understand it — frames AND speech — with the webrain MCP tools. Use when asked to scrape a feed/gallery behind a login, download posts, watch videos, or "use vision to understand" a batch of media. Read the frames webrain_watch already extracted (montage only as fallback). Covers the traps that silently produce useless output: empty audio track, missing STT, and per-item vision calls that take 20 round-trips.
argument-hint: "extract top <N> posts from <site/feed> and describe them"
allowed-tools: Bash, Read, Write
---

# /auth-media-vision

One pipeline, six steps. Every step below was verified end-to-end; the notes are
the failures, not theory.

**Prereqs.** A real Chrome with a logged-in profile reachable over CDP
(`--port 9222`), plus an exported cookie jar (`webrain cookies --port 9222 --out out/auth.json`).

---

## Step 1 — harvest the post list (one call)

`webrain_batch` `op=interact`, `concurrency=1`, `cdp_urls=["http://127.0.0.1:9222"]`.
The interaction scrolls and collects. Keep it short: background tabs throttle
timers to ~1/s, long `await`s can resolve null.

```js
(async () => {
  const seen = [];
  const grab = () => document.querySelectorAll('article').forEach(a => {
    const link = a.querySelector('a[href^="/p/"], a[href^="/reel/"]');
    if (!link) return;                                  // ads have no post URL
    const href = link.getAttribute('href');
    if (seen.some(x => x.href === href)) return;
    seen.push({ href, type: a.querySelector('video') ? 'video' : 'image',
                caption: (a.innerText || '').replace(/\s+/g, ' ').slice(0, 120) });
  });
  for (let i = 0; i < 16 && seen.length < 12; i++) {
    grab(); window.scrollBy(0, Math.round(window.innerHeight * 0.8));
    await new Promise(r => setTimeout(r, 800));
  }
  grab();
  return JSON.stringify({ count: seen.length, items: seen.slice(0, 12) });
})()
```

**Trap 1 — profile pictures.** `img[src*="cdninstagram"]` matches avatars (`s150x150`).
Filter `naturalWidth > 200` and keep the largest, or you will vision a 150px blob.

**Trap 2 — sponsored posts.** Give them `href: null` and skip them; there is no
post page to open.

## Step 2 — capture the real media URLs (one batch call)

Post pages expose **only `og:image`** — no `og:video`. The `<video>` element holds a
`blob:` MediaSource URL, not a fetchable file. The mp4 exists only in the page's
network traffic:

```js
(async () => {
  const t0 = Date.now();                                  // Trap 4: at readyState:complete the
  let v = null;                                           // <video> is often not mounted yet
  while (Date.now() - t0 < 9000 && !(v = document.querySelector('video'))) {
    await new Promise(r => setTimeout(r, 250));           // background tabs throttle to ~1/s
  }
  if (!v) return JSON.stringify('NO_VIDEO');
  v.muted = false; v.volume = 1;                          // MUST be unmuted, see Trap 3
  try { await v.play(); } catch (e) {}
  await new Promise(r => setTimeout(r, 3500));

  const es = [...new Set(performance.getEntriesByType('resource').map(e => e.name)
    .filter(n => /\/o1\/v\/.*\.mp4/i.test(n))
    .map(u => u.split('&bytestart')[0].split('&byteend')[0]))];   // DASH range params
  return JSON.stringify(es.sort((a, b) =>
    (b.indexOf('720p') > -1 ? 1 : 0) - (a.indexOf('720p') > -1 ? 1 : 0))[0] || 'NONE');
})()
```

Return **one** URL per tab. Returning several blows the tool's inline limit and
dumps the payload to a file you then have to read back.

**Trap 3 — muted playback = no audio track, forever.** Setting `v.muted = true`
(what autoplay needs) means the browser never requests the audio representation,
so the only mp4 you can capture is **video-only** (`ffprobe` shows one `vp9` stream).
The fix is order-dependent:

1. `webrain_interact` `action=click` on the player's mute control — a **trusted**
   click, so unmuting is allowed.
2. Then in JS: `v.muted = false; v.volume = 1; await v.play()`.
3. Audio segments now appear as **`/o1/v/t2/f2/m78/…mp4`** — decode the `efg` param
   (base64 JSON): `dash_ln_heaac_vbr3_audio`. Its `xpv_asset_id` matches the
   video's, which is how you pick the right track from the preloaded next-reel.

## Step 3 — download (webrain_download, engine=http)

`webrain_download` with `dir`, `engine: "http"`, `filter_extension: ".mp4"`, all URLs at once.
Signed CDN URLs work **without cookies** — but pass the jar if a site does require it.

## Step 4 — mux, and prove it

```bash
ffmpeg -y -v error -i vid.mp4 -i aud.mp4 -c copy -movflags +faststart full.mp4
ffprobe -v error -show_entries stream=index,codec_type,codec_name,duration -of csv=p=0 full.mp4
# want exactly:   0,vp9,video,59.8   1,aac,audio,59.9
```

No `aac` line means you captured a silent file — go back to Trap 3.

## Step 5 — vision: read the frames `watch` already made

**Rule: never build frames yourself if `watch` already did.** `webrain_watch`
returns `frames: [{index, t, path}]` next to the transcript — 17 for an 8 s clip,
20–100 for longer (see `Detail`). Those files are on disk, full resolution, already
ordered by time. Open **3–5 of them** with your image-viewing tool:

```
frame_0001.jpg   t≈0.0      # but see the black-first-frame note below
frame_0005.jpg             # successive frames if the clip is short
frame_0009.jpg   middle
frame_0013.jpg
frame_0017.jpg   end
```

That is the whole step. No ffmpeg, no HTML page, no navigate.

**Reading beats sampling:** pick frames where the *content* changes, not by even
spacing. Start / first visible action / middle / last is usually enough for a short
reel; for a long video open frames around the transcript timestamps instead of a
uniform grid.

> **`frame_0001.jpg` is often black.** It is `t=0.0`, before the first painted frame.
> If a frame is black, open the next one — do not conclude "the video is black".

### Fallback: montage — only in these three cases

1. The media never went through `watch` (captured by hand with `webrain_download`),
   so no frames exist on disk.
2. Your tool set has **no direct image viewer** — `webrain_vision ask` reads *the
   current page's pixels*, so local files must be rendered into a page first.
3. You need **one call for N videos** and a one-sentence answer per video suffices.

```bash
# 4 evenly spaced frames per video, tiled 2x2  -> m0.jpg … m9.jpg
for f in *.mp4; do d=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$f");
  ffmpeg -y -v error -i "$f" -vf "fps=$(echo "scale=6;4/$d"|bc),scale=420:420:force_original_aspect_ratio=decrease,pad=420:420:(ow-iw)/2:(oh-ih)/2,tile=2x2" \
    -frames:v 1 "m${f:0:1}.jpg"; done
```

Then a throwaway HTML page: blocks of **2 montages**, one `id="b0..bN"` per block,
each block exactly viewport-height, images 420×420. Navigate to
`file:///…/vsheet.html#b3` and ask with two tiles at `(10,10,420,420)` and
`(440,10,420,420)`:

> Image 1 is a 2x2 grid of four frames (reading order) from video A. Image 2 is the
> same for video B. Reply only: 'A: <one sentence>' and 'B: <one sentence>'. No reasoning.

Cost of the fallback: 4 frames downscaled to 420×420 each, plus an HTML file, a
navigate and a vision call — to recover information the primary path already had.
Use it when you must, not by default.

> **Fallback only — the last block is NOT at y=0.** An anchor whose block sits closer to the end than one
> viewport overshoots: the page stops at `height - viewport`, so `#b4` of a 5×440 px sheet
> lands at `y = 1760 - (2200 - 925) = 485`, and tiles at `y=10` silently re-read block b3.
> Cheap check: ask a block you already know and confirm the answer changed.

**Prompt hygiene:** this local model dumps chain-of-thought unless told
`Answer only, no reasoning` — otherwise you get its draft paragraphs plus a
truncated answer.

**Call budget, 10 videos:** watch-already-ran + 3 frames each = ~30 `view_image`
calls but **zero** extra processing, full resolution, and you can stop early once a
post is obvious. Fallback sheet = 1 ffmpeg loop + 1 HTML + 5 navigates + 5 asks, at
420×420 and 4 frames per video. Per-item navigate+ask = 20 vision calls for worse
output than either.

## Step 6 — speech, if it matters

`webrain_watch` takes a **local path or a URL** and returns frames + `transcript`.
Point it straight at a public reel/post URL when you only need one item — no capture,
no mux, no manual frames.

- `has_audio: false` → Step 4 didn't run or the capture was muted.
- `transcript_source: "none"` with `has_audio: true` → the STT backend is missing.
  Needs `whisper-cli` on PATH **and** a model: `webrain install whisper --model small`
  (use a multilingual model, not `*.en`, for non-English audio).

**Fixed bug worth knowing:** `video.rs::default_whisper_model()` used to hardcode
`ggml-small.en.bin`, so any other `--model` left it pointing at a file that did not
exist and `watch` reported "no transcript" with no error. It now falls back to
whatever `ggml-*.bin` is on disk, multilingual first.

**But `transcript_source:"none"` is not proof of that bug — retry once first.**
Measured twice on the same files, same binary: `full3.mp4` returned
`transcript: []` / `"none"` / `ms: 4236`, and the identical call a moment later
returned 5 segments / `"local"` / `ms: 7661`. A correct 26 s transcription cannot
finish in 4 s, so the call bailed early rather than failing to find a model — the
first whisper invocation per work dir is unreliable. Before touching the binary,
model path or `WEBRAIN_WHISPER_MODEL`: call `watch` again. Only a *repeatable*
`"none"` means the STT backend is actually missing (no `whisper-cli` on PATH, or no
`ggml-*.bin` under the browsers dir).

---

## Batch over N posts (the fast path)

Four calls for ten posts, not forty.

1. **Post URLs** — one `batch op=interact` scroll+collect (Step 1).
2. **Video + audio URLs** — one `batch op=interact` running the Step 2 JS. Return **one**
   URL per tab; an array of URLs blows the inline limit and dumps the payload to a file
   you then have to read back.
3. **Download** — two `webrain_download` calls: videos into `<out>/`, audio into `<out>/aud/`.
4. **Mux + transcribe** — one shell loop each, no per-file tool round-trip.

**Index-align the downloads or you will mux two unrelated tracks.** CDN filenames are
opaque hashes (`AQOEs18…mp4`) and `webrain_download` prefixes each with its batch index,
so that index is the *only* join key: submit audio URLs in the same order as video URLs.
Keep audio in a subdirectory so a `*.mp4` glob cannot sweep up both kinds.

```bash
# mux: i_*.mp4 (video-only) + aud/i_*.mp4 -> full<i>.mp4
for f in out/[0-9]_*.mp4; do i=${f:0:1}; ffmpeg -y -v error -i "$f" -i out/aud/${i}_*.mp4 \
  -c copy -movflags +faststart out/full$i.mp4; done
# transcribe: whisper-cli wants 16 kHz mono WAV, not mp4
for i in $(seq 0 9); do ffmpeg -y -v error -i out/full$i.mp4 -ar 16000 -ac 1 -c:a pcm_s16le out/txt/f$i.wav
  whisper-cli -m <ggml-model.bin> -f out/txt/f$i.wav -np -oj -of out/txt/f$i; done
```

A stray audio file left in the *video* directory muxes audio-with-audio and reports a
1-stream "success" — the mux loop is the second place index alignment fails silently.

### Order the passes, or you wait on whisper for nothing

STT is the slow step (~2x realtime for `small` on CPU); frames and mux are instant.
Doing them in one loop serialises vision behind whisper, so the first visual answer
only lands after the first transcript. Split it:

1. mux for **all** items (seconds, no model), then
2. fire all the `watch` calls, then
3. open each item's `frames/` while the later transcripts grind.

### Reading the last block of a tall sheet (fallback path only)

`navigate url#bN` only scrolls if the document is taller than the viewport; at the
bottom the anchor silently does nothing and you read the wrong montage. Do not guess
the offset and do not trust a plain `navigate` — **probe the page first**, one call:

```js
// webrain_batch op=eval, url = sheet.html?v=3#bN   (bust the cache: see below)
(() => { const im=[...document.images].map(i=>({f:(i.currentSrc||'').split('/').pop(),w:i.naturalWidth}));
  return JSON.stringify({scrollY:Math.round(window.scrollY), vh:window.innerHeight,
    total:im.length, loaded:im.filter(x=>x.w>0)}); })()
```

Verified output: `scrollY:835, vh:925, loaded:[m0..m6 all 840x840]`. Then
`tile_y = blockIndex*440 - scrollY + 10` → block 3 sits at **495**, not 10.
`loaded.length < total` means a montage did not decode and that block will read as
pure black — fix it before asking.

If the sheet HTML was loaded while some montages did not exist yet, Chrome keeps the
failed image requests: those blocks render black on every later visit. Re-navigate
with a cache-busting query (`vsheet.html?v=3#b3`) and re-ask.

## Verification checklist (do not skip — all three fail silently)

| Check | Command | Good |
|---|---|---|
| Video has audio | `ffprobe -show_entries stream=codec_type` | `video` **and** `audio` |
| Transcript ran | watch JSON | `transcript_source: "local"` (or a cloud backend) — `"none"` means retry once |
| Frames exist | watch JSON | `frames[].length >= 10`, files on disk |
| Visuals actually read | your own report | names subjects **and** what changes; not "an Instagram post" |

## Rule 0

Never report "understood the videos" from cover images alone. A silent capture and a
missing STT model both look like success — they are `has_audio` and
`transcript_source` lying to you by omission.

The inverse holds too: never report *visuals* from the transcript alone. A song's
lyrics transcribe perfectly and describe nothing on screen. Frames come from `watch`,
read 3–5 of them, then speak.
