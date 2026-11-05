# Addendum (2026-11-05): the page is verified, not embedded

The decision above says the page is `include_str!`'d into the binary. It is not,
any more. The reasoning above still holds — one process, no CDN, no second
binary, the page still cannot be replaced without the binary agreeing — but the
mechanism changed, so this records why and how.

**What changed.** `webrain chat --serve` now reads `chat.html`, `chat.css`,
`chat.js` and `chunk-markdown.js` from a directory at startup, in this order:
`WEBRAIN_CHAT_UI`, the engine cache (`webrain install chat-ui`), a
`webrain-chat-ui/` folder beside the binary, and `webrain-cli/assets/` in a
checkout. `build.rs` hashes the four files at compile time and the server
verifies every candidate against those digests before serving a byte. A file
that does not match is refused by name, with both digests.

**Why the mechanism changed**, in order of weight:

1. **The page is the one artifact that changes without a Rust change.** A theme,
   a layout fix, a copy edit is a `bun run build` — not a 2 000-second CI matrix
   and eight new platform binaries. Keeping it a file makes that a real option.
2. **Size, honestly: 272 KB.** Measured on `x86_64-pc-windows-msvc`, 47 211 008 →
   46 932 480 bytes (0.6 %). The UI was never the bloat — the Copilot runtime was
   (166 MB) and is already resolved at run time. Anyone expecting this to shrink
   the download meaningfully should read that number first.
3. The four files travel as one `webrain-chat-ui.tar.gz` (87 677 bytes gzipped),
   so a release carries the page once instead of eight times.

**Why the security property survives.** The page can ask a human to approve a
shell command, so "which bytes are being served" is the whole question. A digest
compiled into the binary answers it for any directory, including one an attacker
can write to: the server would rather refuse to start than serve a `chat.js`
nobody built. `install chat-ui` verifies into a staging directory and only then
swaps it in, so a bad archive leaves the working install alone.
