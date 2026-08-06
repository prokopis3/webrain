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
    loop {
        if has_session(backend).await? {
            return Ok(json!({ "logged_in": true, "submitted": submitted }));
        }
        if captcha_up(backend).await {
            return Ok(json!({
                "logged_in": false,
                "waiting_for_human": true,
                "challenge": "captcha",
                "message": "reCAPTCHA/anti-bot challenge — solve it in the headed browser, then call login again",
                "submitted": submitted,
            }));
        }
        if gate_up(backend).await {
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
                "message": "2FA/approval gate — approve or enter the code in the browser, then call login again",
                "submitted": submitted,
            }));
        }
        if t0.elapsed().as_secs() > 15 {
            return Ok(json!({
                "logged_in": false,
                "waiting_for_human": false,
                "message": "no session cookie after 15s — check creds or log in manually",
                "submitted": submitted,
            }));
        }
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
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
}
