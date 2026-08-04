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
