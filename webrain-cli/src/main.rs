// webrain-cli: Single binary — `webrain mcp` | `webrain fetch` | `webrain screenshot` | `webrain spider`
//
// ponytail: one binary, subcommands via match, no clap dependency.

use webrain_core::browser::BrowserBackend;
use webrain_core::CdpBackend;
use std::env;
use serde_json::json;

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

    // --doctor: probe CDP ports + MCP status, exit 0 if healthy, 2 if not.
    if args.contains(&"--doctor".to_string()) {
        let cdp_ports = [9222u16, 9224u16];
        let mut all_ok = true;
        println!("webrain --doctor");
        println!("  mcp server       {}", if rt.block_on(async { check_tcp("127.0.0.1", 9223).await }) { "✅ (http://127.0.0.1:9223)" } else { "❌ (not running)" });
        for &port in &cdp_ports {
            let up = rt.block_on(async { check_tcp("127.0.0.1", port).await });
            let label = if up {
                let browser = rt.block_on(async { cdp_browser_name(port).await });
                format!("✅ ({browser})")
            } else { "❌ (no browser)".into() };
            println!("  cdp port {port}      {label}");
            if !up { all_ok = false; }
        }
        if all_ok {
            println!("  recommend        obscura");
        } else {
            println!("  recommend        none — start one: docker start obscura");
        }
        println!("  cargo version    {}  (latest: check crates.io)", env!("CARGO_PKG_VERSION"));
        // stealth_solve probe: just check if python + deps exist
        let py_ok = std::process::Command::new("python").arg("-c").arg("import playwright, undetected_playwright").output().is_ok();
        println!("  stealth_solve    {}", if py_ok { "✅ (Python + playwright + undetected_playwright)" } else { "⚠️  (deps missing; install: pip install playwright undetected-playwright && playwright install chromium)" });
        std::process::exit(if all_ok { 0 } else { 2 });
    }

    match args.get(1).map(|s| s.as_str()) {
        Some("mcp") | None => {
            // `webrain mcp` = stdio; `webrain mcp --http <port>` = HTTP transport
            // with per-connection sessions (lightpanda mcp --port style).
            let http_port: Option<String> = args.iter().position(|a| a == "--http")
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
            let depth: usize = args.iter().position(|a| a == "--depth")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(2);
            let pages: usize = args.iter().position(|a| a == "--pages")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(10);
            let same_domain = !args.contains(&"--no-same-domain".to_string());
            let discover_only = args.contains(&"--discover-only".to_string());
            let respect_robots = args.contains(&"--respect-robots".to_string());
            let bestfirst = args.contains(&"--bestfirst".to_string());
            let keywords: Vec<String> = args.iter().position(|a| a == "--keywords")
                .and_then(|i| args.get(i + 1))
                .map(|s| s.split(',').map(String::from).collect())
                .unwrap_or_default();
            let backend = rt.block_on(CdpBackend::connect_default())?;
            let spider = webrain_core::SpiderEngine::new(depth, pages)
                .with_strategy(if bestfirst { webrain_core::CrawlStrategy::BestFirst } else if dfs { webrain_core::CrawlStrategy::Dfs } else { webrain_core::CrawlStrategy::Bfs })
                .with_same_domain(same_domain)
                .with_discover_only(discover_only)
                .with_respect_robots(respect_robots)
                .with_keywords(keywords);
            let results = rt.block_on(spider.crawl(&backend, seed));
            if json_urls {
                let all_urls: Vec<&String> = results.iter().flat_map(|r| r.links.iter()).collect();
                let unique: std::collections::BTreeSet<&&String> = all_urls.iter().collect();
                let urls: Vec<&String> = unique.into_iter().copied().collect();
                println!("{}", serde_json::to_string_pretty(&json!({"count": urls.len(), "urls": urls}))?);
            } else {
                for r in &results {
                    println!("[depth={}] {} — {} links", r.depth, r.page.url, r.links.len());
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
        Some(cmd) => {
            println!("Unknown command: {cmd}");
            println!("Usage: webrain [mcp|fetch <url>|read <url>|markdown <url>|screenshot <url>|spider <seed_url>|click <i>|type <i> <text>|eval <js>]");
            println!();
            println!("Set CDP_URL to point to your browser:");
            println!("  Chrome:  --remote-debugging-port=9222");
            println!("  Obscura: obscura serve --stealth");
        }
    }

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
            if s.write_all(req.as_bytes()).await.is_err() { return "unknown".into(); }
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).await.unwrap_or(0);
            let body = String::from_utf8_lossy(&buf[..n]);
            // find \r\n\r\n separator, take JSON after it
            if let Some(pos) = body.find("\r\n\r\n") {
                let json_str = &body[pos + 4..];
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) {
                    return v.get("Browser").and_then(|b| b.as_str()).unwrap_or("unknown").to_string();
                }
            }
            "unknown".into()
        }
        Err(_) => "unknown".into(),
    }
}
