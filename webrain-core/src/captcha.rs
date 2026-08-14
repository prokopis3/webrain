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

/// Build the in.php form params. Pure — unit-tested below.
fn build_in_request(
    api_key: &str,
    sitekey: &str,
    page_url: &str,
    data_s: &str,
    proxy: Option<&str>,
) -> Vec<(String, String)> {
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
        if let Ok(u) = url::Url::parse(raw) {
            if let Some(host) = u.host_str() {
                let ptype = if u.scheme().starts_with("socks") {
                    "SOCKS5"
                } else {
                    "HTTP"
                };
                let mut addr = host.to_string();
                if let Some(port) = u.port() {
                    addr = format!("{host}:{port}");
                }
                if !u.username().is_empty() {
                    let pass = u.password().unwrap_or("");
                    addr = if pass.is_empty() {
                        format!("{}@{addr}", u.username())
                    } else {
                        format!("{}:{pass}@{addr}", u.username())
                    };
                }
                p.push(("proxytype".to_string(), ptype.to_string()));
                p.push(("proxy".to_string(), addr));
            }
        }
    }
    p
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
    let params = build_in_request(api_key, sitekey, page_url, data_s, proxy);
    let submit = format!(
        "{IN_URL}?{}",
        url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish()
    );
    let (_, body) = crate::engines::serp_http_get(&submit, None)?;
    let id = body
        .trim()
        .strip_prefix("OK|")
        .with_context(|| format!("2captcha submit failed: {body}"))?
        .to_string();

    // Poll every 5s up to ~2.5min; 2captcha typical solve time is 5-30s.
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let poll = format!("{RES_URL}?key={api_key}&action=get&id={id}");
        let (_, body) = crate::engines::serp_http_get(&poll, None)?;
        let body = body.trim().to_string();
        if let Some(token) = body.strip_prefix("OK|") {
            return Ok(token.trim().to_string());
        }
        if body.contains("CAPCHA_NOT_READY") {
            continue;
        }
        return Err(anyhow::anyhow!("2captcha result failed: {body}"));
    }
    Err(anyhow::anyhow!("2captcha timeout for id {id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_request_params() {
        let p = build_in_request("k", "sk", "https://google.com/sorry", "ds", None);
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
        );
        assert!(p.contains(&("proxytype".into(), "HTTP".into())));
        assert!(p.contains(&("proxy".into(), "user:pass@127.0.0.1:8080".into())));

        let p = build_in_request("k", "sk", "u", "", Some("socks5://127.0.0.1:1080"));
        assert!(p.contains(&("proxytype".into(), "SOCKS5".into())));
        assert!(p.contains(&("proxy".into(), "127.0.0.1:1080".into())));
    }
}
