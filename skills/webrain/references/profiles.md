# Profiles & Sessions (state model)

Browser identity, profile, and session are EXECUTION STATE. This page covers what
they are, when to create vs reuse, and how they carry authentication.

## Profile
- **What:** a persistent per-account browser profile
  (`WEBRAIN_PROFILES_DIR`, default `%APPDATA%/webrain/profiles/<service>/<profile>`),
  used as Chrome's `--user-data-dir`. It holds cookies (incl. HttpOnly session
  cookies, cf_clearance), localStorage, and site data.
- **Why:** a profile survives a Chrome restart. Reusing a profile restores the
  site's session/auth state — no re-login, no re-solving.
- **When to create:** first login for a service (`webrain launch <service> <profile> <url>`
  / `webrain_session(op=login)` with the vault).
- **When to reuse:** every subsequent task for that account. NEVER discard a
  working profile after a challenge.

## Session
- **What:** a live browser instance + its CDP connection, bound to a profile. In
  MCP, `webrain_session(op=open, cdp_url=...)` creates a named session pool that
  later tool calls route to via `session_id`.
- **Lifecycle:** CREATE → PROFILE ATTACHED → REAL CHROME → SESSION ACTIVE →
  NAVIGATE → PAGE STATE → HANDLE → EXTRACT → VERIFY → PRESERVE (keep the session
  for the next call; `webrain_session(op=close)` to release).
- **Reuse:** keep the session open across pages; re-attach an
  already-authenticated Chrome by pointing `cdp_url`/`CDP_URL` at its port.
- **Isolation:** different sessions/browsers have isolated cookie jars. Set
  cookies and batch on the SAME connection (`webrain_session(op=setcookies)` then
  `webrain_batch` without `cdp_urls`), or obscura's per-connection jar isolation
  bites you.

## Auth state transfer (cross-browser cookies)
1. Log in ONCE in real Chrome on a persistent profile (native login, vault+TOTP).
2. Export cookies on the LIVE browser: `webrain_session(op=cookies)` / CLI
   `webrain cookies --port 9222 --out file.json`. ⚠️ Session cookies do NOT
   survive a Chrome restart — export while Chrome is alive.
3. Import on the MCP session connection: `webrain_session(op=setcookies, ...)`.
4. Batch WITHOUT `cdp_urls` on that same connection so it shares the cookies.
5. Alternative portable state: `webrain_session(op=save_state)` /
   `webrain_session(op=restore_state)` (cookies + localStorage JSON per profile).

⚠️ **A vendor clearance does not transfer.** An anti-bot token (`cf_clearance`,
DataDome, `_abck`) is bound to the minting browser's TLS/JA4 fingerprint + UA + IP.
Importing it into another engine fails even with a byte-identical UA — verified
2026-11-01 on skroutz.gr: lightpanda (jar preloaded via `--cookie`, `--user-agent`
set to Chrome's) and obscura (`--user-agent` set, 4/4 cookies incl. `cf_clearance`,
46 s wait) both still got `Just a moment...` on every HTML page, while the same
Chrome that minted the cookie returned 200. So: transfer cookies for **login
state**, and batch Cloudflare-gated pages on the Chrome that owns the clearance
(`cdp_urls:["http://127.0.0.1:9222"]`). Do not diagnose this as a cookie bug.

## Engine identity — obscura / lightpanda profiles

`webrain launch` is Chrome-only; the other engines get identity from their own
binary flags:

| Engine | Start | Identity |
|---|---|---|
| obscura | `obscura serve --host 127.0.0.1 --port 9224 --stealth [--user-agent "<UA>"] [--storage-dir <dir>]` | `--storage-dir` = a real profile: writes `<dir>/cookies.json`, survives a server restart (verified: 5/5 cookies read back after kill + relaunch) |
| lightpanda | `lightpanda serve --host 0.0.0.0 --port 9222 [--user-agent "<UA>"] [--cookie jar.json] [--cookie-jar out.json]` | `--cookie` loads a jar read-only at boot, `--cookie-jar` saves on exit |

- `webrain obscura` passes **only** `--stealth` — no `--storage-dir`, no
  `--user-agent`. For persistent identity or a UA override, launch the binary
  directly: `%LOCALAPPDATA%/webrain/browsers/obscura-*/obscura.exe`.
- obscura refuses to serve CDP bound to `0.0.0.0` without `OBSCURA_CDP_TOKEN`
  (≥32 bytes). `webrain obscura` sidesteps it by binding 127.0.0.1; in Docker set
  the token or keep the bind on loopback.
- `--storage-dir` also removes the per-connection jar isolation above: the store
  is on disk, so `setcookies` on any connection persists for `batch`.
- obscura's `/json/version` still advertises its built-in UA even with
  `--user-agent` — trust `navigator.userAgent` after a navigate, not the discovery doc.

### Why obscura can never pass a Turnstile/Cloudflare challenge

Upstream states the boundary itself (`docs/Configure-stealth-and-proxies.md`):
*"What stealth does not handle: Cloudflare interactive challenges. Datadome and
Akamai bot manager active challenges. CAPTCHAs."* Stealth only covers *"basic bot
detection that checks TLS fingerprint or User-Agent"*. Four hard reasons:

1. **TLS is a fixed profile.** Stealth swaps in wreq/BoringSSL with
   `Profile::Chrome145`, while `navigator.userAgent` reports Chrome/143 out of the
   box. Pointing `--user-agent` at another version (e.g. Chrome/156, to match a
   Chrome-minted `cf_clearance`) only *widens* that disagreement — upstream's own
   comment calls a TLS layer and JS layer that disagree "a bot signal".
2. **No GPU.** `canvas.getContext('webgl')` returns `null` (verified). A browser
   claiming Windows Chrome with no WebGL context cannot be trusted; no flag fixes it.
3. **The challenge VM doesn't run.** Turnstile's orchestrate VM broke on a stubbed
   `structuredClone` (issue #389 — Cloudflare managed challenges *hang*), and
   `obscura-js/src/frame.rs` notes an untrusted iframe `postMessage` means "the
   widget hanging forever". Without a solved challenge there is no token to mint.
4. **Packet-level identity is the kernel's.** `tcp_options`/window/`ip_id` come from
   the host OS, and the docs say Obscura "cannot override these kernel defaults".

Consequence: obscura cannot mint its own clearance, and a clearance minted by any
other browser is unusable to it (reason 1). Profiles do not change this — an
obscura "profile" (`OBSCURA_PROFILE`, timezone, geolocation) only makes its own
fingerprint surfaces *self-consistent*; `--storage-dir` only persists cookies.
`SECURITY.md` additionally declares detection-evasion out of scope, so this is a
design boundary, not a bug that will close.

## Gotchas
- `Network.getCookies` is deprecated on Chrome 151 and returns `[]` — the backend
  prefers `Storage.getCookies` (incl. HttpOnly). If cookies show 0 on a logged-in
  browser, it's a restart-killed session cookie, not the tool.
- The auth cookie is often HttpOnly — verify via `webrain_session(op=cookies)` /
  navigate text, not `eval(document.cookie)`.
