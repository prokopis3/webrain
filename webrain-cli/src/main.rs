// webrain-cli: Single binary — `webrain mcp` | `webrain fetch` | `webrain screenshot` | `webrain spider`
//
// ponytail: one binary, subcommands via match, no clap dependency.

use serde_json::json;
use std::env;
use webrain_core::CdpBackend;
use webrain_core::browser::BrowserBackend;

/// One row per subcommand: (name, usage, one-line about, extra notes).
///
/// Help lives in ONE table used by both the overview and the per-command page, so
/// a new subcommand is described in one place instead of a usage `println!` per
/// arm — and `webrain <cmd> --help` can never drift from `webrain --help`.
const HELP: &[(&str, &str, &str, &str)] = &[
    (
        "mcp",
        "webrain mcp [--http <port>]",
        "MCP server over stdio (default), or HTTP on 127.0.0.1:<port> for networked clients.",
        "",
    ),
    (
        "install",
        "webrain install [--engine chrome|obscura|lightpanda] [--stealth] [--force]\n    webrain install watch|whisper [--model NAME]\n    webrain install vision [--model ID] [--model-update]",
        "Download a browser engine, or the local AI stack (ffmpeg, yt-dlp, whisper, Qwen3-VL).",
        "--model ID picks one of the local vision models (an unknown id prints the\n  choices); --model-update re-checks the model files against Hugging Face.",
    ),
    (
        "uninstall",
        "webrain uninstall vision [--model ID | --all]",
        "Remove an installed local vision model, or all of them.",
        "",
    ),
    (
        "doctor",
        "webrain doctor",
        "Diagnose the install: engines, CDP ports, MCP server, vault. Exit 0 healthy, 2 broken.",
        "",
    ),
    (
        "upgrade",
        "webrain upgrade",
        "Update webrain, detected per install method (brew, scoop, or self-update).",
        "",
    ),
    (
        "launch",
        "webrain launch <service> <profile> [url] [--headless] [--port N]",
        "Stealth Chrome with a persistent per-account profile.",
        "",
    ),
    (
        "login",
        "webrain login <service> <profile> [url] [--port N]",
        "Attach for an interactive login; credentials come from the vault (optional TOTP).",
        "",
    ),
    (
        "vault",
        "webrain vault set|list|user|rm",
        "Encrypted credential vault (AES-256-GCM). Secrets are prompted, never echoed.",
        "",
    ),
    (
        "cookies",
        "webrain cookies [--port N] [--out FILE]",
        "Export every cookie, including HttpOnly ones document.cookie cannot see.",
        "",
    ),
    (
        "setcookies",
        "webrain setcookies <FILE> [--port N]",
        "Import cookies into the CDP backend — cross-engine session migration.",
        "",
    ),
    (
        "fetch",
        "webrain fetch <url>",
        "Attach to CDP_URL and print the page text plus interactive elements.",
        "",
    ),
    (
        "screenshot",
        "webrain screenshot <url>",
        "Full-page screenshot written to disk.",
        "",
    ),
    (
        "spider",
        "webrain spider <url> [--depth N] [--pages N] [--respect-robots] [--concurrency N]",
        "Crawl a site from a seed URL.",
        "",
    ),
    (
        "click",
        "webrain click <index>",
        "Click an element by its index from fetch.",
        "",
    ),
    (
        "type",
        "webrain type <index> <text>",
        "Type into an element by index.",
        "",
    ),
    (
        "eval",
        "webrain eval <js>",
        "Evaluate JS in the current page and print the result.",
        "",
    ),
    (
        "serp",
        "webrain serp \"query\" [--engine duckduckgo|bing|google|brave|yahoo|baidu|yandex|auto]\n    [--limit N] [--page N] [--safe] [--region R] [--http] [--no-fallback] [--json]\n    [--proxy URL] [--headless] [--fresh] [--pipe] [--hold]",
        "Structured SERP JSON: one query per call, pages fetched sequentially.",
        "duckduckgo|bing|yahoo|baidu|yandex|auto need no browser. brave uses its keyed\n  API when BRAVE_API_KEY is set, else the connected CDP engine. google auto-launches\n  a Chrome with a persistent profile unless --fresh. --fresh starts a new profile and\n  cookies every run; --pipe makes that launch use --remote-debugging-pipe (no open\n  debugging port); --hold keeps it open afterwards. --http never touches a browser at\n  all: no render, no guest launch, whatever is attached — the plain-HTML engines answer\n  in under a second each that way. Measured ceilings without a browser: bing 10 (its\n  RSS feed and its HTML SERP both ignore paging), duckduckgo ~20 (two distinct pages,\n  then repeats), yahoo 25+ (pages on b=11,21,...  so a deep limit works), baidu 10\n  before its security check. baidu and yandex are the walled two: baidu serves ~7\n  organic rows and then 302s to wappass.baidu.com (百度安全验证 image captcha), and\n  yandex serves a SmartCaptcha page from a datacenter IP — both are reported as\n  challenges, never as \"0 results\". A real Chrome with a warm profile passes the\n  baidu check; a FRESH or headless profile does not (measured 2026-10-26: guest Chrome\n  7 baidu rows / 0 yandex, fresh + headless both walled). For many queries, issue\n  several calls (or webrain_batch over SERP URLs) — the CLI takes one query.",
    ),
    (
        "watch",
        "webrain watch <video-url-or-file> [--vision] [--detail D] [--start S] [--end S]",
        "Timestamped transcript plus frames, no browser needed.",
        "",
    ),
    (
        "obscura",
        "webrain obscura [--port N]",
        "Spawn the Obscura CDP server (default 9224).",
        "",
    ),
    (
        "lightpanda",
        "webrain lightpanda [--port N]",
        "Spawn the Lightpanda CDP server (default 9225).",
        "",
    ),
];

/// Render the overview (no command) or one command's page.
fn help_page(cmd: &str) -> String {
    let cmd = cmd.trim_start_matches('-');
    let cmd = if cmd == "h" { "help" } else { cmd };
    if cmd.is_empty() || cmd == "help" {
        let mut s = String::from(
            "webrain — portable MCP browser-automation & scraping server\n\nUSAGE\n  webrain <command> [args]\n  webrain <command> --help\n\nCOMMANDS\n",
        );
        for (name, _, about, _) in HELP {
            s.push_str(&format!("  {name:<12} {about}\n"));
        }
        s.push_str(
            "\nRun `webrain <command> --help` for one command's flags.\n\nENVIRONMENT (read at process start, restart to apply; full table with\nplatform defaults in docs/reference/environment.mdx):\n  CDP_URL                     CDP endpoint to attach (default 127.0.0.1:9222)\n  WEBRAIN_STEALTH_NOISE=0     drop the fingerprint-noise block (hydration fix)\n  WEBRAIN_NO_STEALTH=1        drop all injected stealth JS (diagnostic only)\n  WEBRAIN_PROFILES_DIR        persistent profile root (keeps auth/challenges)\n  WEBRAIN_VAULT_DIR           encrypted credential vault dir\n  WEBRAIN_BROWSERS_DIR        engine cache (Chrome for Testing, obscura)\n  WEBRAIN_CHROME / WEBRAIN_LIGHTPANDA / WEBRAIN_OBSCURA\n                              explicit engine binary paths\n  WEBRAIN_PROXIES             proxy rotation pool for the HTTP engines\n  WEBRAIN_SKILLS_DIR          operator .md skills read by webrain_guide\n  WEBRAIN_HTTP_TOKEN          bearer token for `mcp --http`\n  WEBRAIN_NO_STATE=1          shared-instance mode (no vault/session tools)\n",
        );
        return s;
    }
    match HELP.iter().find(|(name, _, _, _)| *name == cmd) {
        Some((name, usage, about, notes)) => {
            let mut s = format!("webrain {name} — {about}\n\nUSAGE\n  {usage}\n");
            if !notes.is_empty() {
                s.push_str(&format!("\nNOTES\n  {notes}\n"));
            }
            s
        }
        None => format!("unknown command: {cmd}\n\n{}", help_page("")),
    }
}

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
    // Help before anything else: `webrain --help`, `webrain <cmd> --help`,
    // `webrain help <cmd>`. No side effects, so it works before engines exist.
    if args.iter().skip(1).any(|a| a == "--help" || a == "-h")
        || args.get(1).map(|s| s.as_str()) == Some("help")
    {
        let first = args.get(1).map(|s| s.as_str()).unwrap_or("");
        let cmd = if first == "help" {
            args.get(2).map(|s| s.as_str()).unwrap_or("")
        } else {
            first
        };
        print!("{}", help_page(cmd));
        return Ok(());
    }
    // google serp runs with no injected stealth JS. edition-2024 set_var is
    // unsafe because concurrent env reads (in tokio's worker pool) are a data
    // race — set it HERE, before Runtime::new() spawns any threads.
    //
    // Resolve the engine the same way the serp arm does (default = auto), so
    // the DEFAULT `webrain serp "q"` invocation — which can join google via the
    // browser path when one is attached — also gets no-stealth, and so do
    // case-variant / `--engine=google` spellings.
    let is_serp = args.get(1).map(|s| s.as_str()) == Some("serp");
    let engine_flag = args
        .windows(2)
        .find(|w| w[0] == "--engine")
        .map(|w| w[1].to_ascii_lowercase());
    let engine_eq = args
        .iter()
        .find_map(|a| a.strip_prefix("--engine="))
        .map(|v| v.to_ascii_lowercase());
    let no_stealth_engine = engine_flag
        .or(engine_eq)
        .map(|e| e == "google" || e == "auto")
        .unwrap_or(true); // no --engine → defaults to auto
    if is_serp && no_stealth_engine {
        unsafe { std::env::set_var("WEBRAIN_NO_STEALTH", "1") };
    }
    let rt = tokio::runtime::Runtime::new()?;

    // `webrain doctor` / `--doctor`: diagnose install — MCP, CDP ports, engines,
    // vault. Exit 0 healthy / 2 broken.
    if args.contains(&"--doctor".to_string()) || args.get(1).map(|s| s.as_str()) == Some("doctor") {
        std::process::exit(run_doctor(&rt));
    }

    match args.get(1).map(|s| s.as_str()) {
        Some("mcp") | None => {
            // `webrain mcp` = stdio (also the no-arg default: Docker's
            // `ENTRYPOINT ["webrain"]`); `webrain mcp --http <port>` = HTTP
            // transport with per-connection sessions (lightpanda mcp --port style).
            let http_port: Option<String> = args
                .iter()
                .position(|a| a == "--http")
                .and_then(|i| args.get(i + 1))
                .cloned();
            match http_port {
                Some(port) => {
                    // Default to loopback only. Override with WEBRAIN_HTTP_ADDR
                    // (e.g. "0.0.0.0:9223" inside Docker, where the host publishes
                    // 127.0.0.1:9223 → container). Never bind the unauthenticated
                    // MCP endpoint to a LAN by default.
                    let addr = std::env::var("WEBRAIN_HTTP_ADDR")
                        .unwrap_or_else(|_| format!("127.0.0.1:{port}"));
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
            // Slicing at an arbitrary byte offset panics if the cut lands mid
            // multi-byte UTF-8 (any non-ASCII page > 2000 B), so walk back to the
            // nearest char boundary. `str::floor_char_boundary` is the direct
            // call, but it is still unstable — rustc 1.88 rejects it with E0658,
            // and a `--release` build treats that as a hard error — so this is
            // the stable equivalent. 0 is always a boundary, hence unwrap_or(0).
            let cap = state.text.len().min(2000);
            let end = (0..=cap)
                .rev()
                .find(|&i| state.text.is_char_boundary(i))
                .unwrap_or(0);
            println!("Text:  {}", &state.text[..end]);
            println!("Elements: {} interactive", state.elements.len());
        }
        Some("screenshot") => {
            let url = args.get(2).map(|s| s.as_str()).unwrap_or("about:blank");
            let backend = rt.block_on(CdpBackend::connect_default())?;
            rt.block_on(backend.navigate(url))?;
            let png = rt.block_on(backend.screenshot(true))?;
            // Sub-second precision so two runs in the same second don't
            // silently overwrite each other's screenshot.
            let out = format!(
                "screenshot_{}_{}.png",
                chrono_now(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_millis())
                    .unwrap_or(0)
            );
            std::fs::write(&out, &png)?;
            println!("Saved: {out} ({} bytes)", png.len());
        }
        Some("spider") => {
            let seed = args.get(2).map(|s| s.as_str()).unwrap_or("about:blank");
            // lazy CLI flags: --dfs, --depth N, --pages N, --concurrency N,
            // --no-same-domain, --json-urls
            let json_urls = args.contains(&"--json-urls".to_string());
            let dfs = args.contains(&"--dfs".to_string());
            // Parallel multi-tab crawl (matches webrain_crawl's concurrency): N
            // workers on multi-target backends (Chrome/obscura); 1/single-target
            // (lightpanda) = sequential, the legacy default.
            let concurrency: usize = args
                .iter()
                .position(|a| a == "--concurrency")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(1);
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
                .with_keywords(keywords)
                .with_concurrency(concurrency);
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
        Some("watch") => {
            // webrain watch <video-url-or-path> [--detail transcript|efficient|balanced]
            //   [--max-frames N] [--resolution W] [--start S] [--end E]
            //   [--out-dir D] [--no-whisper] [--stt whisper|gemini] [--vision]
            // No browser needed: yt-dlp captions -> Whisper fallback + ffmpeg frames.
            let source = args.get(2).cloned().unwrap_or_default();
            if source.is_empty() {
                println!(
                    "usage: webrain watch <video-url-or-path> [--detail transcript|efficient|balanced] [--max-frames N] [--resolution W] [--start S] [--end E] [--out-dir D] [--no-whisper] [--stt whisper|gemini] [--vision]"
                );
                println!(
                    "  STT: local whisper-cli (whisper.cpp) when installed + model (`webrain install whisper`); cloud: GROQ_API_KEY | OPENAI_API_KEY | FIREWORKS_API_KEY (model: WEBRAIN_STT_MODEL)"
                );
                println!(
                    "  Vision: [--vision] sends sampled frames to a vision LLM (GROQ_API_KEY | OPENAI_API_KEY) and prints text captions + visual summary"
                );
                return Ok(());
            }
            let flag = |name: &str| -> Option<String> {
                args.iter()
                    .position(|a| a == name)
                    .and_then(|i| args.get(i + 1))
                    .cloned()
            };
            use webrain_core::video::{Detail, SttBackend, WatchOpts};
            let opts = WatchOpts {
                detail: Detail::parse(flag("--detail").as_deref().unwrap_or("balanced")),
                max_frames: flag("--max-frames").and_then(|v| v.parse().ok()),
                resolution: flag("--resolution")
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(512),
                start: flag("--start").and_then(|v| v.parse().ok()),
                end: flag("--end").and_then(|v| v.parse().ok()),
                out_dir: flag("--out-dir"),
                no_whisper: args.contains(&"--no-whisper".to_string()),
                stt_backend: SttBackend::parse(flag("--stt").as_deref().unwrap_or("whisper")),
                vision: args.contains(&"--vision".to_string()),
            };
            let t0 = std::time::Instant::now();
            let result = webrain_core::video::watch(&source, &opts)?;
            println!("{}", serde_json::to_string_pretty(&result)?);
            eprintln!("watch done in {} ms", t0.elapsed().as_millis());
        }
        Some("serp") => {
            // webrain serp "query"
            //   [--engine duckduckgo|bing|google|brave|yahoo|baidu|yandex|auto]
            //   [--limit N] [--page N] [--safe] [--region R] [--no-fallback] [--json]
            // Structured SERP JSON. duckduckgo/bing/yahoo/baidu/yandex/auto work
            // over plain HTTP (no browser); google/brave render in the connected
            // CDP engine (CDP_URL / --remote-debugging-port=9222, google
            // auto-launches guest Chrome). baidu/yandex fall back to a browser
            // render only when their HTTP reply is a challenge page.
            let query = args.get(2).cloned().unwrap_or_default();
            if query.trim().is_empty() {
                print!("{}", help_page("serp"));
                return Ok(());
            }
            let flag = |name: &str| -> Option<String> {
                args.iter()
                    .position(|a| a == name)
                    .and_then(|i| args.get(i + 1))
                    .cloned()
            };
            let engine = flag("--engine").unwrap_or_else(|| "auto".to_string());
            let limit: usize = flag("--limit")
                .and_then(|v| v.parse().ok())
                .unwrap_or(10)
                .clamp(1, 100);
            let page: usize = flag("--page").and_then(|v| v.parse().ok()).unwrap_or(0);
            let safe = args.contains(&"--safe".to_string());
            let region = flag("--region");
            let fallback = !args.contains(&"--no-fallback".to_string());
            let json_out = args.contains(&"--json".to_string());
            let headless = args.contains(&"--headless".to_string());
            let proxy = flag("--proxy");
            // --http: never touch a browser. duckduckgo/bing answer over plain
            // HTTP (0.5-0.7s each), so a caller asking only for them must not
            // pay a Chrome launch — and `--engine bing --http` stays HTTP even
            // though bing otherwise counts as a browser engine here.
            let http_only = args.contains(&"--http".to_string());
            // --fresh: always start a brand-new google profile + cookies (never
            // attach a warm browser) so the consent modal ALWAYS appears and is
            // always dismissed — the deterministic anti-bot recipe.
            let fresh = args.contains(&"--fresh".to_string());
            // --pipe: launch the fresh google Chrome via `--remote-debugging-pipe`
            // (stdin/stdout, NO open debugging port) — an open debugging port is
            // the automation fingerprint Google walls on /sorry; a pipe-launched
            // Chrome has none. Only meaningful with --fresh.
            let pipe = args.contains(&"--pipe".to_string());
            // --stealth is accepted for CLI compat but is a no-op (the launch-flag
            // stealth was removed — it's a detectable fingerprint). google now
            // auto-launches in GUEST MODE (fresh ephemeral session, no persisted
            // profile state) with CDP-level masking (stealth_js).
            let _stealth = args.contains(&"--stealth".to_string());
            let opts = webrain_core::serp::SerpOpts {
                query: query.trim().to_string(),
                engine,
                limit,
                page,
                safe,
                region,
                retries: 2,
                fallback,
                proxy: proxy.clone(),
                http_only,
            };
            // google: trusted-commands-only flow — no injected stealth JS
            // (WEBRAIN_NO_STEALTH, set before the runtime at the top of main)
            // and no Runtime.evaluate in the page. Other engines (bing/ddg HTTP,
            // brave attached browser) are untouched.
            // google is JS-gated (consent/JS shell over plain HTTP) — same as
            // brave, it needs a real browser. For google, if none is attached,
            // auto-launch Chrome in GUEST MODE (fresh ephemeral session every
            // run — zero persisted cookies/history, consent modal always renders)
            // so the engine always exists, headed or headless alike. The browser
            // stays alive as a warm session.
            // --hold keeps the FRESH browser alive past the search so you can
            // watch it; only fresh holds — the warm 9222 guest keeps its session.
            let hold = args.contains(&"--hold".to_string());
            let mut _launched: Option<webrain_core::launch::Launched> = None;
            let backend = if !http_only
                && (opts.engine == "brave" || opts.engine == "google" || opts.engine == "bing")
            {
                if fresh && opts.engine == "google" {
                    // --fresh: a brand-new profile dir + unique port every run —
                    // zero cookies, so the consent modal always renders and
                    // CONSENT_DISMISS_JS always dismisses it before the humanized
                    // flow. Never touches a running 9222 chrome.
                    let prof_name = format!("google_fresh_{}_{}", chrono_now(), std::process::id());
                    if pipe {
                        // --remote-debugging-pipe: CDP over stdin/stdout, no
                        // listening port -> no open-debugger fingerprint (the
                        // thing that walls google). The child is owned by
                        // connect_pipe and killed when the run ends.
                        if proxy.is_some() {
                            anyhow::bail!(
                                "--fresh --pipe does not support --proxy (pipe Chrome can't bake --proxy-server here)"
                            );
                        }
                        let child = rt.block_on(webrain_core::launch::launch_chrome_pipe(
                            "serp", &prof_name, !headless,
                        ))?;
                        if !json_out {
                            println!(
                                "launched FRESH chrome via --remote-debugging-pipe (no debugging port -> no open-debugger fingerprint): profile serp/{}",
                                prof_name
                            );
                        }
                        Some(rt.block_on(CdpBackend::connect_pipe(child))?)
                    } else {
                        let port = pick_free_port(9230);
                        // GUEST MODE: fresh ephemeral session every run — zero
                        // cookies/history, consent modal always renders.
                        let launched = webrain_core::launch::launch_chrome_guest(
                            "serp",
                            &prof_name,
                            port,
                            !headless,
                            proxy.as_deref(),
                        )?;
                        if !json_out {
                            println!(
                                "launched FRESH chrome (no cookies -> consent modal handled): {} (CDP_URL={})",
                                launched.profile_dir.display(),
                                launched.cdp_url
                            );
                        }
                        // keep it alive for the whole run (+ --hold) so the headed
                        // window actually stays visible — dropping `Launched` kills
                        // Chrome, which was tearing the window down right after connect.
                        _launched = Some(launched);
                        Some(rt.block_on(CdpBackend::connect_with_url(
                            _launched.as_ref().expect("fresh launched").cdp_url.as_str(),
                        ))?)
                    }
                } else {
                    match rt.block_on(CdpBackend::connect_default()) {
                        Ok(b) => Some(b),
                        Err(_)
                            if opts.engine == "google"
                                || (opts.engine == "brave"
                                    && !webrain_core::serp::brave_api_ready())
                                || opts.engine == "bing" =>
                        {
                            // GUEST MODE auto-launch for all browser engines
                            // (--proxy bakes --proxy-server in when set): fresh
                            // ephemeral session every run — zero persisted
                            // cookies/history. ddg never reaches this branch
                            // (backend = None, pure HTTP).
                            let prof = if opts.engine == "google" {
                                "google"
                            } else if opts.engine == "bing" {
                                "bing"
                            } else {
                                "brave"
                            };
                            match webrain_core::launch::launch_chrome_guest(
                                "serp",
                                prof,
                                9222,
                                !headless,
                                proxy.as_deref(),
                            ) {
                                Ok(launched) => {
                                    if !json_out {
                                        println!(
                                            "launched chrome (guest): {} (CDP_URL={})",
                                            launched.profile_dir.display(),
                                            launched.cdp_url
                                        );
                                    }
                                    // WARM SESSION — the guest Chrome stays alive on 9222
                                    // between runs (its in-memory session persists).
                                    // Forget the handle so Drop::kill() never runs.
                                    // `--fresh` is the explicit opt-out for a fresh guest.
                                    let url = launched.cdp_url.clone();
                                    std::mem::forget(launched);
                                    Some(rt.block_on(CdpBackend::connect_with_url(&url))?)
                                }
                                // bing has a working pure-HTTP path — no Chrome →
                                // fall back to HTTP (google NEEDS a browser, and
                                // brave does too unless BRAVE_API_KEY is set — those
                                // launch errors still propagate).
                                Err(e)
                                    if opts.engine == "bing"
                                        || (opts.engine == "brave"
                                            && webrain_core::serp::brave_api_ready()) =>
                                {
                                    if !json_out {
                                        eprintln!(
                                            "no Chrome for bing browser path ({e}); falling back to pure HTTP"
                                        );
                                    }
                                    None
                                }
                                Err(e) => return Err(e),
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
            } else if opts.engine == "auto" {
                // auto: HTTP engines (ddg+bing) need no browser, but when one is
                // already attached (CDP_URL / 9222) google+brave join the merge.
                // Fast probe — connection refused is instant, never launches.
                if !json_out {
                    println!(
                        "engine auto: HTTP (ddg+bing, then baidu/yandex/yahoo top-up) — browser join if one is attached"
                    );
                }
                rt.block_on(CdpBackend::connect_default()).ok()
            } else {
                // HTTP-only engine (duckduckgo): pure ureq HTTP — NEVER
                // launch/connect a browser (auto probes for an attached one above).
                if !json_out {
                    println!("engine {}: pure HTTP — no browser launched", opts.engine);
                }
                None
            };
            match rt.block_on(webrain_core::serp::serp_search(&opts, backend.as_ref())) {
                Ok(r) => {
                    if json_out {
                        println!("{}", serde_json::to_string_pretty(&r)?);
                    } else {
                        let mut head = format!(
                            "query: {} | engine: {} | {} results ({}ms)",
                            r.query,
                            r.engine,
                            r.results.len(),
                            r.ms
                        );
                        if !r.skipped.is_empty() {
                            head.push_str(&format!(" | skipped: {}", r.skipped.join(", ")));
                        }
                        if !r.source.is_empty() && r.source != r.engine {
                            head.push_str(&format!(" | source: {} (fallback)", r.source));
                        }
                        println!("{head}");
                        if !r.per_engine.is_empty() {
                            let pe = r
                                .per_engine
                                .iter()
                                .map(|p| format!("{}={}({})", p.engine, p.status, p.count))
                                .collect::<Vec<_>>()
                                .join(", ");
                            println!("engines: {pe}");
                        }
                        for x in &r.results {
                            // plain ASCII separator — Windows consoles mangle the em-dash
                            println!("{}. {} - {}", x.position, x.title, x.url);
                            if !x.snippet.is_empty() {
                                println!("   {}", x.snippet);
                            }
                        }
                    }
                }
                Err(e) => {
                    // stderr + non-zero exit: scripts must be able to detect a
                    // failed search, and --json stdout must stay pure JSON.
                    // return Err (not exit(1)) so _launched drops cleanly and
                    // a --fresh Chrome isn't orphaned with an open debug port.
                    eprintln!("serp failed: {e}");
                    return Err(anyhow::anyhow!("serp failed: {e}"));
                }
            }
            // --hold: keep the fresh browser up so you can inspect it; close on
            // Enter (dropping `_launched` kills Chrome).
            if hold && _launched.is_some() {
                println!(
                    "[--hold] browser open — inspect the Chrome window, then press Enter to close it"
                );
                let mut line = String::new();
                let _ = std::io::stdin().read_line(&mut line);
            }
        }
        Some("install") => {
            // webrain install [--force] [--engine chrome|obscura] [--stealth] [--no-render]
            // agent-browser-style: download the engine into a cache dir.
            // Obscura v0.2.0 ships 4 packages/OS: render | -stealth | -no-render |
            // -no-render-stealth. --stealth = stealth build, --no-render = drop the
            // native render engine (screenshots/PDF unavailable). Default: render.
            let force = args.contains(&"--force".to_string());
            let stealth = args.contains(&"--stealth".to_string());
            let render = !args.contains(&"--no-render".to_string());
            let engine = args
                .iter()
                .position(|a| a == "--engine")
                .and_then(|i| args.get(i + 1))
                .cloned()
                // `webrain install whisper` → positional; bare `webrain install` → chrome
                .unwrap_or_else(|| args.get(2).cloned().unwrap_or_else(|| "chrome".to_string()));
            match engine.as_str() {
                "obscura" => {
                    let bin = webrain_core::install::install_obscura(force, stealth, render)?;
                    println!("engine ready: obscura at {}", bin.display());
                    println!(
                        "  package: {}{}",
                        if render { "render" } else { "no-render" },
                        if stealth { "+stealth" } else { "" }
                    );
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
                "whisper" => {
                    // webrain install whisper [--model small.en] [--force]
                    // Downloads the GGUF model for the LOCAL webrain watch
                    // backend (the whisper-cli binary can come from
                    // `webrain install watch` or PATH/WEBRAIN_WHISPER_BIN).
                    let model = args
                        .iter()
                        .position(|a| a == "--model")
                        .and_then(|i| args.get(i + 1))
                        .cloned()
                        .unwrap_or_else(|| "small.en".to_string());
                    let dest = webrain_core::install::install_whisper_model(&model, force)?;
                    println!(
                        "whisper model ready: {} — whisper-cli via PATH, WEBRAIN_WHISPER_BIN, or `webrain install watch`.",
                        dest.display()
                    );
                }
                "watch" => {
                    // webrain install watch [--model small.en] [--force]
                    // Mono packages: bundles ffmpeg+ffprobe, yt-dlp, whisper-cli
                    // and a GGUF model into the webrain cache so `webrain watch`
                    // works offline, self-contained, on any OS (no PATH installs).
                    let model = args
                        .iter()
                        .position(|a| a == "--model")
                        .and_then(|i| args.get(i + 1))
                        .cloned()
                        .unwrap_or_else(|| "small.en".to_string());
                    let status = webrain_core::install::install_watch(force, &model)?;
                    println!("{}", serde_json::to_string_pretty(&status)?);
                }
                "vision" => {
                    // webrain install vision [--model ID] [--model-update]
                    // Local "hero" vision backend, the whisper analog: bundles
                    // llama-server + one catalog model (GGUF + its own mmproj)
                    // so `watch --vision` works with NO cloud key, offline.
                    // Only the catalog ids are accepted — anything else prints
                    // the choices instead of fetching an arbitrary repo.
                    let model = args
                        .iter()
                        .position(|a| a == "--model")
                        .and_then(|i| args.get(i + 1))
                        .cloned()
                        // `install vision qwen3-vl-2b-q6k` — positional id, the way
                        // `--engine` accepts one. Without this a bare id silently
                        // installed the DEFAULT model.
                        .or_else(|| args.get(3).filter(|a| !a.starts_with("--")).cloned())
                        .unwrap_or_else(|| webrain_core::install::DEFAULT_VISION_ID.to_string());
                    let update = args.contains(&"--model-update".to_string());
                    let status = webrain_core::install::install_vision(&model, update)?;
                    println!("{}", serde_json::to_string_pretty(&status)?);
                }
                other => {
                    anyhow::bail!(
                        "unknown engine `{other}` — try chrome, obscura, lightpanda, whisper, vision, or watch"
                    )
                }
            }
        }
        Some("uninstall") => {
            // webrain uninstall vision [--model ID | --all]
            let target = args.get(2).cloned().unwrap_or_default();
            if target != "vision" {
                anyhow::bail!(
                    "unknown uninstall target `{target}` — try: webrain uninstall vision [--model ID | --all]"
                );
            }
            let model = args
                .iter()
                .position(|a| a == "--model")
                .and_then(|i| args.get(i + 1))
                .cloned();
            let all = args.contains(&"--all".to_string());
            if model.is_none() && !all {
                anyhow::bail!(
                    "pick what to remove: --model <id> or --all\n{}",
                    webrain_core::install::vision_model_choices()
                );
            }
            let status =
                webrain_core::install::uninstall_vision(if all { None } else { model.as_deref() })?;
            println!("{}", serde_json::to_string_pretty(&status)?);
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
            // `webrain launch` with no args opens Chrome in GUEST MODE at url —
            // a clean, ephemeral session with no profile state (no bookmarks,
            // no sign-in, nothing persisted). Explicit service/profile = persistent-
            // profile CDP launch (the Channel A login flow used by `webrain login`).
            let service = args.get(2).cloned().unwrap_or_default();
            let profile = args.get(3).cloned().unwrap_or_default();
            let url = args
                .get(4)
                .map(|s| s.as_str())
                .unwrap_or("https://www.google.com");
            if service.is_empty() && profile.is_empty() {
                webrain_core::launch::launch_chrome_default(url)?;
                println!("opened: {url} in Chrome GUEST mode");
                return Ok(());
            }
            let headless = args.contains(&"--headless".to_string());
            let explicit_port = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse::<u16>().ok());
            // Just launch & keep: without an explicit --port, pick the next free
            // port from 9222 instead of failing when a warm/leftover Chrome is
            // holding 9222.
            let port = explicit_port.unwrap_or_else(|| pick_free_port(9222));
            let service = if service.is_empty() { "web" } else { &service };
            let profile = if profile.is_empty() {
                "default"
            } else {
                &profile
            };
            let launched = webrain_core::launch::launch_chrome(service, profile, port, !headless)?;
            println!("profile: {}", launched.profile_dir.display());
            println!("CDP_URL={}", launched.cdp_url);
            // Attach + navigate: applies STEALTH_JS + UA override, opens the site.
            let backend = rt.block_on(CdpBackend::connect_with_url(&launched.cdp_url))?;
            rt.block_on(backend.navigate(url))?;
            // Wait out a Cloudflare/anti-bot interstitial (native poll+reload
            // loop). 90s budget; `webrain login` then fills the form with vault creds.
            let cleared = rt.block_on(backend.wait_out_challenge(90));
            println!("opened: {url} challenge_cleared={cleared} — keeping alive; Ctrl-C to stop");
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
            // webrain cookies [--port N] [--out file] [--netscape] — export cookies
            // (incl. HttpOnly) for cross-browser session migration.
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
            let netscape = args.contains(&"--netscape".to_string());
            let backend = rt.block_on(CdpBackend::connect_with_url(&format!(
                "http://127.0.0.1:{port}"
            )))?;
            // Network.getCookies needs a page-level session (Chrome rejects it at the
            // browser endpoint). Open a blank tab to guarantee one exists.
            let _ = rt.block_on(backend.open_tab("about:blank"));
            let cookies = rt.block_on(backend.cookies())?;
            match out {
                Some(path) => {
                    if netscape {
                        // Netscape HTTP Cookie File — the format yt-dlp/curl --cookie
                        // want, not the MCP JSON shape. (yt-dlp --cookies rejects JSON.)
                        let mut lines = vec!["# Netscape HTTP Cookie File".to_string()];
                        for c in &cookies {
                            let dom = c.get("domain").and_then(|v| v.as_str()).unwrap_or("");
                            if dom.is_empty() {
                                continue;
                            }
                            let inc = if dom.starts_with('.') {
                                "TRUE"
                            } else {
                                "FALSE"
                            };
                            let path_ = c.get("path").and_then(|v| v.as_str()).unwrap_or("/");
                            let sec = if c.get("secure").and_then(|v| v.as_bool()).unwrap_or(false)
                            {
                                "TRUE"
                            } else {
                                "FALSE"
                            };
                            let exp = c
                                .get("expires")
                                .and_then(|v| v.as_f64())
                                .map(|e| e as i64)
                                .unwrap_or(0);
                            let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
                            let val = c.get("value").and_then(|v| v.as_str()).unwrap_or("");
                            lines.push(format!(
                                "{dom}\t{inc}\t{path_}\t{sec}\t{exp}\t{name}\t{val}"
                            ));
                        }
                        std::fs::write(&path, lines.join("\n"))?;
                    } else {
                        std::fs::write(&path, serde_json::to_string_pretty(&cookies)?)?;
                    }
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
            // Accept both the bare array AND the `webrain cookies` stdout shape
            // ({"count": N, "cookies": [...]}) so piping cookies → setcookies
            // round-trips instead of silently setting 0 cookies.
            let cookies: Vec<serde_json::Value> = data
                .as_array()
                .cloned()
                .or_else(|| data.get("cookies").and_then(|c| c.as_array()).cloned())
                .unwrap_or_default();
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
                    // ponytail: don't echo the username back — it's half a credential.
                    println!("vault: {service}/{profile} username set");
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
            // Was a hand-written usage line that had drifted (no serp/watch/help)
            // and left out the CDP hint only some commands need. Exit 2: a shell
            // can tell a typo from a successful run.
            println!("unknown command: {cmd}\n");
            print!("{}", help_page(""));
            println!("\nSet CDP_URL to point to your browser:");
            println!("  Chrome:  --remote-debugging-port=9222");
            println!("  Obscura: obscura serve --stealth");
            std::process::exit(2);
        }
    }

    Ok(())
}

// ── webrain upgrade ──────────────────────────────────────────────
// Delegates to the package manager when installed through one (Homebrew /
// Scoop), otherwise self-updates the running binary in place.
// ── webrain upgrade ──────────────────────────────────────────────
// ONE command for every install method. Homebrew / Scoop delegate to the
// package manager (spawned detached, then this process exits so it never
// holds the binary); a raw install self-updates in place. Careful when
// webrain is genuinely running on any OS:
//   • Windows: a running exe is locked — only OTHER instances of THIS exact
//     binary are stopped (never an unrelated `webrain*` process or a second
//     install), and this process exits right after spawning the update.
//   • Unix: the swap is atomic (rename) and a running server keeps the old
//     inode, so nothing is killed — we only warn it's stale until restarted.
fn upgrade() -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    if cmd_exists("brew") {
        println!("webrain: installed via Homebrew \u{2014} running `brew upgrade webrain`");
        std::process::Command::new("brew")
            .args(["upgrade", "webrain"])
            .spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    if cmd_exists("scoop") && scoop_installed() {
        println!("webrain: installed via Scoop \u{2014} running `scoop update webrain`");
        // Windows locks a running exe, so Scoop refuses to replace webrain
        // while ANY instance of this binary is up. Stop only same-path
        // siblings and WAIT for them to exit (the lock must be gone before
        // scoop touches the file); child engines (Chrome/lightpanda/obscura)
        // are never touched — they stay warm for the new binary to re-attach.
        let killed = stop_same_exe_webrain();
        if !killed.is_empty() {
            println!(
                "webrain: stopped {} running instance(s) (PID {}) \u{2014} they held the old exe",
                killed.len(),
                killed
                    .iter()
                    .take(3)
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        // A cached scoop bucket silently reports the old version as "latest":
        // pull the hosting bucket so `scoop update webrain` sees the newest.
        refresh_scoop_bucket();
        std::process::Command::new("cmd")
            .args(["/c", "scoop", "update", "webrain"])
            .spawn()?;
        return Ok(());
    }
    self_update()?;
    warn_stale_running();
    Ok(())
}

#[cfg(target_os = "windows")]
fn scoop_installed() -> bool {
    std::path::Path::new(&std::env::var("USERPROFILE").unwrap_or_default())
        .join("scoop/apps/webrain")
        .exists()
}

/// Stop sibling webrain processes that point at THIS exact binary, wait for
/// them to fully exit (the exe lock must be released before scoop replaces
/// it), and return the PIDs stopped. Same-path-only means a different webrain
/// install or an unrelated process named `webrain*` is left alone; child
/// engines (Chrome/lightpanda/obscura) are never touched — they stay warm for
/// the new binary to re-attach. This process exits right after (it's never a
/// sibling to kill).
#[cfg(target_os = "windows")]
fn stop_same_exe_webrain() -> Vec<u32> {
    let self_pid = std::process::id();
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    // Escape single quotes in the exe path so a path containing `'` can't
    // terminate the PowerShell string and inject arbitrary commands.
    let exe_esc = exe.replace('\'', "''");
    // Kill then Wait-Process — Stop-Process can return before full teardown,
    // and the single synchronous PowerShell call means .output() returns only
    // after the wait completes.
    let ps = format!(
        "$p = Get-Process 'webrain*' -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -ne {self_pid} -and $_.Path -eq '{exe_esc}' }}; $p | ForEach-Object {{ $_.Id }}; $p | Stop-Process -Force -ErrorAction SilentlyContinue; $p | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue"
    );
    let mut killed = Vec::new();
    if let Ok(o) = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .output()
    {
        for line in String::from_utf8_lossy(&o.stdout).lines() {
            if let Ok(pid) = line.trim().parse::<u32>() {
                killed.push(pid);
            }
        }
    }
    killed
}

/// Find the scoop bucket that hosts webrain (`USERPROFILE/scoop/buckets/*/
/// bucket/webrain.json`) and `git pull` it, so `scoop update webrain` sees the
/// newest manifest instead of a cached one that reports the old version as
/// "latest". Best-effort — any failure is ignored (scoop update still runs).
#[cfg(target_os = "windows")]
fn refresh_scoop_bucket() {
    let buckets = std::env::var("USERPROFILE")
        .map(|u| std::path::PathBuf::from(u).join("scoop/buckets"))
        .unwrap_or_default();
    let Ok(rd) = std::fs::read_dir(&buckets) else {
        return;
    };
    for entry in rd.flatten() {
        if entry.path().join("bucket/webrain.json").exists() {
            let _ = std::process::Command::new("git")
                .arg("-C")
                .arg(entry.path())
                .arg("pull")
                .output();
            return;
        }
    }
}

/// Best-effort list of OTHER running webrain processes (self excluded).
/// On Unix this is a warning-only signal: an in-place rename doesn't disturb
/// a running server, it just leaves it on the old inode until a restart.
fn other_webrain_pids() -> Vec<u32> {
    let self_pid = std::process::id();
    let mut out = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let ps = format!(
            "Get-Process 'webrain*' -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -ne {self_pid} }} | Select-Object -ExpandProperty Id"
        );
        if let Ok(o) = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
            .output()
        {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    out.push(pid);
                }
            }
        }
    }
    #[cfg(unix)]
    {
        // pgrep -x matches the exact process name; fails silently if absent.
        if let Ok(o) = std::process::Command::new("pgrep")
            .args(["-x", "webrain"])
            .output()
        {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                if let Ok(pid) = line.trim().parse::<u32>() {
                    if pid != self_pid {
                        out.push(pid);
                    }
                }
            }
        }
    }
    out
}

fn warn_stale_running() {
    let pids = other_webrain_pids();
    if !pids.is_empty() {
        let head = pids
            .iter()
            .take(3)
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "webrain: {} other instance(s) running (PID {head}\u{2026}) \u{2014} they keep the old version until restarted.",
            pids.len()
        );
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn cmd_exists(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p)
            .any(|d| d.join(name).exists() || d.join(format!("{name}.exe")).exists())
    })
}

/// Fetch the release `checksums.txt` and return the expected SHA-256 (lower hex)
/// for `asset`, or `None` if the file/entry is missing on this release.
fn fetch_expected_sha256(sums_url: &str, asset: &str) -> anyhow::Result<Option<String>> {
    let out = std::process::Command::new("curl")
        .arg("-fsSL")
        .arg(sums_url)
        .output()?;
    if !out.status.success() {
        return Ok(None); // no checksums.txt published for this release
    }
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        // sha256sum format: "<hex>  <name>"
        let mut it = line.split_whitespace();
        let (Some(sum), Some(name)) = (it.next(), it.next()) else {
            continue;
        };
        if name == asset {
            return Ok(Some(sum.to_ascii_lowercase()));
        }
    }
    Ok(None)
}

/// SHA-256 (lower hex) of a file.
fn sha256_hex(path: &std::path::Path) -> anyhow::Result<String> {
    use sha2::Digest;
    use std::io::Read;
    let mut h = sha2::Sha256::new();
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
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
    let sums_url =
        "https://github.com/prokopis3/webrain/releases/latest/download/checksums.txt".to_string();
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
    // Supply-chain guard: verify the SHA-256 of the downloaded binary against
    // the release's checksums.txt BEFORE replacing the running executable.
    // Fail closed — a missing/mismatched checksum aborts the self-update.
    let expected = fetch_expected_sha256(&sums_url, asset)?;
    let actual = sha256_hex(&tmp)?;
    match expected {
        Some(e) if e.eq_ignore_ascii_case(&actual) => {}
        _ => {
            let _ = std::fs::remove_file(&tmp);
            anyhow::bail!(
                "checksum verification failed for {asset}: got {actual}, expected {expected:?} — aborting self-update"
            );
        }
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

/// First free TCP port from `start` — used by `--fresh` to never collide with
/// a warm 9222 chrome.
fn pick_free_port(start: u16) -> u16 {
    for p in start..start + 40 {
        if std::net::TcpStream::connect(("127.0.0.1", p)).is_err() {
            return p;
        }
    }
    start
}

// ponytail: TCP connect probe — try opening, return true if port is listening.
async fn check_tcp(host: &str, port: u16) -> bool {
    tokio::net::TcpStream::connect((host, port)).await.is_ok()
}

/// CDP `/json/version` facts: browser name, the `webSocketDebuggerUrl` port,
/// and whether the UA reports `HeadlessChrome`. Zero deps (raw TCP + one GET).
/// ponytail: a relay (wslrelay/docker) answers /json/version but its
/// webSocketDebuggerUrl points at a DIFFERENT port — that mismatch is the relay
/// tell; headless lives in the UA string. Both catch the "9224 looks like a
/// local Chrome" trap that silently cost a login session.
struct CdpInfo {
    name: String,
    ws_port: Option<u16>,
    headless: bool,
}

async fn cdp_info(port: u16) -> CdpInfo {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut info = CdpInfo {
        name: "unknown".into(),
        ws_port: None,
        headless: false,
    };
    if let Ok(mut s) = tokio::net::TcpStream::connect(("127.0.0.1", port)).await {
        let req = format!("GET /json/version HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
        if s.write_all(req.as_bytes()).await.is_ok() {
            let mut buf = vec![0u8; 4096];
            let n = s.read(&mut buf).await.unwrap_or(0);
            let body = String::from_utf8_lossy(&buf[..n]);
            // find \r\n\r\n separator, take JSON after it. Kept as nested
            // if-let, not a let-chain: let-chains need Rust 1.88 but the
            // workspace MSRV is 1.85.
            #[allow(clippy::collapsible_if)]
            if let Some(pos) = body.find("\r\n\r\n") {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&body[pos + 4..]) {
                    info.name = v
                        .get("Browser")
                        .and_then(|b| b.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    if let Some(ws) = v.get("webSocketDebuggerUrl").and_then(|w| w.as_str()) {
                        // ws://host:PORT/devtools/browser
                        info.ws_port = ws
                            .split(':')
                            .nth(2)
                            .and_then(|s| s.split('/').next())
                            .and_then(|s| s.parse().ok());
                    }
                    let ua = v.get("User-Agent").and_then(|u| u.as_str()).unwrap_or("");
                    info.headless = ua.contains("HeadlessChrome");
                }
            }
        }
    }
    info
}

/// `webrain doctor` — diagnose the install: version, MCP server, CDP ports,
/// engines (agent-browser-style discovery), encrypted vault.
/// Exit 0 if a browser is reachable, 2 otherwise.
fn run_doctor(rt: &tokio::runtime::Runtime) -> i32 {
    let cdp_ports = [9222u16, 9224u16, 9225u16];
    let mut any_cdp = false;
    println!("webrain doctor");
    println!("  version          {}", env!("CARGO_PKG_VERSION"));
    println!(
        "  mcp server       {}",
        if rt.block_on(async { check_tcp("127.0.0.1", 9223).await }) {
            "✅ (http://127.0.0.1:9223)".to_string()
        } else {
            "❌ not running — start it: `webrain mcp --http 9223`".into()
        }
    );
    for &port in &cdp_ports {
        let up = rt.block_on(async { check_tcp("127.0.0.1", port).await });
        if up {
            any_cdp = true;
        }
        if !up {
            println!("  cdp port {port}      ❌ (no browser)");
            continue;
        }
        let info = rt.block_on(async { cdp_info(port).await });
        let mut label = format!("✅ ({})", info.name);
        if let Some(ws) = info.ws_port
            && ws != port
        {
            label.push_str(&format!(" ⚠️ relay/tunnel (ws→:{ws})"));
        }
        if info.headless {
            label.push_str(" ⚠️ headless — cannot pass login challenges");
        }
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
            "⚠️ empty — `webrain vault set`"
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
