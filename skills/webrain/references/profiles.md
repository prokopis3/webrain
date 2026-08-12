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

## Gotchas
- `Network.getCookies` is deprecated on Chrome 151 and returns `[]` — the backend
  prefers `Storage.getCookies` (incl. HttpOnly). If cookies show 0 on a logged-in
  browser, it's a restart-killed session cookie, not the tool.
- The auth cookie is often HttpOnly — verify via `webrain_session(op=cookies)` /
  navigate text, not `eval(document.cookie)`.
