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
// Optional: gated by the WEBRAIN_2CAPTCHA_KEY env var. No new dependency —
// the 2captcha API is a plain HTTP POST + poll, done with the existing ureq
// agent. ponytail: one solver (userrecaptcha); other challenge types / hosted
// multi-solver backends when this is ever load-bearing.
use anyhow::Context;

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
    fn build_request_rejects_bad_proxy() {
        // An unparseable proxy must fail loudly, not be silently dropped (the
        // returned token would be unusable for the real egress).
        assert!(build_in_request("k", "sk", "u", "", Some("not a url")).is_err());
        // A proxy without a port is unusable on 2captcha.
        assert!(build_in_request("k", "sk", "u", "", Some("http://127.0.0.1")).is_err());
        // Unknown scheme → error.
        assert!(build_in_request("k", "sk", "u", "", Some("gopher://h:1")).is_err());
    }
}
