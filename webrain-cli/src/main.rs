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
    // vault. Exit 0 healthy / 2 broken.
    if args.contains(&"--doctor".to_string()) || args.get(1).map(|s| s.as_str()) == Some("doctor") {
        std::process::exit(run_doctor(&rt));
    }

    match args.get(1).map(|s| s.as_str()) {
        Some("mcp") => {
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
            // webrain serp "query" [--engine duckduckgo|bing|google|brave|auto]
            //   [--limit N] [--page N] [--safe] [--region R] [--no-fallback] [--json]
            // Structured SERP JSON. duckduckgo/bing/google/auto need no browser;
            // brave renders in the connected CDP engine (CDP_URL /
            // --remote-debugging-port=9222).
            let query = args.get(2).cloned().unwrap_or_default();
            if query.trim().is_empty() {
                println!(
                    "usage: webrain serp \"query\" [--engine duckduckgo|bing|google|brave|auto] [--limit N] [--page N] [--safe] [--region R] [--no-fallback] [--json] [--headless] [--proxy URL] [--fresh] [--pipe] [--stealth] [--hold]"
                );
                println!(
                    "  duckduckgo|bing|auto need no browser; brave uses the connected CDP engine; google auto-launches a persistent-profile Chrome when none is attached — DEFAULT = warm persistent profile + session (the browser stays alive on 9222 between runs and warms into a trusted google profile, the skill's real bypass path); CDP_URL / --remote-debugging-port=9222 to override, --headless for a headless one, --proxy http://user:pass@host:port to route HTTP engines + the google auto-launch through a proxy, --fresh to opt OUT of the warm session: brand-new profile + cookies every run so the consent modal always appears and is dismissed, --pipe (with --fresh) to launch that Chrome via --remote-debugging-pipe so there's NO open debugging port (the automation fingerprint that walls google), --hold to keep the launched Chrome open after the search so you can watch it — press Enter to close)"
                );
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
                .clamp(1, 50);
            let page: usize = flag("--page").and_then(|v| v.parse().ok()).unwrap_or(0);
            let safe = args.contains(&"--safe".to_string());
            let region = flag("--region");
            let fallback = !args.contains(&"--no-fallback".to_string());
            let json_out = args.contains(&"--json".to_string());
            let headless = args.contains(&"--headless".to_string());
            let proxy = flag("--proxy");
            // --fresh: always start a brand-new google profile + cookies (never
            // attach a warm browser) so the consent modal ALWAYS appears and is
            // always dismissed — the deterministic anti-bot recipe.
            let fresh = args.contains(&"--fresh".to_string());
            // --pipe: launch the fresh google Chrome via `--remote-debugging-pipe`
            // (stdin/stdout, NO open debugging port) — an open debugging port is
            // the automation fingerprint Google walls on /sorry; a pipe-launched
            // Chrome has none. Only meaningful with --fresh.
            let pipe = args.contains(&"--pipe".to_string());
            // --stealth: launch with the AutomationControlled suppression flags
            // (the detectable launch-flag stealth). Default is a PLAIN launch +
            // CDP-level masking (stealth_js) — the patchright/browsemind
            // recommended combo for google. Opt-in for parity with `webrain launch`.
            let stealth = args.contains(&"--stealth".to_string());
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
            };
            // google is JS-gated (consent/JS shell over plain HTTP) — same as
            // brave, it needs a real browser. For google, if none is attached,
            // auto-launch a stealth Chrome with a PERSISTENT profile
            // (profiles/serp/google) so the engine always exists; that profile
            // accumulates Google consent/session cookies over runs and becomes
            // trusted (the warm-up that made serp_test work), headed or
            // headless alike. The browser stays alive as a warm session.
            // --hold keeps the FRESH browser alive past the search so you can
            // watch it; only fresh holds — the warm 9222 profile keeps its
            // warm-session persistence.
            let hold = args.contains(&"--hold".to_string());
            let mut _launched: Option<webrain_core::launch::Launched> = None;
            let backend = if opts.engine == "brave" || opts.engine == "google" {
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
                            anyhow::bail!("--fresh --pipe does not support --proxy (pipe Chrome can't bake --proxy-server here)");
                        }
                        let child = rt.block_on(webrain_core::launch::launch_chrome_pipe(
                            "serp", &prof_name, !headless,
                        ))?;
                        println!(
                            "launched FRESH chrome via --remote-debugging-pipe (no debugging port -> no open-debugger fingerprint): profile serp/{}",
                            prof_name
                        );
                        Some(rt.block_on(CdpBackend::connect_pipe(child))?)
                    } else {
                        let port = pick_free_port(9230);
                        let launched = if stealth {
                            match proxy.as_deref() {
                                Some(p) => webrain_core::launch::launch_chrome_with_proxy(
                                    "serp", &prof_name, port, !headless, p,
                                )?,
                                None => webrain_core::launch::launch_chrome(
                                    "serp", &prof_name, port, !headless,
                                )?,
                            }
                        } else {
                            match proxy.as_deref() {
                                Some(p) => webrain_core::launch::launch_chrome_plain_with_proxy(
                                    "serp", &prof_name, port, !headless, p,
                                )?,
                                None => webrain_core::launch::launch_chrome_plain(
                                    "serp", &prof_name, port, !headless,
                                )?,
                            }
                        };
                        println!(
                            "launched FRESH chrome (no cookies -> consent modal handled): {} (CDP_URL={})",
                            launched.profile_dir.display(),
                            launched.cdp_url
                        );
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
                        Err(_) if opts.engine == "google" => {
                            // --proxy bakes --proxy-server into the auto-launched Chrome so
                            // the google browser path egresses through the proxy (IP rotation
                            // on a walled IP). No proxy -> plain persistent-profile launch.
                            let launched = if stealth {
                                match proxy.as_deref() {
                                    Some(p) => webrain_core::launch::launch_chrome_with_proxy(
                                        "serp", "google", 9222, !headless, p,
                                    )?,
                                    None => webrain_core::launch::launch_chrome(
                                        "serp", "google", 9222, !headless,
                                    )?,
                                }
                            } else {
                                match proxy.as_deref() {
                                    Some(p) => webrain_core::launch::launch_chrome_plain_with_proxy(
                                        "serp", "google", 9222, !headless, p,
                                    )?,
                                    None => webrain_core::launch::launch_chrome_plain(
                                        "serp", "google", 9222, !headless,
                                    )?,
                                }
                            };
                            println!(
                                "launched chrome: {} (CDP_URL={})",
                                launched.profile_dir.display(),
                                launched.cdp_url
                            );
                            // WARM PERSISTENT SESSION — the skill's real
                            // google-bypass path (references/challenges.md).
                            // Forget the handle so Drop::kill() never runs: the
                            // browser STAYS alive on 9222 between runs and warms
                            // up (consent/session cookies accumulate until it's
                            // "trusted"). `--fresh` is the explicit opt-out for
                            // deterministic consent every run.
                            let url = launched.cdp_url.clone();
                            std::mem::forget(launched);
                            Some(rt.block_on(CdpBackend::connect_with_url(&url))?)
                        }
                        Err(e) => return Err(e),
                    }
                }
            } else {
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
                        println!("{head}");
                        for x in &r.results {
                            // plain ASCII separator — Windows consoles mangle the em-dash
                            println!("{}. {} - {}", x.position, x.title, x.url);
                            if !x.snippet.is_empty() {
                                println!("   {}", x.snippet);
                            }
                        }
                    }
                }
                Err(e) => println!("error: {e}"),
            }
            // --hold: keep the fresh browser up so you can inspect it; close on
            // Enter (dropping `_launched` kills Chrome).
            if hold && _launched.is_some() {
                println!("[--hold] browser open — inspect the Chrome window, then press Enter to close it");
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
                    // webrain install vision [--force]
                    // Local "hero" vision backend, the whisper analog: bundles
                    // llama-server + Qwen3-VL-2B (GGUF + mmproj) into the cache
                    // so `watch --vision` works with NO cloud key, offline.
                    let status = webrain_core::install::install_vision(force)?;
                    println!("{}", serde_json::to_string_pretty(&status)?);
                }
                other => {
                    anyhow::bail!(
                        "unknown engine `{other}` — try chrome, obscura, lightpanda, whisper, vision, or watch"
                    )
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
        Some("launch") | None => {
            // webrain launch <service> <profile> [url] [--headless] [--port N]
            // Spawns a Chrome with a persistent per-account profile and opens the
            // site so the human can log in (Channel A). Bare `webrain` (double-click
            // the exe) or `webrain launch` with no args both do the DEFAULT launch:
            // service=web, profile=default, url=google.com.
            let service = args.get(2).cloned().unwrap_or_default();
            let profile = args.get(3).cloned().unwrap_or_default();
            let url = args
                .get(4)
                .map(|s| s.as_str())
                .unwrap_or("https://www.google.com");
            let headless = args.contains(&"--headless".to_string());
            let port: u16 = args
                .iter()
                .position(|a| a == "--port")
                .and_then(|i| args.get(i + 1))
                .and_then(|s| s.parse().ok())
                .unwrap_or(9222);
            let service = if service.is_empty() { "web" } else { &service };
            let profile = if profile.is_empty() { "default" } else { &profile };
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
        // ponytail: spawn detached + exit, never block-wait. If this process stays
        // alive while the package manager runs, Scoop sees the upgrade command
        // itself as a running instance of webrain and refuses to replace the exe.
        // Detach so the binary is free when brew/scoop swap it.
        std::process::Command::new("brew")
            .args(["upgrade", "webrain"])
            .spawn()?;
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    if cmd_exists("scoop")
        && std::path::Path::new(&std::env::var("USERPROFILE").unwrap_or_default())
            .join("scoop/apps/webrain")
            .exists()
    {
        println!("webrain: installed via Scoop \u{2014} running `scoop update webrain`");
        // Windows locks a running exe, so Scoop refuses to replace webrain while
        // ANY instance is up — the MCP server keeps one alive and blocks every
        // upgrade. Close the other instances first (self exits right after
        // spawning scoop, so it's never locked), then update detached.
        let ps = format!(
            "Get-Process 'webrain*' -ErrorAction SilentlyContinue | Where-Object {{ $_.Id -ne {} }} | Stop-Process -Force",
            std::process::id()
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
            .output();
        // scoop is a .cmd/.ps1 shim, not scoop.exe — spawn it through cmd.exe
        std::process::Command::new("cmd")
            .args(["/c", "scoop", "update", "webrain"])
            .spawn()?;
        return Ok(());
    }
    self_update()
}

fn cmd_exists(name: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| {
        std::env::split_paths(&p)
            .any(|d| d.join(name).exists() || d.join(format!("{name}.exe")).exists())
    })
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
            // find \r\n\r\n separator, take JSON after it
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
        if let Some(ws) = info.ws_port {
            if ws != port {
                label.push_str(&format!(" ⚠️ relay/tunnel (ws→:{ws})"));
            }
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
            "⚠️ empty — `webrain vault set`".into()
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
