# Veo 3 — hero-terminal.mp4 (prepared 2026-08-07)

## Two frames (image-to-video)
- **First frame:** `hero-terminal.png` (680x500) — idle start: prompt empty, no logs, no JSON card, cursor blinking
- **Last frame:** `hero-terminal-end.png` (680x500) — completed end: command typed, tool logs green, JSON card glowing green border, `✓ 42 products scraped in 1.4s`, sidebar items marked used

Sources: `hero-terminal.svg` / `hero-terminal-end.svg` (same design language, dark-tech, electric blue + green).

## Structured prompt
`hero-terminal-veo.json` — full Veo structure (title, technical specs, artistic direction, camera control, main text prompt, 5-beat timeline). Use its `main_text_prompt` for a text-only call.

## Runnables
- Image-to-video: pass both PNGs + `main_text_prompt` from the JSON.
- Text-only: paste `main_text_prompt` into `mcp_mcp_veo_3_vid_generate_video`.

---

## Original prompt (one shot, do not iterate — usage is limited)

Locked static camera, zero camera movement. A dark premium developer terminal
app window centered on a near-black background. The window has a title bar with
red, yellow and green macOS dots and monospace text 'webrain mcp'. Left sidebar
titled 'TOOLS' lists navigate, observe, interact, extract, crawl, watch,
session, vision in monospace, with 'navigate' and 'extract' highlighted in
electric blue. Main terminal area shows a '$ webrain mcp' line, then 'Starting
MCP server on stdio... 16 tools loaded', then an 'agent>' prompt where the
command 'scrape product prices from URL X' types itself in character by
character; then three tool log lines appear one by one each with a green
success tag: 'webrain_navigate' with green '→ ok', 'webrain_extract
(mode=autoschema)' with green '→ schema', 'webrain_extract (mode=schema)' with
green '→ 42 rows'. A rounded JSON result card fades in showing a product title
'USB-C Hub', a price of $42.90, and a green true value. The blue block cursor
blinks steadily. Electric blue and green accents on deep near-black, crisp
monospace text, clean and premium, no watermark, no style change.

## Motion beats (what makes it "live", in order)
1. Blue block cursor blinks (throughout)
2. `agent> scrape product prices from URL X` types in char-by-char
3. Tool logs resolve one by one: → ok, → schema, → 42 rows (green)
4. JSON result card fades in
5. Soft electric-blue glow pulses behind the window
