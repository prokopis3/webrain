# Challenge Handling (detect → classify → state → capability → act → verify)

A challenge is a page/runtime state, not a dead end. Handle it as a state:
detect it, understand it, apply only what the runtime supports, verify.

## 1. Detect
`webrain_navigate` returns `challenge`: `null` (ok) | `cloudflare_challenge` |
`blocked` (403/forbidden) | `captcha`. Read it on EVERY navigate. Also check
`crippled` (few elements, no challenge) and `chrome_error` (ERR_/DNS_ pages).

## 2. Classify
- `cloudflare_challenge` — "Just a moment…" interstitial / Turnstile / `_cf_chl*`.
- `blocked` — 403 / forbidden page.
- `captcha` — reCAPTCHA / hCaptcha / interactive widget.
- `crippled: true` — HARD BLOCK page ("Attention Required!" / "you have been
  blocked"). NOT a solvable challenge: a WAF rule (usually IP/ASN) refused the
  request. Signs: `crippled:true` + `challenge:null` + only Cloudflare footer
  elements (`#cf-footer-ip-reveal`, `#brand_link`). See §5b.

## 3. State
The challenge is part of the browser's page state. Profile + session + cookies
(cf_clearance) are what survive and let you through on re-navigate.

## 4. Capability (runtime-dependent)
- obscura / lightpanda CANNOT pass interactive challenges (the challenge JS
  crashes). Non-interactive Turnstile may pass (verified).
- real Chrome + persistent profile: native `webrain_session(op=login)` /
  `webrain launch` + `webrain login` auto-waits challenges and claims
  Turnstile/reCAPTCHA/hCaptcha widgets; TOTP auto-fill from the vault; a 2FA /
  approval gate returns `waiting_for_human:true` and the human acts in the headed
  browser.
- No Python sidecar. Interactive Turnstile/hCaptcha the native path can't claim
  need a human in the headed browser.

## 5. Execute
When `challenge != null`:
1. Real Chrome + persistent profile: `webrain_session(op=open, cdp_url="http://127.0.0.1:9222")`.
2. `webrain_session(op=login, service, profile, url)` — vault + TOTP.
   OR re-attach an already-authenticated Chrome via `CDP_URL`.
3. Re-navigate the protected URL → expect `challenge: null`.
4. If a 2FA/approval gate: the human acts in the headed browser, then login again
   (`waiting_for_human:false`).

## 5b. Hard block (`crippled:true`) — bypass recipe
1. **Headless is detectable.** Launched Chrome headless → Cloudflare hard-blocks
   instead of challenging. Relaunch real HEADED Chrome with a persistent profile:
   `--user-data-dir=<persistent dir>` + `--disable-blink-features=AutomationControlled`
   on `--remote-debugging-port=9222`.
2. **Scope first.** A path can be blocked while the site loads — test the
   homepage before assuming the whole domain is gone. A blocked `/rss/` is often
   an ABSENT feed path that is ALSO explicitly WAF/nginx-blocked (you can't even
   see its 404). Verify with a control: a random nonexistent path should return
   a normal 404 — if `/rss/` returns a Cloudflare block / nginx 403 instead, the
   block is path-specific, AND there is simply no feed there. The real feed
   lives OFF-domain (FeedBurner/CDN). Find it: `webrain_eval` →
   `JSON.stringify([...document.querySelectorAll('link[rel="alternate"]')].map(l=>({type:l.type,href:l.href})))`
   or a spider filtered to `(?i)rss|feed|atom|\.xml`; then fetch that off-domain
   URL plain-HTTP (`webrain_download`) — it is NOT blocked.
3. **Same IP keeps blocking = IP/ASN block.** No browser change helps. Route a
   clean-IP proxy (`proxy` param) or report the gate to the user. Do NOT loop.
4. Re-navigate the TARGET after the fix → expect `challenge:null` +
   `crippled:false` + real content → extract.

## 6. Verify
Re-navigate and confirm the TARGET content (not a challenge/login/consent page).
Never report an unverified bypass.

## 7. Failure / human fallback
If the native path can't claim an interactive challenge (interactive Turnstile /
hCaptcha), the human solves it in the headed real-Chrome browser (2FA/approval
gates return `waiting_for_human:true`). If that fails, report the gate to the
user — do not loop.
