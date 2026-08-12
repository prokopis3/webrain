# /captcha-solve — solve ANY CAPTCHA with webrain vision

Provider-agnostic. Works for image grids (reCAPTCHA / hCaptcha / Turnstile),
drag-and-drop / slider puzzles, checkbox widgets, and any visual challenge.
Vision INTERPRETS the challenge and picks the ACTION (click / drag / type);
**the token is the ground truth** — never report solved without it.

## Vision provider chain (cloud-first, auto)
`OPENROUTER_API_KEY` → `OPENAI_API_KEY` → `FIREWORKS_API_KEY` → `GROQ_API_KEY`
→ bundled local Qwen3-VL-2B (`webrain install vision`). A 27B cloud model
classifies tiles the local 2B misreads (it echoes templates / lists every
tile) — set ANY cloud key for accurate solving; local is the offline fallback.

## Prereqs
- Real Chrome on CDP: `webrain_session(op=open, cdp_url="http://127.0.0.1:9222")`.
- Vision backend: one of `OPENROUTER_API_KEY` / `OPENAI_API_KEY` /
  `FIREWORKS_API_KEY` / `GROQ_API_KEY`, or `webrain install vision` (local).

## The one true flow
This generic tool flow (proven live) solves ANY challenge — there is no
monolithic solver tool. Each step is a plain webrain tool; an LLM agent drives
the loop, and the token is the only ground truth.

## Step 0 — Ground truth token (check FIRST and after every step)
`webrain_eval`:
```js
(function(){ var t=document.querySelector('textarea[name="cf-turnstile-response"],textarea[name="g-recaptcha-response"],textarea[name="h-captcha-response"]'); return t?t.value:''; })()
```
Non-empty → **solved, stop**. This field is the ONLY proof. Vision "looks
done" is never proof.

## Step 1 — Open the challenge
A checkbox-style widget is the entry to most challenges (reCAPTCHA /
hCaptcha / Turnstile). Claim it first:
1. `webrain_eval` → rect of the small widget iframe (`iframe[src*="anchor"],
   iframe[src*="recaptcha"], iframe[src*="hcaptcha"],
   iframe[src*="challenges.cloudflare"]`).
2. The checkbox is a ~28px box at the iframe's **top-left**, offset ≈
   `(x+27, y+37)` — NOT the center (center is the label text).
3. Trusted click: `webrain_interact click_coords x y` (webrain sends
   `mouseMoved` first so the widget registers it).
4. Wait 3-4s → token (Step 0) or the challenge popup opens.

## Step 2 — Interpret the challenge with vision (ONE scaled shot)
`webrain_vision op=ask` on the puzzle frame rect (clip x,y,w,h) **with
`scale: 2`** (see Step 3 for why 2, not 3). Ask for the ACTION + NUMBERS:
> How to solve this CAPTCHA? Reply: TYPE + the exact instruction + the action.
> TYPE: CHECKBOX (tick a box) | GRID (select squares with X) | TEXT/ASSEMBLE
> (type a code / assemble 2 tiles to match a reference) | DRAG (drag a
> piece/slider to a target) | OTHER. For GRID give the OBJECT WORD + tile
> NUMBERS (1-9 row-major). For DRAG give handle + target as (x,y).
Ask for NUMBERS / the object WORD — never raw (x,y) pixel estimates; the model
hallucinates coordinates (it invented a 477px image size in a live run).

## Step 3 — Act by type
### CHECKBOX
`webrain_interact click_coords(center)`; poll token (Step 0).

### GRID (select all squares with X)
1. **One batched, upscaled call (the tuned payload):** `op=ask` with the
   `tiles` param — each grid cell as one clip, `scale: 2`, ALL in ONE request:
```
op=ask, scale: 2, tiles:[9 cell clips], prompt:
"Google reCAPTCHA puzzle. These 9 numbered images are the 3x3 grid tiles in
row-major order (1=top-left ... 9=bottom-right). The header says which object
to select. Decide for EACH tile YES or NO whether it contains that object.
Reply ONLY: OBJECT: <object> | 1:YES | 2:NO | ... | 9:NO"
```
   - **Why `scale: 2` (≈224px), not 3:** VLMs resize input to ~224-384px
     internally — scaling a 112px crop to 336px only interpolates pixels the
     model can't see, ~2× tokens/bytes for zero accuracy gain. 224px is the
     sweet spot (~25-45KB PNG/tile → ~300KB for 9, ~half of scale 3). Use
     `scale: 3` only for the weak local 2B fallback.
   - If the provider errors "Too many images" (Groq caps at 3), split into
     batches of ≤3 tiles.
2. **Click coordinates come from GEOMETRY, not the model's pixels.** Read the
   exact grid + verify rects from inside the cross-origin frame with
   `webrain_eval_in_frame` (the only tool that crosses the origin boundary):
```
webrain_eval_in_frame { url_contains: "bframe", js: <puzzle-geometry JS below> }
```
```js
(function(){ var out={target:'',tiles:[],verify:null};
var s=document.querySelector('strong,h1,h2,h3,[role="heading"],.rc-imageselect-instructions');
if(s) out.target=(s.innerText||s.textContent||'').trim();
var seen=[]; document.querySelectorAll('img').forEach(function(im){ var r=im.getBoundingClientRect();
if(r.width<30||r.height<30||r.width>400||r.height>400) return; if(r.top<0||r.left<0) return;
var dup=false; for(var k=0;k<seen.length;k++){ if(Math.abs(seen[k].x-r.x)<8&&Math.abs(seen[k].y-r.y)<8){dup=true;break;} }
if(!dup) seen.push({x:Math.round(r.x),y:Math.round(r.y),w:Math.round(r.width),h:Math.round(r.height)}); });
seen.sort(function(a,b){return (a.y-b.y)||(a.x-b.x);}); out.tiles=seen.slice(0,9);
document.querySelectorAll('button').forEach(function(btn){ var t=(btn.innerText||'').trim();
if(/^(verify|submit|continue|done)/i.test(t)){ var v=btn.getBoundingClientRect();
out.verify={x:Math.round(v.x),y:Math.round(v.y),w:Math.round(v.width),h:Math.round(v.height)}; }});
return JSON.stringify(out); })()
```
   Coords are **iframe-relative** — add the iframe's viewport origin (from
   `webrain_eval` on `iframe[src*="bframe"]`) to each tile/verify rect, then
   center = x + w/2, y + h/2. Fallback (no iframe match): grid top = first
   photo row below the header, cells fill the frame ~120-135px; ±10px is fine.
3. **Click matches in PARALLEL** — fire `webrain_interact click_coords` for
   every matching tile in one message (beats expiry).
4. Verify: locate the submit button — reCAPTCHA = solid blue band in the
   puzzle's bottom strip (live run: ~x408-507 / y552-593), or isolated-world
   geometry — click it, poll the token. Fresh puzzle on fail → loop.

### TEXT / ASSEMBLE-CODE (2captcha xcaptcha — "assemble from 2 elements the same code as shown")
A reference code + 8 tiles; pick the 2 tiles that concatenate (in click order)
to the reference. The chars are CSS `background-image` data-URLs in a
**same-origin** iframe — extract deterministically, then OCR expiry-immune:
1. **Extract the sprite (one eval, immune to expiry):** all cells + the
   reference share ONE sprite PNG shown at different `background-position`.
   Reach into the same-origin iframe (`iframe[src*="api.xcaptcha"]` →
   `contentDocument`) and read the shared `data:image/png;base64,` URL + each
   element's `background-position`/rect via `webrain_eval`. Decode + crop each
   region to files (System.Drawing) — exact geometry, no screenshot noise.
2. **OCR single-image, NOT batched:** Qwen3 dumps a long `<think>` that
   truncates the multi-image answer in the tool result and it hallucinates
   distorted chars (live: reference read as "QVZRSG" then "QVZPSE"). Read
   each cropped image with its OWN `op=ask` (scale 2-3, prompt "Reply ONLY the
   characters") — short, clean, reliable. Re-read any tile that looks off
   (G/6, 1/I/Z/2 confusion).
3. **Match** the 2 tiles that concatenate exactly to the reference (e.g. REF
   `h7uH5q` = tile `h7u` + tile `H5q`).
4. **Click them IN ORDER** — first click = START of the code, second = END.
   Wrong order = tiles look selected + Confirm shows but NO token (puzzle
   resets silently). Then click Confirm, poll the token (Step 0).

### DRAG / SLIDER (drag the piece / slide to the gap)
1. Locate the draggable handle and the drop target: `op=ask` ("handle center
   and target center as (x,y)") or the isolated-world DOM probe.
2. Trusted drag: `webrain_interact drag x1 y1 x2 y2` — CDP press at the
   handle, move with the button held to the target, release. Crosses
   cross-origin iframes like clicks.
3. Poll token; if the piece snaps back, re-locate and drag again (fresh
   coords), bounded rounds.

### OTHER
Do what the vision says: type an answer, click a control, wait. Poll token.

## Step 4 — Expiry / loop
- Token set → **solved**.
- Token empty after a puzzle → the challenge expired (back to checkbox) or a
  fresh puzzle appeared. Loop Steps 1-3 until the token sets or rounds exhaust.
- Expired → re-click the checkbox (Step 2) automatically; don't restart the
  page.

## Anti-patterns
- **Treating every challenge as checkbox/grid** — vision interprets the TYPE
  first (Step 2); drag/slider challenges need the trusted drag action
  (`webrain_interact drag`), not clicks.
- **Asking the model for raw pixel coordinates** — ask for NUMBERS (row-major
  tiles) or explicit handle/target coords; raw (x,y) estimates are unreliable.
- **Never use `webrain_batch` for the clicks** — its `interact` is JS-only
  (`eval_session`), so it CANNOT trusted-click the cross-origin widget; use
  `webrain_interact click_coords` (or `webrain_eval_in_frame` to read exact
  widget geometry first).
- **Trusting `webrain_vision op=index` to identify puzzle tiles** — index tiles
  the whole PAGE (misaligned + slow); use `op=ask` on the puzzle frame.
- **Cropping/classifying tiny raw tiles** — a ~100px crop is too small for the
  2B model (it misread a bicycle tile in a real run). One-shot the whole frame
  first; if that's wrong, crop + UPSCALE 2-3× before per-tile `op=ask`.
- **Clicking the checkbox iframe center** — that's the text; the box is the
  28px top-left corner (~x+27,y+37).
- **Reporting solved from a screenshot** — the token, always.
- **Sequential clicks** — fire tile clicks in parallel; a slow sequence lets
  the challenge expire mid-flow.
- **Loops without rounds** — bound the loop; report timeout, don't spin.
- **Blind trust in vision** — if verify fails repeatedly, the model may be
  misclassifying; re-read exact geometry with `webrain_eval_in_frame`, re-ask
  `op=ask` with fresh tiles, or use a stronger vision model.
- **Trusting the model's raw (x,y) pixel estimates** — it hallucinates sizes
  (invented a 477px image in a live run); ask for tile NUMBERS, map to centers
  via geometry.
- **Multi-image OCR for text/assemble puzzles** — Qwen3's think-dump truncates
  the answer in the tool result and misreads distorted chars; read each tile
  as its OWN single-image `op=ask`.
- **Clicking assemble-code tiles in any order** — first click = START of the
  code; wrong order = silent reset (no token) even though tiles look selected.
- **Screenshot-OCR'ing text puzzles under time pressure** — extract the
  data-URL sprite once (same-origin iframe) and crop to files; then OCR at
  leisure (expiry-proof).
- **Vision shots at `scale: 1`** — small tiles get misread; use `scale: 2`.
- **`scale: 3` on a cloud model** — ~2× tokens/bytes for no gain (models
  resize to ~224-384px internally); reserve 3× for the local 2B.
- **Re-claiming an already-open puzzle** — after the checkbox click, poll for
  the popup before re-clicking (re-click dismisses it).
- **>3 images per request on Groq** — it caps at 3; batch tiles ≤3 per call.
