# Anti-Patterns (what NOT to do)

DO NOT:
- start protected workflows statelessly (anonymous → blocked → fresh browser → retry)
- discard a working profile
- discard a working session
- switch to a fresh browser after a challenge (you lose the identity/session that would get you through)
- treat challenge detection as successful scraping (a challenge page is not content)
- assume CAPTCHA handling exists without checking runtime capability
- reintroduce a Python/stealth sidecar (challenge handling is native)
- restore the obsolete stealth architecture
- claim a bypass succeeded without verification (verify the target content)
- extract challenge/login/consent pages as target content
- create sequential loops when `webrain_batch`/`webrain_crawl` exist
- re-run a failing browser call more than once without changing state: name the
  symptom first (wedged engine? wrong engine? cookies on another connection?)
- treat `setcookies` `readback: N` as proof the next call is authenticated - prove
  it with an authenticated endpoint (403 vs 200)
- import cookies on one connection (CLI `--port N`) and expect them on another:
  obscura jars are per-connection; bridge in-session
- assume two `session_id`s are two engines: the last `op=open` decides the engine
  for every handle (probe `webgl`), so open the one you want LAST
- move cookies with `restore_state` on obscura: `Network.setCookies` is not
  implemented there, and its `cdp_url` path fills that connection's jar only
- copy the whole jar when the session cookie is enough (one httpOnly `connect.sid`
  authenticated a cold-start obscura)
- compare two `session_id`s without an identity probe (`webgl`, UA) - identical
  page state means one browser, i.e. a "verified" bridge on the wrong engine
- rely on obscura `--storage-dir` for live cookie sharing (restart-time disk
  persistence only; measured: 0 cookies on a fresh connection)
- dump raw HTML to the model when `observe`/`extract` give text/structure cheaper
- wait 30 s for `visible` on an SPA list instead of `attached` + 6 s on the card
  selector (measured: 159 s → 10 s for 27 URLs, same rows)
- anchor a wait on a shell element that exists before the cards (`#ProductsClass`:
  0.2-2.4 s, `found: 0` everywhere) — anchor on the repeated card instead
- blame `concurrency` for a slow batch before reading `ms_total` vs the real page
  time: wait timeouts look exactly like a slow site
