# Rust Stealth Browser Research (2026-08-04) — verified facts

Source-verified via github_repo source-dumps + saved GitHub search JSONs (api.github.com
& github.com blocked in this env; use the 3 saved content.json files for star data).

## Verified star counts (from saved search JSON files)
- mattsse/chromiumoxide 1361★ Rust pushed 2026-04-03 ACTIVE, Apache-2.0, 58 open issues
- rust-headless-chrome/rust-headless-chrome 2940★ Rust pushed 2026-06-11 ACTIVE
  ("Rust equivalent of Puppeteer", sync API)
- CLOEI/chromiumoxide_stealth 6★ JavaScript (JS snippets only) stale
- browserbase/stagehand-rust 22★ Rust ALPHA (chromiumoxide-based)
- rebrowser-patches 1406★ / rebrowser-bot-detector 156★ / rebrowser-playwright-python 102★
- thirtyfour / fantoccini / playwright-rs: NOT re-verified (network blocked) ~1.1-1.5k★ approx

## chromiumoxide capabilities (VERIFIED in source)
- BrowserConfig::builder(): user_data_dir, HeadlessMode::New (--headless=new), chrome_executable,
  args, disable_default_args, window_size, incognito, proxy via args
- Browser::connect(url) — attach existing Chrome via /json/version or ws (webrain re-attach!)
- fetch_targets() — adopt pre-existing pages after connect
- Page::add_init_script / evaluate_on_new_document (AddScriptToEvaluateOnNewDocument) — stealth
- enable_stealth_mode()/with_agent(ua): webdriver→false, permissions.query, plugins PluginArray,
  WebGL getParameter(37445/37446), window.chrome={runtime:{}}, UA
- cookies: get_cookies/set_cookie(s)/delete_cookie(s) (Network.getCookies/SetCookies/DeleteCookies)
- Network domain: SetUserAgentOverride, Fetch request intercept (block-navigation example)
- events: page.event_listener<T>(), wait_for_navigation, lifecycle, console
- tests/stealth/rebrowser.rs + incolumitas.rs — actively measured vs bot-detector.rebrowser.net
- NOTE: enable_runtime is ON by default → Runtime.enable leak (rebrowser flags this)

## headless_chrome capabilities (VERIFIED)
- LaunchOptions: user_data_dir, headless, path, args, proxy_server, port, fetcher_options
  (auto-download Chromium), Browser::new / Browser::connect(debug_ws_url)
- Tab::enable_stealth_mode(): bypass_user_agent (HeadlessChrome→Chrome, Windows NT 10.0),
  bypass_wedriver, bypass_chrome, bypass_permissions, bypass_plugins, bypass_webgl_vendor
- Tab::get_cookies/set_cookies (Network::CookieParam), set_user_agent(ua,platform,lang)

## thirtyfour (VERIFIED)
- WebDriver client (needs chromedriver/geckodriver; WebDriver::managed auto-downloads;
  set_debugger_address("localhost:9222") attaches existing Chrome — webrain pattern)
- typed CDP: cdp().page().add_script_to_evaluate_on_new_document,
  cdp().network().set_user_agent_override, cdp().runtime().evaluate_value
- chrome caps: add_arg, add_exclude_switch("enable-automation") — uc-style flags
- BiDi opt-in (emulation overrides geolocation/locale/timezone)
- No built-in stealth (WebDriver sets navigator.webdriver=true)

## Python anti-bot techniques to replicate in Rust (VERIFIED)
- uc: binary-patch chromedriver cdc_ block (gen_random_cdc) + addScriptToEvaluateOnNewDocument:
  webdriver Proxy→false, window.chrome full stub, maxTouchPoints, connection.rtt,
  permissions.query, Function.prototype.toString native spoof, Headless UA strip
- nodriver: PURE CDP no Selenium — addScriptToEvaluateOnNewDocument, Emulation.setUserAgentOverride
  (+userAgentMetadata for Sec-CH-UA), Emulation.setAutomationOverride,
  Emulation.setHardwareConcurrencyOverride, Network.getCookies. ← Rust should MIRROR this (CDP-direct)
- rebrowser-patches: Runtime.enable leak fix (addBinding / createIsolatedWorld / enable+disable),
  sourceURL→app.js generic, utility world rename. README warns: not bulletproof, need
  proxies+fingerprints+behavior; "less JS injection the better"

## Recommendation (architecture)
- Primary: chromiumoxide = Rust nodriver (CDP-direct, no chromedriver → no cdc_/webdriver class).
  Launch real Chrome headed with user-data-dir per profile; attach via Browser::connect; apply
  add_init_script stealth bundle (extend webrain STEALTH_JS); Network.getCookies/SetCookies for
  portable cookie JSON; keep Python stealth_solve.py as optional hard-challenge sidecar.
- 2FA gate: run headed; poll Runtime.evaluate for gate markers (auth URL, "Enter the code from
  your authenticator app", autocomplete=one-time-code inputs, Turnstile/reCAPTCHA iframe);
  BLOCK the MCP tool with tokio Notify until human acts; TOTP auto-fill from vault.rs.
- Profiles: %APPDATA%/webrain/profiles/<service>/<profile>/ as user-data-dir (cf_clearance lives
  there → reuse = no re-solve). Cookies JSON per profile for portability (keep Secure/HttpOnly/
  SameSite attrs or set fails).
- Human-in-the-loop MANDATORY: interactive Turnstile, reCAPTCHA v2 checkbox/image/audio, SMS/email
  OTP, push approvals (Google "check your phone"), device verification, DataDome/PerimeterX risk.

## Automatic login & stealth — most efficient architecture (2026-08-06, re-verified)

### Verified landscape (live GitHub search, 2026-08-06)
| Repo | ★ | Approach | Fit for webrain |
|---|---|---|---|
| CloakHQ/CloakBrowser | 29,679 | source-patched Chromium, 30/30 bot tests | gold standard, but vendors a custom Chromium build — per-version rebuild, breaks the 3-engine model |
| ultrafunkamsterdam/undetected-chromedriver | 12,789 | **chromedriver** binary patch (`cdc_` marker) + init-script | N/A — webrain is CDP-direct, no chromedriver |
| daijro/camoufox | 10,860 | anti-detect Firefox, in-engine fingerprint | engine swap; doesn't fit the CDP-Chrome 3-engine model |
| jo-inc/camofox-browser | 8,368 | JS stealth headless (drop-in Puppeteer/Playwright) | JS analog, not Rust |
| ultrafunkamsterdam/nodriver | 4,622 | **CDP-direct** (successor of uc, no chromedriver) | **closest to webrain's architecture** |
| Kaliiiiiiiiii-Vinyzu/patchright | 4,022 | undetected Playwright (patches at protocol level) | the "runs like undetected-playwright" UX target |
| cdpdriver/zendriver | 1,385 | async nodriver fork | same CDP-direct idea |
| feder-cr/invisible_playwright | 1,845 | stealth baked into Firefox engine | engine swap |
| Vinyzu/Botright | 1,007 | AI captcha-solving on top of Playwright | solver-API dependency |
| AtuboDad/playwright_stealth | 980 | init-script patch set (JS) | port-source for STEALTH_JS gaps |
| Ulyssedev/Rust-undetected-chromedriver | 63 | Rust, thirtyfour-based | immature, thirtyfour (WebDriver) not CDP |

### Decision: extend the CDP-direct path — do NOT add an engine or binary
Webrain **already is** the nodriver/undetected-playwright architecture: Rust
CDP-direct, real Chrome, per-profile `--user-data-dir` (`launch.rs:184`),
`STEALTH_JS` on `Page.addScriptToEvaluateOnNewDocument` (`cdp.rs:643`), UA +
automation overrides on attach (`cdp.rs:621/632`), a credential/TOTP vault, and
challenge detection (`PageState.challenge`, `login::captcha_up`). The efficient
change is **completion, not replacement**: port the verified patch gaps and
turn the manual login hand-off into an automatic solve ladder.

### Stealth gaps to port (verified vs current STEALTH_JS + attach)
1. **`userAgentMetadata` in `Network.setUserAgentOverride`** — the attach forges a
   Win-Chrome 151 UA but sends no `userAgentMetadata`, so real Chrome emits a
   `Sec-CH-UA` header from its ACTUAL build that can mismatch the forged UA.
   Add `{brands, fullVersion, mobile, platform}` so Sec-CH-UA matches (nodriver
   sets it via `Emulation.setUserAgentOverride` + `userAgentMetadata`).
2. **`Function.prototype.toString` native spoof** (uc) — `permissions.query`
   etc. must return `"function query() { [native code] }"`, not the wrapped
   source.
3. **`Runtime.enable` leak** (rebrowser-patches) — `Runtime.enable` is on by
   default in the attach; harden via utility-world rename / addBinding or keep
   it scoped to the eval window. Webrain's `STEALTH_JS` already masks
   `webdriver`, real-prototype plugins, `window.chrome`, permissions.query,
   canvas/audio noise, and the tracker blocklist — keep all of it.
4. Headless UA strip (`"Headless"` → `""`) only if headless mode is used —
   webrain runs headed by default, so skip unless headless is added.

### Automatic login solve ladder (login.rs: replace `waiting_for_human` default with auto-solve)
```
navigate(login_url)  →  fill+submit (login_js)  →  poll loop:
  1. has_session()?                                  → logged_in ✓
  2. CF/Turnstile "Verify you are human" checkbox?   → auto-click via CDP frame+coords
                                                       (nodriver cf_verify pattern) → re-poll
  3. reCAPTCHA v2 checkbox?                          → auto-click → re-poll
  4. 2FA/OTP gate?                                   → vault TOTP auto-fill (exists) → re-poll
  5. interactive (image/audio grid, Turnstile
     interactive, push approval, SMS/email OTP)?     → LAST resort: waiting_for_human,
                                                       auto-spawn the headed stealth_solve.py
                                                       sidecar instead of a manual step
  6. >15s no session                                 → report "check creds / solve manually"
```
Profile reuse is the force-multiplier: `cf_clearance` + session cookies live in
the per-service `--user-data-dir`, so a solved challenge never re-appears on the
next `webrain_login`. That alone kills most repeat challenges (nodriver keeps
`user_data_dir` for the same reason).

### Integration points (existing code, no new deps)
- `webrain-core/src/login.rs` `run_login` — add the solve ladder to the poll
  loop; `captcha_up` becomes "auto-click first, escalate after".
- `webrain-core/src/backends/cdp.rs` `attach_and_init` — add `userAgentMetadata`
  + `Function.prototype.toString` spoof to the override/STEALTH_JS.
- `webrain-core/src/launch.rs` — keep per-profile `--user-data-dir` as the login
  default (already the shape).
- `webrain-core/src/vault.rs` — TOTP auto-fill already wired into the 2FA gate.

### Verify (one runnable check)
Extend `login.rs` tests with a checkbox-handler unit test: given a fake
"verify you are human" iframe + checkbox, the ladder clicks, re-polls, and
returns `logged_in` (or escalates to `waiting_for_human`). Mirror nodriver's
live `cf_verify` measurement on a real Turnstile page.
