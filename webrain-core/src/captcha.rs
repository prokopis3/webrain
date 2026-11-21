// webrain-core/src/captcha.rs
// CAPTCHA solving for Google's /sorry wall — the open-serp 2captcha recipe:
//
//   1. On a walled page, read the recaptcha `data-sitekey` + `data-s` from the
//      challenge element.
//   2. POST to 2captcha in.php with method=userrecaptcha, googlekey, pageurl,
//      datas, and — critically — the SAME proxy the request egressed through,
//      so 2captcha's worker solves from that IP and the token is valid for it.
//   3. Poll res.php until the token is ready.
//   4. Inject the token into #g-recaptcha-response and call submitCallback().
//
// Optional, and never automatic: a key buys a TOKEN — it does not read the
// sitekey off the page and it does not inject the result. Two vendors under
// three accepted names (`CAPSOLVER_API_KEY`, `WEBRAIN_2CAPTCHA_KEY`,
// `TWOCAPTCHA_API_KEY`); `configured_provider` is the one place that decides.
// No new dependency — both APIs are a plain HTTP POST + poll, done with the
// existing ureq agent. ponytail: one task type (reCAPTCHA v2 userrecaptcha);
// hCaptcha/Turnstile task types when the page-side sitekey reader exists.
use anyhow::Context;
use serde_json::{Value, json};

const IN_URL: &str = "https://2captcha.com/in.php";
const RES_URL: &str = "https://2captcha.com/res.php";

/// Minimal percent-decoding for proxy userinfo (`%XX` escapes only).
fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let h = (bytes[i + 1] as char).to_digit(16);
            let l = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (h, l) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Build the in.php form params. Pure — unit-tested below. Returns an error on
/// an invalid/unusable proxy instead of silently dropping it (a mangled proxy
/// yields a token unusable for the real egress and wastes a paid solve).
fn build_in_request(
    api_key: &str,
    sitekey: &str,
    page_url: &str,
    data_s: &str,
    proxy: Option<&str>,
) -> anyhow::Result<Vec<(String, String)>> {
    let mut p = vec![
        ("key".to_string(), api_key.to_string()),
        ("method".to_string(), "userrecaptcha".to_string()),
        ("googlekey".to_string(), sitekey.to_string()),
        ("pageurl".to_string(), page_url.to_string()),
    ];
    if !data_s.is_empty() {
        p.push(("datas".to_string(), data_s.to_string()));
    }
    // 2captcha workers must exit from the same IP that will use the token.
    if let Some(raw) = proxy {
        let u =
            url::Url::parse(raw).with_context(|| format!("invalid 2captcha proxy URL '{raw}'"))?;
        let host = u
            .host_str()
            .with_context(|| format!("proxy '{raw}' has no host"))?;
        // 2captcha supports HTTP / SOCKS4 / SOCKS5 — map the scheme precisely
        // (socks4/socks4a are NOT SOCKS5; an unknown scheme is an error).
        let ptype = match u.scheme() {
            "http" | "https" => "HTTP",
            "socks4" | "socks4a" => "SOCKS4",
            "socks" | "socks5" | "socks5h" => "SOCKS5",
            other => anyhow::bail!("unsupported 2captcha proxy scheme '{other}'"),
        };
        // 2captcha cannot use a proxy without an explicit port — error out.
        let port = u
            .port()
            .with_context(|| format!("proxy '{raw}' needs an explicit port"))?;
        // u.username()/u.password() are percent-encoded — decode so credentials
        // containing '@', ':', spaces etc. aren't mangled on the wire.
        let user = pct_decode(u.username());
        let pass = pct_decode(u.password().unwrap_or(""));
        let addr = if user.is_empty() {
            format!("{host}:{port}")
        } else if pass.is_empty() {
            format!("{user}@{host}:{port}")
        } else {
            format!("{user}:{pass}@{host}:{port}")
        };
        p.push(("proxytype".to_string(), ptype.to_string()));
        p.push(("proxy".to_string(), addr));
    }
    Ok(p)
}

/// Submit a reCAPTCHA-v2 task and poll until the token is ready. Blocking
/// (ureq) — matches the rest of the serp HTTP path. Returns the token.
pub fn solve_recaptcha2(
    api_key: &str,
    sitekey: &str,
    page_url: &str,
    data_s: &str,
    proxy: Option<&str>,
) -> anyhow::Result<String> {
    let params = build_in_request(api_key, sitekey, page_url, data_s, proxy)?;
    // POST the form body — the API key + proxy credentials must never go in the
    // URL (query strings are captured by access logs / reverse proxies and long
    // proxy creds can blow URL length limits).
    let (_, body) = crate::engines::serp_http_post(IN_URL, &params, None)?;
    let id = body
        .trim()
        .strip_prefix("OK|")
        .with_context(|| format!("2captcha submit failed: {body}"))?
        .to_string();

    // Poll every 5s up to ~2.5min; 2captcha typical solve time is 5-30s.
    let mut transport_fails = 0usize;
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        // POST the poll too — the key must never appear in a query string
        // (2captcha accepts POSTed form params; a proxied poll would otherwise
        // put the key in the proxy's access logs).
        let poll_params = vec![
            ("key".to_string(), api_key.to_string()),
            ("action".to_string(), "get".to_string()),
            ("id".to_string(), id.clone()),
        ];
        let (status, body) = match crate::engines::serp_http_post(RES_URL, &poll_params, None) {
            Ok(v) => v,
            // Transient transport blip — the (possibly charged) task is still
            // pending server-side; keep polling instead of aborting the solve.
            Err(e) => {
                transport_fails += 1;
                if transport_fails >= 5 {
                    return Err(e.context("2captcha poll failed repeatedly"));
                }
                continue;
            }
        };
        let body = body.trim().to_string();
        if let Some(token) = body.strip_prefix("OK|") {
            return Ok(token.trim().to_string());
        }
        if body.contains("CAPCHA_NOT_READY") {
            transport_fails = 0;
            continue;
        }
        return Err(anyhow::anyhow!(
            "2captcha result failed (HTTP {status}): {body}"
        ));
    }
    Err(anyhow::anyhow!("2captcha timeout for id {id}"))
}

// ── CapSolver ──────────────────────────────────────────────────────────────
// Same job, JSON instead of a form: createTask → poll getTaskResult. Scoped to
// reCAPTCHA v2 because that is the only challenge this module already knows how
// to name; a Turnstile/hCaptcha task type belongs with the page-side code that
// reads its sitekey, which does not exist yet (see the module note).
const CAPSOLVER_CREATE: &str = "https://api.capsolver.com/createTask";
const CAPSOLVER_RESULT: &str = "https://api.capsolver.com/getTaskResult";
const CAPSOLVER_POLL_SECS: u64 = 5;
const CAPSOLVER_POLL_TRIES: usize = 30;

/// CapSolver proxy syntax is `scheme:user:pass@host:port` — a colon after the
/// scheme, not `://` — and it has no SOCKS4. A proxy is never silently dropped:
/// the token would then be bound to the solver's IP and unusable for the egress
/// that asked for it, which is a paid solve wasted.
fn capsolver_proxy(raw: &str) -> anyhow::Result<String> {
    let u = url::Url::parse(raw).with_context(|| format!("invalid CapSolver proxy URL '{raw}'"))?;
    let scheme = match u.scheme() {
        "http" | "https" => "http",
        "socks5" | "socks5h" => "socks5",
        other => anyhow::bail!("CapSolver does not support proxy scheme '{other}'"),
    };
    let host = u
        .host_str()
        .with_context(|| format!("proxy '{raw}' has no host"))?;
    let port = u
        .port()
        .with_context(|| format!("proxy '{raw}' needs an explicit port"))?;
    let user = pct_decode(u.username());
    let pass = pct_decode(u.password().unwrap_or(""));
    Ok(if user.is_empty() {
        format!("{scheme}:{host}:{port}")
    } else if pass.is_empty() {
        format!("{scheme}:{user}@{host}:{port}")
    } else {
        format!("{scheme}:{user}:{pass}@{host}:{port}")
    })
}

/// CapSolver `createTask` body. Pure — unit-tested below.
fn build_capsolver_task(
    client_key: &str,
    sitekey: &str,
    page_url: &str,
    data_s: &str,
    proxy: Option<&str>,
) -> anyhow::Result<Value> {
    let mut task = json!({
        // A proxied task solves from the caller's exit; the proxy-less task
        // does not, so the two are different types, not a flag.
        "type": if proxy.is_some() { "ReCaptchaV2Task" } else { "ReCaptchaV2TaskProxyLess" },
        "websiteURL": page_url,
        "websiteKey": sitekey,
    });
    if !data_s.is_empty() {
        // Google's /sorry ships the challenge secret in `data-s`.
        task["recaptchaDataSValue"] = json!(data_s);
    }
    if let Some(raw) = proxy {
        task["proxy"] = json!(capsolver_proxy(raw)?);
    }
    Ok(json!({ "clientKey": client_key, "task": task }))
}

/// `errorId == 0` means success; anything else carries `errorCode` +
/// `errorDescription` and must be surfaced, not retried.
fn capsolver_error(v: &Value) -> Option<String> {
    if v.get("errorId").and_then(|x| x.as_i64()).unwrap_or(0) == 0 {
        return None;
    }
    let code = v
        .get("errorCode")
        .and_then(|x| x.as_str())
        .unwrap_or("ERROR");
    match v.get("errorDescription").and_then(|x| x.as_str()) {
        Some(d) if !d.is_empty() => Some(format!("{code}: {d}")),
        _ => Some(code.to_string()),
    }
}

/// Submit a reCAPTCHA-v2 task to CapSolver and poll until the token is ready.
/// Blocking (ureq), matching the rest of this module.
pub fn solve_recaptcha2_capsolver(
    client_key: &str,
    sitekey: &str,
    page_url: &str,
    data_s: &str,
    proxy: Option<&str>,
) -> anyhow::Result<String> {
    let body = build_capsolver_task(client_key, sitekey, page_url, data_s, proxy)?;
    let (_, text) = crate::engines::serp_http_post_json(CAPSOLVER_CREATE, &body, None)?;
    let created: Value = serde_json::from_str(&text)
        .with_context(|| format!("CapSolver createTask returned non-JSON: {text}"))?;
    if let Some(e) = capsolver_error(&created) {
        anyhow::bail!("CapSolver createTask failed: {e}");
    }
    let task_id = created
        .get("taskId")
        .and_then(|x| x.as_str())
        .with_context(|| format!("CapSolver createTask returned no taskId: {text}"))?
        .to_string();

    let mut transport_fails = 0usize;
    for _ in 0..CAPSOLVER_POLL_TRIES {
        std::thread::sleep(std::time::Duration::from_secs(CAPSOLVER_POLL_SECS));
        let poll = json!({ "clientKey": client_key, "taskId": task_id });
        let (status, text) =
            match crate::engines::serp_http_post_json(CAPSOLVER_RESULT, &poll, None) {
                Ok(v) => v,
                // The task is already billed and still running server-side, so a
                // transport blip must not abort the solve.
                Err(e) => {
                    transport_fails += 1;
                    if transport_fails >= 5 {
                        return Err(e.context("CapSolver poll failed repeatedly"));
                    }
                    continue;
                }
            };
        let v: Value = serde_json::from_str(&text).with_context(|| {
            format!("CapSolver getTaskResult returned non-JSON (HTTP {status}): {text}")
        })?;
        if let Some(e) = capsolver_error(&v) {
            anyhow::bail!("CapSolver getTaskResult failed: {e}");
        }
        match v.get("status").and_then(|x| x.as_str()) {
            Some("ready") => {
                return v
                    .get("solution")
                    .and_then(|s| s.get("gRecaptchaResponse"))
                    .and_then(|t| t.as_str())
                    .map(|t| t.trim().to_string())
                    .with_context(|| {
                        format!("CapSolver ready without gRecaptchaResponse: {text}")
                    });
            }
            Some("processing") => {
                transport_fails = 0;
                continue;
            }
            other => anyhow::bail!("CapSolver unexpected status {other:?}: {text}"),
        }
    }
    Err(anyhow::anyhow!("CapSolver timeout for task {task_id}"))
}

/// Pure: pick a vendor from a pair of key values. A blank or whitespace-only
/// key counts as unset, so `CAPSOLVER_API_KEY=` cannot shadow a real 2captcha key.
fn choose_provider(
    capsolver: Option<&str>,
    twocaptcha: Option<&str>,
) -> Option<(&'static str, String)> {
    let clean = |v: Option<&str>| {
        v.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    clean(capsolver)
        .map(|k| ("capsolver", k))
        .or_else(|| clean(twocaptcha).map(|k| ("2captcha", k)))
}

/// Read a key from the environment, blank/whitespace meaning unset.
fn env_key(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Which paid solver the environment configures, if any. CapSolver wins when
/// both are set — it is cheaper and faster per solve, and a machine holding two
/// paid keys is not asked to pick.
///
/// 2captcha answers to two spellings: `WEBRAIN_2CAPTCHA_KEY` (ours, shipped
/// since v0.7.3) and `TWOCAPTCHA_API_KEY` (what everyone guesses, because the
/// vendor writes its own name as "2captcha"). `2CAPTCHA_API_KEY` is deliberately
/// NOT read: an env name starting with a digit cannot be exported in a POSIX
/// shell — `export 2CAPTCHA_API_KEY=…` is a syntax error in bash/dash — so
/// honouring it would ship a variable that only ever works on Windows.
pub fn configured_provider() -> Option<(&'static str, String)> {
    let twocaptcha = env_key("WEBRAIN_2CAPTCHA_KEY").or_else(|| env_key("TWOCAPTCHA_API_KEY"));
    choose_provider(
        env_key("CAPSOLVER_API_KEY").as_deref(),
        twocaptcha.as_deref(),
    )
}

/// Solve with whichever provider has a key. Errors name the variable to set, so
/// a caller never has to guess why nothing happened.
pub fn solve_recaptcha2_auto(
    sitekey: &str,
    page_url: &str,
    data_s: &str,
    proxy: Option<&str>,
) -> anyhow::Result<String> {
    match configured_provider() {
        Some(("capsolver", key)) => {
            solve_recaptcha2_capsolver(&key, sitekey, page_url, data_s, proxy)
        }
        Some((_, key)) => solve_recaptcha2(&key, sitekey, page_url, data_s, proxy),
        None => anyhow::bail!(
            "no CAPTCHA solver configured — set CAPSOLVER_API_KEY (or WEBRAIN_2CAPTCHA_KEY / TWOCAPTCHA_API_KEY)"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_params() {
        let p = build_in_request("k", "sk", "https://google.com/sorry", "ds", None).unwrap();
        assert!(p.contains(&("key".into(), "k".into())));
        assert!(p.contains(&("googlekey".into(), "sk".into())));
        assert!(p.contains(&("datas".into(), "ds".into())));
        assert!(!p.iter().any(|(k, _)| k == "proxytype"));

        // Proxy is forwarded so the solver egresses from the same IP.
        let p = build_in_request(
            "k",
            "sk",
            "https://google.com/sorry",
            "",
            Some("http://user:pass@127.0.0.1:8080"),
        )
        .unwrap();
        assert!(p.contains(&("proxytype".into(), "HTTP".into())));
        assert!(p.contains(&("proxy".into(), "user:pass@127.0.0.1:8080".into())));

        let p = build_in_request("k", "sk", "u", "", Some("socks5://127.0.0.1:1080")).unwrap();
        assert!(p.contains(&("proxytype".into(), "SOCKS5".into())));
        assert!(p.contains(&("proxy".into(), "127.0.0.1:1080".into())));

        // socks4 → SOCKS4 (NOT SOCKS5); credentials are percent-decoded.
        let p = build_in_request(
            "k",
            "sk",
            "u",
            "",
            Some("socks4://us%40er:p%40ss@127.0.0.1:1080"),
        )
        .unwrap();
        assert!(p.contains(&("proxytype".into(), "SOCKS4".into())));
        assert!(p.contains(&("proxy".into(), "us@er:p@ss@127.0.0.1:1080".into())));
    }

    #[test]
    fn capsolver_task_body_and_proxy() {
        let v = build_capsolver_task("ck", "sk", "https://google.com/sorry", "", None).unwrap();
        assert_eq!(v["clientKey"], "ck");
        assert_eq!(v["task"]["type"], "ReCaptchaV2TaskProxyLess");
        assert_eq!(v["task"]["websiteKey"], "sk");
        assert!(v["task"].get("proxy").is_none());
        assert!(v["task"].get("recaptchaDataSValue").is_none());

        // data-s is forwarded, and a proxy switches the task type + syntax.
        let v = build_capsolver_task(
            "ck",
            "sk",
            "u",
            "ds",
            Some("http://us%40er:p%40ss@127.0.0.1:8080"),
        )
        .unwrap();
        assert_eq!(v["task"]["type"], "ReCaptchaV2Task");
        assert_eq!(v["task"]["recaptchaDataSValue"], "ds");
        assert_eq!(v["task"]["proxy"], "http:us@er:p@ss@127.0.0.1:8080");
        assert_eq!(
            capsolver_proxy("socks5://127.0.0.1:1080").unwrap(),
            "socks5:127.0.0.1:1080"
        );
        // CapSolver has no SOCKS4 and needs an explicit port — both error.
        assert!(capsolver_proxy("socks4://127.0.0.1:1080").is_err());
        assert!(capsolver_proxy("http://127.0.0.1").is_err());
    }

    #[test]
    fn capsolver_error_surfaces_vendor_text() {
        assert!(capsolver_error(&json!({"errorId": 0, "taskId": "t"})).is_none());
        assert_eq!(
            capsolver_error(&json!({
                "errorId": 12,
                "errorCode": "ERROR_ZERO_BALANCE",
                "errorDescription": "no funds"
            }))
            .unwrap(),
            "ERROR_ZERO_BALANCE: no funds"
        );
    }

    #[test]
    fn build_request_rejects_bad_proxy() {
        // An unparseable proxy must fail loudly, not be silently dropped (the
        // returned token would be unusable for the real egress).
        assert!(build_in_request("k", "sk", "u", "", Some("not a url")).is_err());
        // A proxy without a port is unusable on 2captcha.
        assert!(build_in_request("k", "sk", "u", "", Some("http://127.0.0.1")).is_err());
        // Unknown scheme → error.
        assert!(build_in_request("k", "sk", "u", "", Some("gopher://h:1")).is_err());
    }

    #[test]
    fn both_vendors_are_selectable_and_capsolver_wins() {
        // Both keys set → CapSolver (the vendor the module was extended for).
        let (name, key) = choose_provider(Some("cs-key"), Some("2c-key")).unwrap();
        assert_eq!((name, key.as_str()), ("capsolver", "cs-key"));
        // Only the older vendor set → 2captcha, not "no solver configured".
        let (name, key) = choose_provider(None, Some("2c-key")).unwrap();
        assert_eq!((name, key.as_str()), ("2captcha", "2c-key"));
        // A blank/whitespace CAPSOLVER_API_KEY must not shadow a real 2captcha key.
        let (name, _) = choose_provider(Some("   "), Some("2c-key")).unwrap();
        assert_eq!(name, "2captcha");
        // Empty vars are unset, not empty keys.
        assert!(choose_provider(Some(""), Some(" ")).is_none());
        assert!(choose_provider(None, None).is_none());
    }
}
