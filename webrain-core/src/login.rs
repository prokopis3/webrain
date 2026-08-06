// Native login automation — port of stealth_solve.py's submit_login + 2FA gate,
// driven through the existing CdpBackend.evaluate + vault (no Python, no
// chromiumoxide). Browser + CDP are the only moving parts, so this is OS-agnostic.
//
// ponytail: composite login JS in one evaluate call (vs N fill/click round-trips);
// synthetic-Enter is a weak fallback — the headed human always finishes the rest.

use crate::backends::cdp::CdpBackend;
use crate::browser::BrowserBackend;
use crate::vault;
use serde_json::{Value, json};

fn jstr(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
}

/// Session-ish cookie names: presence of any after submit implies logged in
/// (Instagram sessionid, Google SID, Facebook c_user, ...).
// ponytail: real session cookies only. datr/dpr/mid/ig_did are set on the
// LOGIN page while logged OUT — including them made has_session() report
// logged_in:true falsely (the session's false-positive).
pub const SESSION_COOKIES: &[&str] = &[
    "sessionid",
    "ds_user_id",
    "session",
    "SID",
    "SSID",
    "APISID",
    "SAPISID",
    "c_user",
    "auth_token",
];

/// Fill the visible login form + submit. Returns `{ok, clicked, reason?}`.
pub fn login_js(user: &str, pass: &str) -> String {
    const TPL: &str = r#"(() => {
  const q = (s) => document.querySelector(s);
  const setVal = (el, val) => {
    const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, val);
    el.dispatchEvent(new Event('input', {bubbles:true}));
    el.dispatchEvent(new Event('change', {bubbles:true}));
  };
  const userEl = q('input[name="email"], input[name="username"], input[name="login"], input[type="email"]');
  const passEl = q('input[name="password"], input[type="password"]');
  if (!userEl || !passEl) return {ok:false, reason:'no-fields'};
  setVal(userEl, USER);
  setVal(passEl, PASS);
  const btn = q('button[type="submit"]') ||
    Array.from(document.querySelectorAll('[role="button"]')).find(b => /^(log in|sign in|continue|submit)$/i.test((b.innerText||'').trim()));
  if (btn) { btn.click(); return {ok:true, clicked:true}; }
  passEl.dispatchEvent(new KeyboardEvent('keydown', {key:'Enter', bubbles:true}));
  return {ok:true, clicked:false};
})()"#;
    TPL.replace("USER", &jstr(user))
        .replace("PASS", &jstr(pass))
}

/// True when a 2FA / approval / device-verification gate needs the human.
pub fn twofa_js() -> &'static str {
    r#"(() => {
  const u = location.href.toLowerCase();
  if (/challenge|two_factor|checkpoint|verify|onetap|login_required|otp|2fa/.test(u)) return true;
  const otp = document.querySelector('input[autocomplete="one-time-code"], input[name="code"], input[name="otp"], input[inputmode="numeric"][maxlength="6"]');
  if (otp && otp.offsetParent !== null) return true;
  const t = (document.body ? document.body.innerText : '').toLowerCase();
  return /enter the code from your authenticator app|enter your verification code|approve this device|confirm it's you|check your phone|we've sent a code/.test(t);
})()"#
}

/// Selector for the one-time-code field (TOTP injection).
pub fn otp_selector() -> &'static str {
    r#"input[autocomplete="one-time-code"], input[name="code"], input[name="otp"], input[inputmode="numeric"][maxlength="6"]"#
}

/// True when the active page carries a session-ish cookie (HttpOnly-aware).
async fn has_session(b: &CdpBackend) -> anyhow::Result<bool> {
    Ok(b.cookies()
        .await?
        .iter()
        .filter_map(|c| c["name"].as_str())
        .any(|n| SESSION_COOKIES.contains(&n)))
}

async fn gate_up(b: &CdpBackend) -> bool {
    b.evaluate(twofa_js())
        .await
        .map(|v| v.as_bool().unwrap_or(false))
        .unwrap_or(false)
}

/// reCAPTCHA / anti-bot challenge — creds were accepted, a human must solve.
/// Distinct from the 2FA gate: URL/iframe markers only, not the 2FA vocabulary.
async fn captcha_up(b: &CdpBackend) -> bool {
    b.evaluate(
        r#"(() => {
  const u = location.href.toLowerCase();
  if (/recaptcha|captcha|auth_platform/.test(u)) return true;
  if (document.getElementById('captcha-recaptcha')) return true;
  const t = (document.body ? document.body.innerText : '').toLowerCase();
  return /unusual traffic|verify you are human|complete the security check/.test(t);
})()"#,
    )
    .await
    .map(|v| v.as_bool().unwrap_or(false))
    .unwrap_or(false)
}

/// Detect the CF/Turnstile / reCAPTCHA / hCaptcha checkbox iframe and return its
/// viewport origin (top-left), or None. The checkbox sits near the iframe's
/// top-left corner.
async fn checkbox_up(b: &CdpBackend) -> Option<(i64, i64)> {
    let v = b
        .evaluate(
            r#"(() => {
  const f = document.querySelector(
    'iframe[src*="challenges.cloudflare.com"], iframe[src*="recaptcha"], iframe[title*="recaptcha"], iframe[src*="hcaptcha"]'
  );
  if (!f) return null;
  const r = f.getBoundingClientRect();
  if (!r.width || !r.height) return null;
  return { x: Math.round(r.left), y: Math.round(r.top) };
})()"#,
        )
        .await
        .ok()?;
    let x = v.get("x").and_then(|n| n.as_f64())? as i64;
    let y = v.get("y").and_then(|n| n.as_f64())? as i64;
    Some((x, y))
}

/// Trusted CDP click at the checkbox — crosses the cross-origin iframe boundary
/// where a JS click only focuses (reCAPTCHA/Turnstile).
/// ponytail: checkbox at ~(12,12) inside the iframe; real CF layout varies — if
/// the click doesn't clear, the ladder escalates to the human.
async fn auto_click_checkbox(b: &CdpBackend, (x, y): (i64, i64)) {
    let _ = b.click_coords(x + 12, y + 12).await;
}

/// Next step in the login ladder — pure and unit-testable (no browser).
#[derive(PartialEq, Eq, Debug)]
enum Ladder {
    Done,
    Poll,
    ClickCheckbox,
    FillTotp,
    WaitHuman,
    Timeout,
}

#[allow(clippy::too_many_arguments)]
fn next_ladder(
    has_session: bool,
    captcha: bool,
    twofa: bool,
    totp_available: bool,
    checkbox_clickable: bool,
    checkbox_attempts: u32,
    elapsed_secs: u64,
    budget_secs: u64,
) -> Ladder {
    if has_session {
        Ladder::Done
    } else if elapsed_secs >= budget_secs {
        Ladder::Timeout
    } else if twofa {
        if totp_available {
            Ladder::FillTotp
        } else {
            Ladder::WaitHuman
        }
    } else if captcha {
        // Auto-click the checkbox (max 2 attempts), then escalate to the human.
        if checkbox_clickable && checkbox_attempts < 2 {
            Ladder::ClickCheckbox
        } else {
            Ladder::WaitHuman
        }
    } else {
        Ladder::Poll
    }
}

/// One login attempt: fill+submit, poll briefly for a session cookie, and if a
/// 2FA/approval gate appears return immediately with `waiting_for_human: true`
/// (never blocks — the agent tells the user to act, then calls login again).
/// Shared by the CLI (`webrain login`) and the MCP `webrain_login` tool.
pub async fn run_login(
    backend: &CdpBackend,
    user: &str,
    pass: &str,
    totp: Option<&str>,
    url: Option<&str>,
) -> anyhow::Result<Value> {
    if let Some(u) = url {
        backend.navigate(u).await?;
    }
    let submitted = backend.evaluate(&login_js(user, pass)).await?;
    // no form found => page is a tablet/app interstitial or not the login form;
    // waiting out the 15s would report a misleading "check creds".
    if submitted.get("ok").and_then(|v| v.as_bool()) == Some(false) {
        return Ok(json!({
            "logged_in": false,
            "waiting_for_human": false,
            "message": "login form not found — the page is likely a tablet/app interstitial; click its 'Log in' button or use the real login URL, then call login again",
            "submitted": submitted,
        }));
    }
    let t0 = std::time::Instant::now();
    let mut checkbox_attempts = 0u32;
    loop {
        let has = has_session(backend).await?;
        let cap = captcha_up(backend).await;
        let gate = gate_up(backend).await;
        let ck = cap && checkbox_up(backend).await.is_some();
        match next_ladder(
            has,
            cap,
            gate,
            totp.is_some(),
            ck,
            checkbox_attempts,
            t0.elapsed().as_secs(),
            15,
        ) {
            Ladder::Done => return Ok(json!({ "logged_in": true, "submitted": submitted })),
            Ladder::Poll => tokio::time::sleep(std::time::Duration::from_secs(3)).await,
            Ladder::ClickCheckbox => {
                // Auto-solve: trusted click on the CF/reCAPTCHA checkbox.
                if let Some(rc) = checkbox_up(backend).await {
                    auto_click_checkbox(backend, rc).await;
                }
                checkbox_attempts += 1;
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Ladder::FillTotp => {
                // TOTP auto-fill if a seed is stored; the human still confirms submit.
                if let Some(seed) = totp {
                    if let Ok(code) = vault::totp_code(seed) {
                        let _ = backend
                            .evaluate(&vault::fill_js(otp_selector(), &code))
                            .await;
                    }
                }
                return Ok(json!({
                    "logged_in": false,
                    "waiting_for_human": true,
                    "message": "2FA/approval gate — TOTP code filled; confirm submit or enter the code, then call login again",
                    "submitted": submitted,
                }));
            }
            Ladder::WaitHuman => {
                let (msg, challenge) = if cap {
                    (
                        "reCAPTCHA/anti-bot challenge — auto-checkbox failed or is interactive; solve it in the headed browser (or the stealth sidecar), then call login again",
                        Some("captcha"),
                    )
                } else {
                    (
                        "2FA/approval gate — approve or enter the code in the browser, then call login again",
                        None,
                    )
                };
                return Ok(json!({
                    "logged_in": false,
                    "waiting_for_human": true,
                    "challenge": challenge,
                    "message": msg,
                    "submitted": submitted,
                }));
            }
            Ladder::Timeout => {
                return Ok(json!({
                    "logged_in": false,
                    "waiting_for_human": false,
                    "message": "no session cookie after 15s — check creds or log in manually",
                    "submitted": submitted,
                }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_cookies_exclude_login_page_only() {
        // regression: datr/dpr/mid/ig_did are set on the login page while logged
        // OUT; having them in SESSION_COOKIES made has_session() report
        // logged_in:true falsely.
        assert!(!SESSION_COOKIES.contains(&"datr"));
        assert!(!SESSION_COOKIES.contains(&"dpr"));
        assert!(!SESSION_COOKIES.contains(&"mid"));
        assert!(!SESSION_COOKIES.contains(&"ig_did"));
        assert!(SESSION_COOKIES.contains(&"sessionid"));
        assert!(SESSION_COOKIES.contains(&"ds_user_id"));
    }

    #[test]
    fn ladder_auto_solves_then_escalates() {
        // session present → done
        assert_eq!(
            next_ladder(true, false, false, false, false, 0, 0, 15),
            Ladder::Done
        );
        // nothing yet → keep polling
        assert_eq!(
            next_ladder(false, false, false, false, false, 0, 1, 15),
            Ladder::Poll
        );
        // captcha + clickable checkbox → auto-click (max 2 attempts), then human
        assert_eq!(
            next_ladder(false, true, false, false, true, 0, 1, 15),
            Ladder::ClickCheckbox
        );
        assert_eq!(
            next_ladder(false, true, false, false, true, 1, 1, 15),
            Ladder::ClickCheckbox
        );
        assert_eq!(
            next_ladder(false, true, false, false, true, 2, 1, 15),
            Ladder::WaitHuman
        );
        // interactive captcha (no clickable checkbox) → human immediately
        assert_eq!(
            next_ladder(false, true, false, false, false, 0, 1, 15),
            Ladder::WaitHuman
        );
        // 2FA + TOTP seed → auto-fill; no seed → human
        assert_eq!(
            next_ladder(false, false, true, true, false, 0, 1, 15),
            Ladder::FillTotp
        );
        assert_eq!(
            next_ladder(false, false, true, false, false, 0, 1, 15),
            Ladder::WaitHuman
        );
        // budget exhausted → timeout
        assert_eq!(
            next_ladder(false, false, false, false, false, 0, 15, 15),
            Ladder::Timeout
        );
    }
}
