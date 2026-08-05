// webrain-cli: Single binary — `webrain mcp` | `webrain fetch` | `webrain screenshot` | `webrain spider`
//
// ponytail: one binary, subcommands via match, no clap dependency.

use serde_json::json;
use std::env;
use webrain_core::CdpBackend;
use webrain_core::browser::BrowserBackend;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "webrain=info,tungstenite=warn".into()),
        )
        // MCP speaks pure JSON on stdout — logs must never pollute it.
        .with_writer(std::io::stderr)
        .init();

    let args: Vec<String> = env::args().collect();
    let rt = tokio::runtime::Runtime::new()?;

    // `webrain doctor` / `--doctor`: diagnose install — MCP, CDP ports, engines,
    // vault, Python sidecar. Exit 0 healthy / 2 broken.
    if args.contains(&"--doctor".to_string()) || args.get(1).map(|s| s.as_str()) == Some("doctor") {
        std::process::exit(run_doctor(&rt));
    }

    match args.get(1).map(|s| s.as_str()) {
        Some("mcp") | None => {
            // `webrain mcp` = stdio; `webrain mcp --http <port>` = HTTP transport
            // with per-connection sessions (lightpanda mcp --port style).
            let http_port: Option<String> = args
                .iter()
                .position(|a| a == "--http")
                .and_then(|i| args.get(i + 1))
                .cloned();
            match http_port {
                Some(port) => {
                    let addr = format!("127.0.0.1:{port}");
                    tracing::info!("Starting webrain MCP server over HTTP on {addr}...");
                    rt.block_on(webrain_mcp::run_http(&addr))?;
                }
                None => {
                    tracing::info!("Starting webrain MCP server on stdio...");
                    rt.block_on(webrain_mcp::run_stdio())?;
                }
            }
        }
        Some("fetch") => {
            let url = args.get(2).map(|s| s.as_str()).unwrap_or("about:blank");
            let backend = rt.block_on(CdpBackend::connect_default())?;
            tracing::info!("Fetching: {url}");
            let state = rt.block_on(backend.navigate(url))?;
            println!("Title: {}", state.title);
            println!("Text:  {}", &state.text[..state.text.len().min(2000)]);
            println!("Elements: {} interactive", state.elements.len());
        }
        Some("screenshot") => {
            let url = args.get(2).map(|s| s.as_str()).unwrap_or("about:blank");
            let backend = rt.block_on(CdpBackend::connect_default())?;
            rt.block_on(backend.navigate(url))?;
            let png = rt.block_on(backend.screenshot(true))?;
            let out = format!("screenshot_{}.png", chrono_now());
            std::fs::write(&out, &png)?;
            println!("Saved: {out} ({} bytes)", png.len());
        }
        Some("spider") => {
            let seed = args.get(2).map(|s| s.as_str()).unwrap_or("about:blank");
            // lazy CLI flags: --dfs, --depth N, --pages N, --no-same-domain, --json-urls
            let json_urls = args.contains(&"--json-urls".to_string());
            let dfs = args.contains(&"--dfs".to_string());
            let depth: usize = args
                .iter()
                .position(|a| a == "--depth")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(2);
            let pages: usize = args
                .iter()
                .position(|a| a == "--pages")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(10);
            let same_domain = !args.contains(&"--no-same-domain".to_string());
            let discover_only = args.contains(&"--discover-only".to_string());
            let respect_robots = args.contains(&"--respect-robots".to_string());
            let bestfirst = args.contains(&"--bestfirst".to_string());
            let keywords: Vec<String> = args
                .iter()
                .position(|a| a == "--keywords")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.split(',').map(String::from).collect())
                .unwrap_or_default();
            let backend = rt.block_on(CdpBackend::connect_default())?;
            let spider = webrain_core::SpiderEngine::new(depth, pages)
                .with_strategy(if bestfirst {
                    webrain_core::CrawlStrategy::BestFirst
                } else if dfs {
                    webrain_core::CrawlStrategy::Dfs
                } else {
                    webrain_core::CrawlStrategy::Bfs
                })
                .with_same_domain(same_domain)
                .with_discover_only(discover_only)
                .with_respect_robots(respect_robots)
                .with_keywords(keywords);
            let results = rt.block_on(spider.crawl(&backend, seed));
            if json_urls {
                let all_urls: Vec<&String> = results.iter().flat_map(|r| r.links.iter()).collect();
                let unique: std::collections::BTreeSet<&&String> = all_urls.iter().collect();
                let urls: Vec<&String> = unique.into_iter().copied().collect();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"count": urls.len(), "urls": urls}))?
                );
            } else {
                for r in &results {
                    println!(
                        "[depth={}] {} — {} links",
                        r.depth,
                        r.page.url,
                        r.links.len()
                    );
                }
            }
        }
        Some("click") => {
            let index: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let backend = rt.block_on(CdpBackend::connect_default())?;
            rt.block_on(backend.click(index))?;
            println!("Clicked element {index}");
        }
        Some("type") => {
            let index: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
            let text = args.get(3).map(|s| s.as_str()).unwrap_or("");
            let backend = rt.block_on(CdpBackend::connect_default())?;
            rt.block_on(backend.type_text(index, text))?;
            println!("Typed into element {index}");
        }
        Some("eval") => {
            let js = args.get(2).map(|s| s.as_str()).unwrap_or("document.title");
            let backend = rt.block_on(CdpBackend::connect_default())?;
            let result = rt.block_on(backend.evaluate(js))?;
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Some("install") => {
            // webrain install [--force] [--engine chrome|obscura] [--stealth]
            // agent-browser-style: download the engine into a cache dir.
            let force = args.contains(&"--force".to_string());
            let stealth = args.contains(&"--stealth".to_string());
            let engine = args
                .iter()
                .position(|a| a == "--engine")
                .and_then(|i| args.get(i + 1))
                .cloned()
                .unwrap_or_else(|| "chrome".to_string());
            match engine.as_str() {
                "obscura" => {
                    let bin = webrain_core::install::install_obscura(force, stealth)?;
                    println!("engine ready: obscura at {}", bin.display());
                    println!("  start it: `webrain obscura`");
                }
                "lightpanda" => {
                    let bin = webrain_core::install::install_lightpanda(force)?;
                    println!("engine ready: lightpanda at {}", bin.display());
                    println!("  start it: `webrain lightpanda`");
                }
                "chrome" => {
                    let bin = webrain_core::install::install_chrome(force)?;
                    println!("engine ready: chrome (default) at {}", bin.display());
                }
                other => {
                    anyhow::bail!("unknown engine `{other}` — try chrome, obscura, or lightpanda")
                }
            }
        }
        Some("obscura") => {
            // webrain obscura [--port N] — spawn the Obscura CDP server
            let port: u16 = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(9224);
            let launched = webrain_core::launch::launch_obscura(port)?;
            println!("CDP_URL={}", launched.cdp_url);
            println!("engine: obscura — keeping alive; Ctrl-C to stop");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        Some("lightpanda") => {
            // webrain lightpanda [--port N] — spawn the lightpanda CDP server
            let port: u16 = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(9225);
            let launched = webrain_core::launch::launch_lightpanda(port)?;
            println!("CDP_URL={}", launched.cdp_url);
            println!("engine: lightpanda — keeping alive; Ctrl-C to stop");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        Some("launch") => {
            // webrain launch <service> <profile> [url] [--headless] [--port N]
            // Native replacement for the Python stealth_solve.py launch path:
            // spawns a stealth Chrome with a persistent per-account profile and
            // opens the site so the human can log in (Channel A).
            let service = args.get(2).cloned().unwrap_or_default();
            let profile = args.get(3).cloned().unwrap_or_default();
            let url = args
                .get(4)
                .map(|s| s.as_str())
                .unwrap_or("https://accounts.google.com");
            let headless = args.contains(&"--headless".to_string());
            let port: u16 = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(9222);
            if service.is_empty() || profile.is_empty() {
                println!("usage: webrain launch <service> <profile> [url] [--headless] [--port N]");
                return Ok(());
            }
            let launched =
                webrain_core::launch::launch_chrome(&service, &profile, port, !headless)?;
            println!("profile: {}", launched.profile_dir.display());
            println!("CDP_URL={}", launched.cdp_url);
            // Attach + navigate: applies STEALTH_JS + UA override, opens the site.
            let backend = rt.block_on(CdpBackend::connect_with_url(&launched.cdp_url))?;
            rt.block_on(backend.navigate(url))?;
            println!("opened: {url} — keeping alive; Ctrl-C to stop");
            loop {
                std::thread::sleep(std::time::Duration::from_secs(60));
            }
        }
        Some("login") => {
            // webrain login <service> <profile> [url] [--port N]
            // Native login: creds from the vault (in-process decrypt) or
            // $env:WEBRAIN_USER / $env:WEBRAIN_PASS. Secrets never in argv.
            let service = args.get(2).cloned().unwrap_or_default();
            let profile = args.get(3).cloned().unwrap_or_default();
            let url = args
                .get(4)
                .map(|s| s.to_string())
                .filter(|s| !s.starts_with("--"));
            let port: u16 = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(9222);
            if service.is_empty() || profile.is_empty() {
                println!("usage: webrain login <service> <profile> [url] [--port N]");
                return Ok(());
            }
            // Creds: vault first, else env vars (both referenced by name, never printed).
            let (user, pass, totp) = match webrain_core::vault::get(&service, &profile) {
                Ok(c) => (c.username, c.password, c.totp),
                Err(_) => (
                    std::env::var("WEBRAIN_USER").unwrap_or_default(),
                    std::env::var("WEBRAIN_PASS").unwrap_or_default(),
                    None,
                ),
            };
            if user.is_empty() || pass.is_empty() {
                println!(
                    "no credentials: run `webrain vault set {service} {profile}` or set WEBRAIN_USER/WEBRAIN_PASS"
                );
                return Ok(());
            }
            let cdp = format!("http://127.0.0.1:{port}");
            let backend = rt.block_on(CdpBackend::connect_with_url(&cdp))?;
            let result = rt.block_on(webrain_core::login::run_login(
                &backend,
                &user,
                &pass,
                totp.as_deref(),
                url.as_deref(),
            ))?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            if result.get("waiting_for_human").and_then(|v| v.as_bool()) == Some(true) {
                println!(
                    "2fa: human must approve/enter the code in the browser, then re-run: webrain login {service} {profile}"
                );
            }
        }
        Some("cookies") => {
            // webrain cookies [--port N] [--out file] — export cookies (incl.
            // HttpOnly) for cross-browser session migration.
            let port: u16 = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(9222);
            let out = args
                .iter()
                .position(|a| a == "--out")
                .and_then(|i| args.get(i + 1))
                .cloned();
            let backend = rt.block_on(CdpBackend::connect_with_url(&format!(
                "http://127.0.0.1:{port}"
            )))?;
            // Network.getCookies needs a page-level session (Chrome rejects it at the
            // browser endpoint). Open a blank tab to guarantee one exists.
            let _ = rt.block_on(backend.open_tab("about:blank"));
            let cookies = rt.block_on(backend.cookies())?;
            match out {
                Some(path) => {
                    std::fs::write(&path, serde_json::to_string_pretty(&cookies)?)?;
                    println!("wrote {} cookies to {path}", cookies.len());
                }
                None => println!(
                    "{}",
                    serde_json::to_string_pretty(
                        &json!({"count": cookies.len(), "cookies": cookies})
                    )?
                ),
            }
        }
        Some("setcookies") => {
            // webrain setcookies <file> [--port N] — import cookies into a browser
            // (obscura/lightpanda) for cross-browser session migration.
            let file = args.get(2).cloned().unwrap_or_default();
            if file.is_empty() {
                println!("usage: webrain setcookies <cookies.json> [--port N]");
                return Ok(());
            }
            let port: u16 = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(9222);
            let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
            let cookies: Vec<serde_json::Value> = data.as_array().cloned().unwrap_or_default();
            let backend = rt.block_on(CdpBackend::connect_with_url(&format!(
                "http://127.0.0.1:{port}"
            )))?;
            // Network.setCookies needs a page-level session too.
            let _ = rt.block_on(backend.open_tab("about:blank"));
            rt.block_on(backend.set_cookies(&cookies))?;
            // Same-connection readback: a fresh connection/tab may land in a
            // different browser context, so 0 here means the set itself failed
            // (vs. context isolation across connections).
            let back = rt.block_on(backend.cookies())?;
            // Second tab, same connection — tells per-connection vs per-tab
            // context isolation (obscura-style stealth browsers).
            let tab2 = rt.block_on(backend.open_tab("about:blank"));
            if let Ok(t2) = tab2 {
                let _ = rt.block_on(backend.activate_tab(&t2));
                let back2 = rt.block_on(backend.cookies())?;
                println!(
                    "set {} cookies on :{port} (readback: {}, tab2: {})",
                    cookies.len(),
                    back.len(),
                    back2.len()
                );
            } else {
                println!(
                    "set {} cookies on :{port} (readback: {})",
                    cookies.len(),
                    back.len()
                );
            }
        }
        Some("vault") => {
            match args.get(2).map(|s| s.as_str()).unwrap_or("") {
                "set" => {
                    let service = args.get(3).cloned().unwrap_or_default();
                    let profile = args.get(4).cloned().unwrap_or_default();
                    let username = args
                        .iter()
                        .position(|a| a == "--username")
                        .and_then(|i| args.get(i + 1))
                        .cloned()
                        .unwrap_or_default();
                    if service.is_empty() || profile.is_empty() {
                        println!(
                            "usage: webrain vault set <service> <profile> [--username <user>]"
                        );
                        return Ok(());
                    }
                    // secrets come from hidden prompts — never argv, never chat, never logs
                    let password = rpassword::prompt_password("Password: ")?;
                    let totp_raw = rpassword::prompt_password(
                        "TOTP secret (base32, optional; Enter to skip): ",
                    )?;
                    let totp = if totp_raw.trim().is_empty() {
                        None
                    } else {
                        Some(totp_raw.trim().to_string())
                    };
                    webrain_core::vault::set(&service, &profile, &username, &password, totp)?;
                    println!("vault: {service}/{profile} stored");
                }
                "list" => {
                    for m in webrain_core::vault::list()? {
                        println!(
                            "{:<12} {:<20} {:<24} {}",
                            m.service, m.profile, m.username, m.created_at
                        );
                    }
                }
                "rm" => {
                    let service = args.get(3).cloned().unwrap_or_default();
                    let profile = args.get(4).cloned().unwrap_or_default();
                    webrain_core::vault::remove(&service, &profile)?;
                    println!("vault: {service}/{profile} removed");
                }
                "user" => {
                    let service = args.get(3).cloned().unwrap_or_default();
                    let profile = args.get(4).cloned().unwrap_or_default();
                    let username = args.get(5).cloned().unwrap_or_default();
                    if service.is_empty() || profile.is_empty() || username.is_empty() {
                        println!("usage: webrain vault user <service> <profile> <username>");
                        return Ok(());
                    }
                    webrain_core::vault::set_username(&service, &profile, &username)?;
                    println!("vault: {service}/{profile} username set to {username}");
                }
                other => println!("unknown vault subcommand: {other} (use: set|list|user|rm)"),
            }
        }
        Some("upgrade") => {
            upgrade()?;
        }
        Some("-v" | "--version" | "version") => {
            println!("webrain {}", env!("CARGO_PKG_VERSION"));
        }
        Some(cmd) => {
            println!("Unknown command: {cmd}");
            println!(
                "Usage: webrain [mcp|doctor|install|obscura|lightpanda|launch|login|cookies|setcookies|fetch <url>|screenshot <url>|spider <seed_url>|click <i>|type <i> <text>|eval <js>|vault <set|list|user|rm>|upgrade]"
            );
            println!();
            println!("Set CDP_URL to point to your browser:");
            println!("  Chrome:  --remote-debugging-port=9222");
            println!("  Obscura: obscura serve --stealth");
        }
    }

    Ok(())
}

// ── webrain upgrade ──────────────────────────────────────────────
// Delegates to the package manager when installed through one (Homebrew /
// Scoop), otherwise self-updates the running binary in place.
fn upgrade() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if cmd_exists("brew") {
        println!("webrain: installed via Homebrew \u{2014} running `brew upgrade webrain`");
        return run_cmd("brew", &["upgrade", "webrain"]).map_err(Into::into);
    }
    #[cfg(target_os = "windows")]
    if cmd_exists("scoop")
        && std::path::Path::new(&std::env::var("USERPROFILE").unwrap_or_default())
            .join("scoop/apps/webrain")
            .exists()
    {
        println!("webrain: installed via Scoop \u{2014} running `scoop update webrain`");
        // scoop is a .cmd/.ps1 shim, not scoop.exe — spawn it through cmd.exe
        return run_cmd("cmd", &["/c", "scoop", "update", "webrain"]).map_err(Into::into);
    }
    self_update()
}

fn cmd_exists(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p)
            .any(|d| d.join(name).exists() || d.join(format!("{name}.exe")).exists())
    })
}

fn run_cmd(prog: &str, args: &[&str]) -> std::io::Result<()> {
    let st = std::process::Command::new(prog).args(args).status()?;
    if !st.success() {
        return Err(std::io::Error::other(format!("{prog} exited with {st}")));
    }
    Ok(())
}

fn self_update() -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let asset = if cfg!(target_os = "linux") {
        "webrain-linux"
    } else if cfg!(target_os = "macos") {
        "webrain-macos"
    } else {
        "webrain-windows.exe"
    };
    let url = format!("https://github.com/prokopis3/webrain/releases/latest/download/{asset}");
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("no parent directory for current binary"))?;
    let tmp = dir.join(format!("webrain.new.{}", std::process::id()));
    println!("webrain: downloading {url}");
    let st = std::process::Command::new("curl")
        .arg("-fsSL")
        .arg("-o")
        .arg(&tmp)
        .arg(&url)
        .status()?;
    if !st.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::anyhow!("download failed (curl exit {st})"));
    }
    #[cfg(unix)]
    {
        std::fs::rename(&tmp, &exe)?;
    }
    #[cfg(windows)]
    {
        // A running .exe can be renamed but not overwritten: move the old
        // one aside, put the new one in place, then drop the old.
        let old = dir.join("webrain.old.exe");
        let _ = std::fs::remove_file(&old);
        std::fs::rename(&exe, &old)?;
        if std::fs::rename(&tmp, &exe).is_err() {
            let _ = std::fs::rename(&old, &exe);
            return Err(anyhow::anyhow!("could not replace webrain.exe"));
        }
        let _ = std::fs::remove_file(&old);
    }
    println!("webrain: updated to the latest release.");
    Ok(())
}

fn chrono_now() -> String {
    use std::time::SystemTime;
    let t = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", t.as_secs())
}

// ponytail: TCP connect probe — try opening, return true if port is listening.
async fn check_tcp(host: &str, port: u16) -> bool {
    tokio::net::TcpStream::connect((host, port)).await.is_ok()
}

// ponytail: GET /json/version on a CDP port via raw TCP, extract Browser name.
// Zero deps — one HTTP request, parse the JSON Browser field.
async fn cdp_browser_name(port: u16) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    match tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
        Ok(mut s) => {
            let req = format!("GET /json/version HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
            if s.write_all(req.as_bytes()).await.is_err() {
                return "unknown".into();
            }
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).await.unwrap_or(0);
            let body = String::from_utf8_lossy(&buf[..n]);
            // find \r\n\r\n separator, take JSON after it
            if let Some(pos) = body.find("\r\n\r\n") {
                let json_str = &body[pos + 4..];
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                    return v
                        .get("Browser")
                        .and_then(|b| b.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                }
            }
            "unknown".into()
        }
        Err(_) => "unknown".into(),
    }
}

/// `webrain doctor` — diagnose the install: version, MCP server, CDP ports,
/// engines (agent-browser-style discovery), encrypted vault, Python sidecar.
/// Exit 0 if a browser is reachable, 2 otherwise.
fn run_doctor(rt: &tokio::runtime::Runtime) -> i32 {
    let cdp_ports = [9222u16, 9224u16, 9225u16];
    let mut any_cdp = false;
    println!("webrain doctor");
    println!("  version          {}", env!("CARGO_PKG_VERSION"));
    println!(
        "  mcp server       {}",
        if rt.block_on(async { check_tcp("127.0.0.1", 9223).await }) {
            "✅ (http://127.0.0.1:9223)"
        } else {
            "❌ (not running)"
        }
    );
    for &port in &cdp_ports {
        let up = rt.block_on(async { check_tcp("127.0.0.1", port).await });
        if up {
            any_cdp = true;
        }
        let label = if up {
            let browser = rt.block_on(async { cdp_browser_name(port).await });
            format!("✅ ({browser})")
        } else {
            "❌ (no browser)".into()
        };
        println!("  cdp port {port}      {label}");
    }
    let ch = webrain_core::install::find_cft_chrome();
    println!(
        "  engine chrome     {}",
        match &ch {
            Some(p) => format!("✅ {}", p.display()),
            None => "❌ — run `webrain install`".into(),
        }
    );
    println!(
        "  engine obscura    {}",
        match webrain_core::install::find_obscura() {
            Some(p) => format!("✅ {}", p.display()),
            None => "⚠️ — `webrain install --engine obscura` or Docker".into(),
        }
    );
    println!(
        "  engine lightpanda {}",
        match webrain_core::install::find_lightpanda() {
            Some(p) => format!("✅ {}", p.display()),
            None => "⚠️ — install the binary or set WEBRAIN_LIGHTPANDA".into(),
        }
    );
    println!(
        "  vault             {}",
        if webrain_core::vault::vault_dir().join("vault.json").exists() {
            "✅ present"
        } else {
            "⚠️ empty — `webrain vault set`".into()
        }
    );
    let py_ok = std::process::Command::new("python")
        .arg("-c")
        .arg("import playwright, undetected_playwright")
        .output()
        .is_ok();
    println!(
        "  stealth_solve     {}",
        if py_ok {
            "✅ (Python + playwright + undetected_playwright)"
        } else {
            "⚠️ deps missing — pip install playwright undetected_playwright && playwright install chromium".into()
        }
    );
    let rec = if rt.block_on(async { check_tcp("127.0.0.1", 9222).await }) {
        "chrome (9222)"
    } else if rt.block_on(async { check_tcp("127.0.0.1", 9224).await }) {
        "obscura (9224)"
    } else if rt.block_on(async { check_tcp("127.0.0.1", 9225).await }) {
        "lightpanda (9225)"
    } else if ch.is_some() {
        "chrome — set CDP_URL or spawn an engine (`webrain lightpanda`/`webrain obscura`)"
    } else {
        "none — run `webrain install`, then start an engine"
    };
    println!("  recommend        {rec}");
    if any_cdp { 0 } else { 2 }
}
