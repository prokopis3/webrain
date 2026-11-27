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
  later tool calls route to via `session_id`. **Measured 2026-11-21 (this build):
  it is NOT a parallel-browser pool — the most recently opened session decides the
  engine for *every* handle.** Two `session_id`s then answer from the same window
  (a `window.__probe` written through one was readable through the other), so open
  the engine you actually want LAST and re-check `webgl` before trusting a result.
- **Lifecycle:** CREATE → PROFILE ATTACHED → REAL CHROME → SESSION ACTIVE →
  NAVIGATE → PAGE STATE → HANDLE → EXTRACT → VERIFY → PRESERVE (keep the session
  for the next call; `webrain_session(op=close)` to release).
- **Reuse:** keep the session open across pages; re-attach an
  already-authenticated Chrome by pointing `cdp_url`/`CDP_URL` at its port.
- **Isolation (the rule that decides whether a cookie bridge works):** every CDP
  *connection* has its own jar. Import the cookies **on the connection that will
  then navigate/batch** -- `webrain_session(op=setcookies, session_id=X)` then
  navigate/extract with the same `session_id`, or set them in the single process
  that does the work. `set: N / readback: N` proves only that the *writer's* jar
  holds them; it says nothing about the reader's. Verify with an authenticated
  request from the same session, never with a readback count.
  Measured 2026-11-21 (serper.dev, obscura on :9224): `webrain setcookies
  serper-cookies.json --port 9224` answered `set 8 ... readback: 8`, a *fresh*
  connection to the same running obscura answered `wrote 0 cookies`, the page
  bounced to `/login` and `api.serper.dev/auth/me/` returned **403**. Importing
  cookies through `webrain_session(op=setcookies, session_id="obsc3")` and then
  navigating in `obsc3` returned **200** with the account email and the dashboard's
  `Credits left 2,499` -- same cookies, same engine, same IP; only the holding
  connection differed. Cold start, same rule, ONE cookie: a fresh obscura (no
  `--storage-dir`), `op=setcookies` with only the httpOnly `connect.sid` on the
  navigating session -> `/auth/me/` 200 and `Credits left 2,499`. The analytics and
  consent cookies are not needed for the bridge.
- **Verify which engine you are actually on before diagnosing anything.** Session
  routing can silently leave you on the default browser; two handles then show the
  same page and you will "verify" the wrong engine. The tell:
  `webgl: !!canvas.getContext('webgl')` -- **false + UA Chrome/143 = obscura**,
  **true + UA Chrome/15x = real Chrome**. A Chrome/143 UA with `webgl: true`, or a
  second handle reporting byte-identical page state, means the call did not go
  where you think it did.

## Auth state transfer (cross-browser cookies)
1. Log in ONCE in real Chrome on a persistent profile (native login, vault+TOTP).
2. Export cookies on the LIVE browser: `webrain_session(op=cookies)` / CLI
   `webrain cookies --port 9222 --out file.json`. ⚠️ Session cookies do NOT
   survive a Chrome restart — export while Chrome is alive.
3. Import **on the connection that will do the work**:
   `webrain_session(op=setcookies, session_id=X, cookies=[...])`, then
   navigate/extract with that same `session_id`. A CLI `webrain setcookies --port N`
   writes into *that process's* jar, which the MCP session cannot see.
4. Batch WITHOUT `cdp_urls` on that same connection so it shares the cookies.
5. VERIFY from the same session before reporting success: call an authenticated
   endpoint (`fetch('https://api.<site>/auth/me/', {credentials:'include'})` -> 200)
   or read the logged-in page's own text. `set: N / readback: N` is not
   verification; a 403 from the API while the SPA shell still returns 200 is the
   classic false positive.
6. Alternative portable state: `webrain_session(op=save_state)` /
   `webrain_session(op=restore_state)` (cookies + localStorage JSON per profile).
   On obscura this path does NOT work: `restore_state` calls `Network.setCookies`,
   which obscura does not implement (`-32601 'Network.setCookies' wasn't found`),
   and when it does succeed against a `cdp_url` it fills *that* connection's jar,
   not the navigating session's (measured: `cookies_restored: 86`, navigating
   session still 403). Use `op=setcookies` with `session_id` instead.

⚠️ **A vendor clearance does not transfer.** An anti-bot token (`cf_clearance`,
DataDome, `_abck`) is bound to the minting browser's TLS/JA4 fingerprint + UA + IP.
Importing it into another engine fails even with a byte-identical UA — verified
2026-11-01 on a large marketplace: lightpanda (jar preloaded via `--cookie`, `--user-agent`
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
- `--storage-dir` is **disk persistence, not live sharing**: it does not lift the
  per-connection jar isolation while the server runs (measured 2026-11-21: cookies
  set on one connection, then `webrain cookies --port 9224` on a fresh connection
  -> `wrote 0 cookies`). It saves/loads the jar across a server restart. Bridge
  cookies IN-SESSION (previous section) instead of relying on it.
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
