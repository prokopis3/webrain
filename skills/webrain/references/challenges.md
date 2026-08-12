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

## 6. Verify
Re-navigate and confirm the TARGET content (not a challenge/login/consent page).
Never report an unverified bypass.

## 7. Failure / human fallback
If the native path can't claim an interactive challenge (interactive Turnstile /
hCaptcha), the human solves it in the headed real-Chrome browser (2FA/approval
gates return `waiting_for_human:true`). If that fails, report the gate to the
user — do not loop.
