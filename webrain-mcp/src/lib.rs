// webrain-mcp: Model Context Protocol server for AI model browser automation.
//
// 17 tools: webrain_navigate, webrain_eval (JS→JSON), webrain_screenshot,
// webrain_click/type/scroll (index-based), webrain_get_html, webrain_spider
// (BFS/DFS), webrain_snapshot (D1 skip), webrain_pixel (PixelRAG tiles),
// webrain_extract_json (CSS/XPath schema), webrain_extract_regex, webrain_pdf,
// webrain_tab (multi-tab), webrain_a11y (accessibility tree),
// webrain_semantic_tree (text snapshot), webrain_batch
// (fetch/extract/screenshot over URLs), webrain_download (http+ytdlp),
// webrain_search (4 engines + plus), webrain_nav, webrain_press, webrain_get_images,
// webrain_media (CDP network capture), webrain_console, webrain_dismiss_overlays,
// webrain_vision_index / webrain_vision_retrieve (PixelRAG cosine).
// webrain_navigate returns {url,title,text,elements}; elements are index-only.
// Precise selectors come from webrain_a11y. No HTML→Markdown converter: text via
// navigate, layout via webrain_pixel.
//
// ponytail: one shared handle_rpc for stdio AND HTTP; HTTP gives per-connection
// session isolation (lightpanda Mcp-Session-Id pattern, one backend per conn).

mod tools;

use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::sync::Arc;
use webrain_core::backends::cdp::CdpBackend;
use webrain_core::browser::BrowserBackend;

/// Compact error envelope for no-browser tool short-circuits.
/// Current-origin localStorage dump (agent-browser --state borrow).
const SAVE_LS_JS: &str = r#"(() => { const o = {}; for (let i=0;i<localStorage.length;i++){ const k=localStorage.key(i); o[k]=localStorage.getItem(k); } return o; })()"#;

/// Build JS to restore a saved localStorage object on the current origin.
fn restore_ls_js(ls: &Value) -> String {
    let obj = serde_json::to_string(ls).unwrap_or_else(|_| "{}".into());
    format!(
        r#"(() => {{ let n = 0; for (const [k,v] of Object.entries({obj})) {{ try {{ localStorage.setItem(k,v); n++; }} catch(e) {{}} }} return n; }})()"#
    )
}

fn tool_error(id: Option<Value>, msg: &str) -> Value {
    let text = serde_json::to_string(&json!({ "status": "error", "message": msg }))
        .unwrap_or_else(|_| "{}".into());
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {"content": [{"type": "text", "text": text}], "isError": true}
    })
}

/// Shared-instance mode. A container serving many anonymous users must not offer
/// tools that write PROCESS-GLOBAL state: the credential vault and browser
/// profiles are per-process, not per-session, so one caller's login would hand
/// the next caller a working profile. See docs/PLATFORM_EXPOSURE_PLAN.md §2b.
///
/// `WEBRAIN_NO_STATE=1|on|true|yes` refuses them. Off by default, so local and
/// stdio users keep full behaviour.
pub fn no_state() -> bool {
    match std::env::var("WEBRAIN_NO_STATE") {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "1" | "on" | "true" | "yes"
        ),
        Err(_) => false,
    }
}

/// Names that read or write state a shared instance must not expose: the vault,
/// the browser profiles, and everything session-scoped.
///
/// `webrain_session` is listed as the UMBRELLA, not merely as its folded
/// executors. It folds into eight of them — open, close, list, cookies,
/// setcookies, save_state, restore_state, close_launch — and an earlier version
/// of this list named only `profiles` and `login`, so the other six were served
/// on a shared worker: one tenant could enumerate another tenant's sessions and
/// read or write its cookies and auth state. Verified against a running worker by
/// the deepscrape integration, which then had to rebuild the boundary in front of
/// webrain (`functions/src/domain/webrain-tools.domain.ts`) because this guard
/// did not hold.
///
/// Matching the umbrella rather than enumerating the eight is deliberate: it
/// fails closed. A ninth `webrain_session` op is guarded the day it ships, whereas
/// a longer literal list would silently not be.
const STATEFUL_TOOLS: &[&str] = &[
    "webrain_session",
    "webrain_profiles",
    "webrain_launch",
    "webrain_login",
];

fn is_stateful(tool: &str) -> bool {
    STATEFUL_TOOLS.contains(&tool)
}

/// Length-independent byte comparison. A plain `==` on a secret short-circuits at
/// the first differing byte, which leaks the shared prefix through response
/// timing; this touches every byte regardless.
fn token_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Pulls the credential out of an `Authorization` header value. Only the scheme
/// is case-folded (RFC 7235); the token itself is opaque and must stay
/// byte-exact, so the value is never lowercased on the way in.
fn bearer_token(authorization: &str) -> Option<&str> {
    authorization
        .split_once(' ')
        .and_then(|(scheme, tok)| scheme.eq_ignore_ascii_case("bearer").then_some(tok.trim()))
}

/// Optional shared-secret gate for the HTTP transport. `expected` is the RAW
/// `WEBRAIN_HTTP_TOKEN` value — unset, empty or whitespace-only means "no token
/// configured", which has to stay a no-op or every loopback/stdio user needs
/// configuration they never needed.
///
/// Takes the expected token as a parameter instead of reading the env itself:
/// `std::env::set_var` races across threads and the test harness is one process,
/// so an env-reading gate could only be tested by serialising the whole binary.
/// The call site does the env read and passes the result here.
///
/// ponytail: a shared secret is the whole of the auth here — no rotation, no
/// per-caller identity, no replay window. That is adequate for a worker behind
/// a caller that owns the tier allowlist; the upgrade path is real tenancy
/// (mTLS / signed requests) if this port is ever exposed without that caller.
fn bearer_allows(expected: Option<&str>, authorization: Option<&str>) -> bool {
    let Some(expected) = expected.map(str::trim).filter(|t| !t.is_empty()) else {
        return true;
    };
    authorization
        .and_then(bearer_token)
        .is_some_and(|presented| token_eq(presented.as_bytes(), expected.as_bytes()))
}

/// Most tabs a single caller may fan out to on a shared instance. Without a cap
/// one tenant can request a thousand tabs and starve every other tenant on the
/// box. Only enforced under `WEBRAIN_NO_STATE` — a local user owns the machine.
const MAX_SHARED_CONCURRENCY: usize = 6;

/// Tab budget for THIS box: two per logical core (the second absorbs network
/// wait, not render work). Measured on 16 cores — past ~2× cores throughput
/// stops improving while peak RSS keeps climbing (24 tabs / 100 pages → 4.4 GB).
/// `WEBRAIN_MAX_TABS` overrides for boxes that must stay small or deliberately
/// over-subscribe.
pub(crate) fn tab_budget() -> usize {
    if let Ok(v) = std::env::var("WEBRAIN_MAX_TABS") {
        if let Ok(n) = v.trim().parse::<usize>() {
            if n > 0 {
                return n;
            }
        }
    }
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .saturating_mul(2)
        .max(1)
}

pub fn clamp_concurrency(n: usize) -> usize {
    let n = n.max(1).min(tab_budget());
    if no_state() {
        n.min(MAX_SHARED_CONCURRENCY)
    } else {
        n
    }
}

/// Shared JSON-RPC dispatch: stdio and HTTP both route through here.
async fn handle_rpc(msg: Value, backend: &mut Option<CdpBackend>, cdp_url: Option<&str>) -> Value {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");

    match method {
        "initialize" => json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "webrain-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
        "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
        "tools/list" => {
            let tools = tools::list_tools();
            json!({"jsonrpc": "2.0", "id": id, "result": {"tools": tools}})
        }
        "tools/call" => {
            let params = msg.get("params").cloned().unwrap_or_default();
            // Kept separately: the guard below must see the umbrella the caller
            // spelled (`webrain_session`), not only what it folded into.
            let requested = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let mut arguments = params.get("arguments").cloned().unwrap_or(json!({}));
            // Fold the consolidated intent-based surface (webrain_observe{what},
            // webrain_interact{action}, webrain_session{op}, …) into the legacy
            // executor name up front, so the no-browser short-circuits below
            // (keyed on legacy names) and call_tool dispatch see the executor.
            let tool_name: String = tools::map_surface(requested, &arguments)
                .map(|(n, a)| {
                    arguments = a;
                    n.to_string()
                })
                .unwrap_or_else(|| requested.to_string());

            // Shared-instance guard. Checks the folded executor AND the umbrella the
            // caller actually used, because folding is lossy in the direction that
            // matters: `webrain_session(op=list)` becomes `webrain_list_sessions`,
            // a name no list of stateful executors would contain.
            if no_state() && (is_stateful(&tool_name) || is_stateful(requested)) {
                return tool_error(
                    id,
                    &format!("{tool_name} is disabled on this instance: WEBRAIN_NO_STATE is set"),
                );
            }

            // No-browser tools: serve without a CDP backend, so they work
            // on a fresh install before any browser is up (any-LLM portability).
            if tool_name == "webrain_guide" {
                // One implementation, both transports: the topic/skills logic lives
                // in tools::guide_from_args, so stdio and HTTP can never diverge.
                let out = tools::guide_from_args(&arguments);
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&out).unwrap_or_else(|_| "{}".into())}],
                        "isError": false
                    }
                });
            }
            // No-browser, no-page tool: the machine's own /proc. Registered beside
            // `webrain_guide` because both answer before a CDP backend exists — which
            // is exactly when "why is this box unresponsive" gets asked.
            if tool_name == "webrain_stats" {
                // On a pooled worker these numbers describe a box other tenants run in,
                // so the platform already refuses this tool there. The engine has to
                // agree: a direct MCP client holding the pool token would otherwise read
                // the host's /proc regardless of what TypeScript allows.
                if no_state() {
                    return tool_error(
                        id,
                        "webrain_stats is disabled on this instance: WEBRAIN_NO_STATE is set",
                    );
                }
                let out = tools::host_stats();
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&out).unwrap_or_else(|_| "{}".into())}],
                        "isError": false
                    }
                });
            }
            // ponytail: fetch_http uses ureq (no browser), so skip CDP connect too.
            if tool_name == "webrain_fetch_http" {
                // ureq is blocking — run off the executor so a slow fetch can't
                // stall other sessions. The work itself lives in
                // `tools::fetch_http_from_args` so this layer and the dispatcher
                // cannot drift (this short-circuit used to drop `base_selector`).
                let args = arguments.clone();
                let result = tokio::task::spawn_blocking(move || {
                    crate::tools::fetch_http_from_args(&args)
                })
                .await
                .unwrap_or_else(
                    |e| json!({"status": "error", "message": format!("fetch task failed: {e}")}),
                );
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}],
                        "isError": result.get("status").and_then(|v| v.as_str()) == Some("error")
                    }
                });
            }
            // ponytail: download_files uses ureq (no browser), so skip CDP connect too.
            if tool_name == "webrain_download" {
                let urls: Vec<String> = arguments
                    .get("urls")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                if urls.is_empty() {
                    return json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "content": [{"type": "text", "text": serde_json::to_string(&json!({"status": "error", "message": "urls required"})).unwrap_or_else(|_| "{}".into())}],
                            "isError": true
                        }
                    });
                }
                let dir = arguments
                    .get("dir")
                    .and_then(|v| v.as_str())
                    .unwrap_or("downloads")
                    .to_string();
                // ponytail: honor engine — ytdlp shells out to the installed binary.
                // (This short-circuit used to ALWAYS force the HTTP path, silently
                // ignoring engine:"ytdlp", so the advertised ytdlp engine was dead.)
                // http keeps the no-browser streaming path.
                let result = if arguments.get("engine").and_then(|v| v.as_str()) == Some("ytdlp") {
                    let audio_only = arguments
                        .get("audio_only")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let format = arguments
                        .get("format")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let extra: Vec<String> = arguments
                        .get("args")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    webrain_core::engines::download_ytdlp(
                        &urls,
                        &dir,
                        audio_only,
                        format.as_deref(),
                        &extra,
                    )
                } else {
                    let results = webrain_core::engines::download_files(&urls, &dir);
                    json!({"status": "ok", "count": results.len(), "results": results})
                };
                let is_err = result.get("status").and_then(|v| v.as_str()) == Some("error");
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}],
                        "isError": is_err
                    }
                });
            }
            // ponytail: webrain_watch shells out to yt-dlp/ffmpeg/ffprobe (no
            // browser), so skip CDP connect too — same shared helper as the tool
            // dispatch (tools::watch_from_args), one implementation, two callers.
            // It can run for minutes, so keep it OFF the executor.
            if tool_name == "webrain_watch" {
                let args = arguments.clone();
                let result =
                    match tokio::task::spawn_blocking(move || crate::tools::watch_from_args(&args))
                        .await
                    {
                        Ok(v) => v,
                        Err(e) => {
                            json!({"status": "error", "message": format!("watch task failed: {e}")})
                        }
                    };
                let is_err = result.get("status").and_then(|v| v.as_str()) == Some("error");
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}],
                        "isError": is_err
                    }
                });
            }
            // ponytail: webrain_search uses ureq (no browser), so skip CDP connect too.
            if tool_name == "webrain_search" {
                let q = arguments.get("q").and_then(|v| v.as_str()).unwrap_or("");
                if q.is_empty() {
                    return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":r#"{"status":"error","message":"q required"}"#}],"isError":true}});
                }
                let engine = arguments
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("duckduckgo");
                // Percent-encode the query — space→+ lets &/#/% inject params or
                // truncate the query ("C++" became ambiguous "C+++").
                let encoded: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
                let url = match engine {
                    "bing" => format!("https://www.bing.com/search?q={encoded}"),
                    "brave" => format!("https://search.brave.com/search?q={encoded}"),
                    "google" => format!("https://www.google.com/search?q={encoded}"),
                    _ => format!("https://html.duckduckgo.com/html/?q={encoded}"),
                };
                let engine = engine.to_string();
                let result = tokio::task::spawn_blocking(move || {
                    match webrain_core::engines::http_fetch(&url) {
                        Ok(v) => {
                            json!({"status":"ok","engine":engine,"url":url,"text":v.get("text").cloned().unwrap_or_default()})
                        }
                        Err(e) => json!({"status":"error","message":e.to_string()}),
                    }
                })
                .await
                .unwrap_or_else(|e| {
                    json!({"status":"error","message":format!("search task failed: {e}")})
                });
                return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&result).unwrap_or_default()}],"isError":result.get("status").and_then(|v|v.as_str())==Some("error")}});
            }
            // ponytail: serp HTTP engines (duckduckgo/bing/auto) use ureq — no
            // browser, so skip CDP connect too. `google`/`brave` are JS-gated →
            // they need a connected CDP engine (Chrome/obscura/lightpanda) and
            // fall through to the backend connect + call_tool dispatch below.
            // `auto` is HTTP-only by default, but joins google/brave when a
            // backend is already attached (passed through below).
            let mut serp_no_stealth = false;
            if tool_name == "webrain_serp" {
                let engine = arguments
                    .get("engine")
                    .and_then(|v| v.as_str())
                    .unwrap_or("duckduckgo");
                // Only google|brave REQUIRE a browser (JS-gated → they parse to
                // zero without one). bing CAN drive one for deeper paging, but the
                // browserless path is the DEFAULT: measured 2026-10-10, `limit=50`
                // returned 11 results for duckduckgo and 10 for bing without a
                // browser, so a caller who wants depth opts IN with
                // `browser: true` rather than paying for a guest Chrome by default.
                // Absent/unparseable `browser` means false — the cheap path.
                let wants_browser = arguments
                    .get("browser")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                // brave stops being browser-only once BRAVE_API_KEY is set (its
                // keyed API needs no CDP engine).
                let needs_browser = engine == "google"
                    || (engine == "brave" && !webrain_core::serp::brave_api_ready())
                    || (engine == "bing" && wants_browser);
                if !needs_browser {
                    // `auto` also runs HTTP-only, but when a backend is already
                    // attached it joins google/brave into the merge (the core's
                    // browser-join path) — pass it through and mark it trusted
                    // (no stealth injection for the google render).
                    let backend_ref = if engine == "auto" {
                        backend.as_ref()
                    } else {
                        None
                    };
                    if let Some(b) = backend_ref {
                        b.set_no_stealth(true);
                    }
                    let result = crate::tools::serp_from_args(&arguments, backend_ref).await;
                    // Scoped no-stealth: clear the flag after the serp call so a
                    // later navigate/batch/scrape in this session isn't silently
                    // run without stealth_js (the google/auto serp arms set it).
                    if let Some(b) = backend_ref {
                        b.set_no_stealth(false);
                    }
                    let is_err = result.get("status").and_then(|v| v.as_str()) == Some("error");
                    return json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}],
                            "isError": is_err
                        }
                    });
                }
                // google|brave → guest-browser flow (mirrors the CLI). google
                // disables the stealth_js injection on ITS backend
                // (trusted-commands-only) — per-backend, not a process-global
                // set_var (that's an edition-2024 data race from a concurrent
                // handler and leaks to every session). Then ensure a backend:
                // attach to CDP_URL/9222 if one is up, else guest-launch Chrome
                // (warm session kept alive between calls).
                if backend.is_none() {
                    let res = if let Some(url) = cdp_url {
                        CdpBackend::connect_with_url(url).await
                    } else {
                        CdpBackend::connect_default().await
                    };
                    match res {
                        Ok(b) => *backend = Some(b),
                        Err(_) => {
                            let prof = if engine == "google" {
                                "google"
                            } else if engine == "bing" {
                                "bing"
                            } else {
                                "brave"
                            };
                            let proxy = arguments
                                .get("proxy")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                            match webrain_core::launch::launch_chrome_guest(
                                "serp",
                                prof,
                                9222,
                                true,
                                proxy.as_deref(),
                            ) {
                                Ok(l) => {
                                    let url = l.cdp_url.clone();
                                    match CdpBackend::connect_with_url(&url).await {
                                        Ok(b) => {
                                            // Attach succeeded — keep the guest alive
                                            // between calls (warm session). Only forget
                                            // AFTER attach, so a failed attach drops the
                                            // guard and the guest Chrome is killed
                                            // (no orphan process).
                                            std::mem::forget(l);
                                            *backend = Some(b);
                                        }
                                        Err(e) => {
                                            return tool_error(
                                                id,
                                                &format!(
                                                    "guest-launched browser at {url} but attach failed: {e}"
                                                ),
                                            );
                                        }
                                    }
                                }
                                // bing has a working pure-HTTP path — no Chrome →
                                // run serp_from_args(None) HERE: the shared dispatch
                                // below re-attempts the browser connect and would
                                // hard-error (serp_from_args with None is never
                                // reached from the needs_browser branch otherwise).
                                // google/brave NEED a browser, so they error out.
                                Err(e) if engine == "bing" => {
                                    tracing::debug!(
                                        "no Chrome for bing browser path ({e}); running pure HTTP"
                                    );
                                    let result =
                                        crate::tools::serp_from_args(&arguments, None).await;
                                    let is_err = result.get("status").and_then(|v| v.as_str())
                                        == Some("error");
                                    return json!({
                                        "jsonrpc": "2.0", "id": id,
                                        "result": {
                                            "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}],
                                            "isError": is_err
                                        }
                                    });
                                }
                                Err(e) => {
                                    return tool_error(
                                        id,
                                        &format!(
                                            "cannot connect to a browser and guest-launch failed: {e}. Set CDP_URL or start Chrome with --remote-debugging-port=9222."
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
                // Stealth-off is per-backend: mark the resolved backend so
                // attach_and_init skips the anti-bot injection for this session.
                if engine == "google" {
                    if let Some(b) = backend.as_ref() {
                        b.set_no_stealth(true);
                        serp_no_stealth = true;
                    }
                }
                // google|brave → continue to the shared backend connect + call_tool
                // (webrain_serp dispatches to serp_from_args with the backend).
            }
            // ponytail: pdf_render uses pdfium (no browser), so skip CDP connect too.
            if tool_name == "webrain_pdf_render" {
                let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if path.is_empty() {
                    return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":r#"{"status":"error","message":"path required"}"#}],"isError":true}});
                }
                #[cfg(feature = "pdfium")]
                {
                    let pages: Option<Vec<u32>> =
                        arguments.get("pages").and_then(|v| v.as_array()).map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_u64().map(|n| n as u32))
                                .collect()
                        });
                    let dpi = arguments
                        .get("dpi")
                        .and_then(|v| v.as_f64())
                        .map(|f| f as f32);
                    let tile_size = arguments
                        .get("tile_size")
                        .and_then(|v| v.as_u64())
                        .map(|n| n as u32);
                    let result = match webrain_core::engines::pdf_render(
                        path,
                        pages.as_deref(),
                        dpi,
                        tile_size,
                    ) {
                        Ok(v) => json!({"status":"ok","result":v}),
                        Err(e) => json!({"status":"error","message":e.to_string()}),
                    };
                    return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&result).unwrap_or_default()}],"isError":result.get("status").and_then(|v|v.as_str())==Some("error")}});
                }
                #[cfg(not(feature = "pdfium"))]
                let result = json!({"status":"error","message":"pdf_render requires --features pdfium AND pdfium.dll/libpdfium.so in PATH. Download from https://github.com/bblanchon/pdfium-binaries. Rebuild: cargo build --features pdfium"});
                #[cfg(not(feature = "pdfium"))]
                return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&result).unwrap_or_default()}],"isError":result.get("status").and_then(|v|v.as_str())==Some("error")}});
            }
            // ponytail: pdf_images uses lopdf (no browser), so skip CDP connect too.
            if tool_name == "webrain_pdf_images" {
                let path = arguments.get("path").and_then(|v| v.as_str()).unwrap_or("");
                if path.is_empty() {
                    return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":r#"{"status":"error","message":"path required"}"#}],"isError":true}});
                }
                let pages: Option<Vec<u32>> =
                    arguments.get("pages").and_then(|v| v.as_array()).map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_u64().map(|n| n as u32))
                            .collect()
                    });
                let path = path.to_string();
                let result = match tokio::task::spawn_blocking(move || {
                    webrain_core::engines::pdf_images(&path, pages.as_deref())
                })
                .await
                {
                    Ok(v) => match v {
                        Ok(v) => json!({"status":"ok","result":v}),
                        Err(e) => json!({"status":"error","message":e.to_string()}),
                    },
                    Err(e) => {
                        json!({"status":"error","message":format!("pdf_images task failed: {e}")})
                    }
                };
                return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&result).unwrap_or_default()}],"isError":result.get("status").and_then(|v|v.as_str())==Some("error")}});
            }
            if tool_name == "webrain_pdf_extract" {
                let results = if let Some(paths) = arguments.get("paths").and_then(|v| v.as_array())
                {
                    let p: Vec<String> = paths
                        .iter()
                        .filter_map(|x| x.as_str().map(String::from))
                        .collect();
                    // ponytail: PDF parsing is CPU-bound — run the parallel batch
                    // on the blocking pool so a huge batch never stalls a tokio worker.
                    tokio::task::spawn_blocking(move || {
                        webrain_core::engines::pdf_extract_batch(&p)
                    })
                    .await
                    .unwrap_or_default()
                } else if let Some(path) = arguments.get("path").and_then(|v| v.as_str()) {
                    let path = path.to_string();
                    tokio::task::spawn_blocking(move || {
                        vec![
                            webrain_core::engines::pdf_extract(&path)
                                .unwrap_or_else(|e| json!({"error": e.to_string()})),
                        ]
                    })
                    .await
                    .unwrap_or_default()
                } else {
                    return json!({
                        "jsonrpc": "2.0", "id": id,
                        "result": {
                            "content": [{"type": "text", "text": serde_json::to_string(&json!({"status": "error", "message": "path or paths required"})).unwrap_or_else(|_| "{}".into())}],
                            "isError": true
                        }
                    });
                };
                let result = json!({"status": "ok", "count": results.len(), "results": results});
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}],
                        "isError": false
                    }
                });
            }
            // ponytail: vault listing needs no browser — serve without a CDP backend.
            if tool_name == "webrain_profiles" {
                let result = match tokio::task::spawn_blocking(webrain_core::vault::list).await {
                    Ok(Ok(profiles)) => {
                        json!({"status": "ok", "count": profiles.len(), "profiles": profiles})
                    }
                    Ok(Err(e)) => json!({"status": "error", "message": e.to_string()}),
                    Err(e) => {
                        json!({"status": "error", "message": format!("vault list task failed: {e}")})
                    }
                };
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}],
                        "isError": result.get("status").and_then(|v| v.as_str()) == Some("error")
                    }
                });
            }

            // ponytail: launch/login manage their own Chrome + backend — serve
            // without the session backend (the launched Chrome lives in a registry).
            if tool_name == "webrain_launch" {
                let service = arguments
                    .get("service")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let profile = arguments
                    .get("profile")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if service.is_empty() || profile.is_empty() {
                    return tool_error(id, "service and profile required");
                }
                let url = arguments
                    .get("url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("https://accounts.google.com")
                    .to_string();
                let headless = arguments
                    .get("headless")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let port: u16 = arguments
                    .get("port")
                    .and_then(|v| v.as_u64())
                    // u16::try_from rejects >65535 instead of silently truncating
                    // (e.g. 70000 → 4464) to an unintended local port.
                    .and_then(|n| u16::try_from(n).ok())
                    .unwrap_or(9222);
                let result = match webrain_core::launch::launch_chrome(
                    service, profile, port, !headless,
                ) {
                    Ok(l) => {
                        let cdp_url = l.cdp_url.clone();
                        let profile_dir = l.profile_dir.clone();
                        tools::store_launched(&format!("{service}:{profile}"), l);
                        // prime: attach applies stealth, navigate opens the login page
                        let prime = async {
                            let b = CdpBackend::connect_with_url(&cdp_url).await?;
                            b.navigate(&url).await?;
                            Ok::<_, anyhow::Error>(())
                        }
                        .await;
                        match prime {
                            Ok(_) => {
                                json!({"status": "ok", "cdp_url": cdp_url, "profile_dir": profile_dir.display().to_string(), "opened": url})
                            }
                            Err(e) => {
                                json!({"status": "ok", "cdp_url": cdp_url, "profile_dir": profile_dir.display().to_string(), "warning": format!("launched but attach failed: {e}")})
                            }
                        }
                    }
                    Err(e) => json!({"status": "error", "message": e.to_string()}),
                };
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_default()}],
                        "isError": result.get("status").and_then(|v| v.as_str()) == Some("error")
                    }
                });
            }
            if tool_name == "webrain_login" {
                let service = arguments
                    .get("service")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let profile = arguments
                    .get("profile")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if service.is_empty() || profile.is_empty() {
                    return tool_error(id, "service and profile required");
                }
                let url = arguments
                    .get("url")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                let port: u16 = arguments
                    .get("port")
                    .and_then(|v| v.as_u64())
                    // u16::try_from rejects >65535 instead of silently truncating.
                    .and_then(|n| u16::try_from(n).ok())
                    .unwrap_or(9222);
                // Creds: vault first (in-process decrypt), else env — never argv/logs.
                let (user, pass, totp) = match webrain_core::vault::get(service, profile) {
                    Ok(c) => (c.username, c.password, c.totp),
                    Err(_) => (
                        std::env::var("WEBRAIN_USER").unwrap_or_default(),
                        std::env::var("WEBRAIN_PASS").unwrap_or_default(),
                        None,
                    ),
                };
                if user.is_empty() || pass.is_empty() {
                    return tool_error(
                        id,
                        "no credentials for this profile — run `webrain vault set <service> <profile>`",
                    );
                }
                // A caller-supplied `cdp_url` logs in in THEIR browser. See
                // `vault_cdp_target` for the tenant caveat on the state file.
                let cdp = vault_cdp_target(&arguments, port);
                let result = match CdpBackend::connect_with_url(&cdp).await {
                    Ok(b) => match webrain_core::login::run_login(
                        &b,
                        &user,
                        &pass,
                        totp.as_deref(),
                        url.as_deref(),
                    )
                    .await
                    {
                        Ok(r) => json!({"status": "ok", "result": r}),
                        Err(e) => json!({"status": "error", "message": e.to_string()}),
                    },
                    Err(e) => {
                        json!({"status": "error", "message": format!("cannot connect to {cdp}: {e}")})
                    }
                };
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_default()}],
                        "isError": result.get("status").and_then(|v| v.as_str()) == Some("error")
                    }
                });
            }

            if tool_name == "webrain_close_launch" {
                let service = arguments
                    .get("service")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let profile = arguments
                    .get("profile")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if service.is_empty() || profile.is_empty() {
                    return tool_error(id, "service and profile required");
                }
                let closed = tools::close_launched(&format!("{service}:{profile}"));
                let result = json!({"status": "ok", "closed": closed});
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_default()}],
                        "isError": false
                    }
                });
            }

            // ── Portable auth state (agent-browser --state/--restore borrow) ──
            // state.json per profile: cookies + localStorage, so a login follows
            // you across machines. Restore assumes the page is already on the
            // right origin (localStorage is origin-scoped).
            if tool_name == "webrain_save_state" || tool_name == "webrain_restore_state" {
                let service = arguments
                    .get("service")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let profile = arguments
                    .get("profile")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if service.is_empty() || profile.is_empty() {
                    return tool_error(id, "service and profile required");
                }
                let port: u16 = arguments
                    .get("port")
                    .and_then(|v| v.as_u64())
                    // u16::try_from rejects >65535 instead of silently truncating.
                    .and_then(|n| u16::try_from(n).ok())
                    .unwrap_or(9222);
                let dir = webrain_core::launch::profiles_dir()
                    .join(service)
                    .join(profile);
                let path = dir.join("state.json");
                // `cdp_url` when the caller brought a browser, else this host's
                // Chrome on `port`. The state FILE still lands under
                // `profiles_dir()` either way -- see `vault_cdp_target`.
                let cdp = vault_cdp_target(&arguments, port);
                let result = if tool_name == "webrain_save_state" {
                    match CdpBackend::connect_with_url(&cdp).await {
                        Ok(b) => {
                            // A dead page still answers `connect`, so both reads can fail
                            // after a successful attach. Swallowing that reported
                            // `{status: ok, cookies: 0}`, and in file mode it overwrote a
                            // good state.json with an empty one -- a silent logout that
                            // the restore side alone could not detect.
                            let cookies = match b.cookies().await {
                                Ok(c) => c,
                                Err(e) => {
                                    return tool_error(id, &format!("cannot read cookies: {e}"));
                                }
                            };
                            let ls = match b.evaluate(SAVE_LS_JS).await {
                                Ok(v) => v,
                                Err(e) => {
                                    return tool_error(
                                        id,
                                        &format!("cannot read localStorage: {e}"),
                                    );
                                }
                            };
                            let cookie_count = cookies.len();
                            let key_count = ls.as_object().map(|o| o.len()).unwrap_or(0);
                            let payload = json!({"cookies": cookies, "localStorage": ls});
                            // `inline` returns the state to the CALLER and writes no
                            // file. The platform's vault is the durable copy, and a
                            // file here would leave a credential on a worker the
                            // caller does not necessarily own.
                            if inline_state_requested(&arguments) {
                                json!({
                                    "status": "ok",
                                    "cookies": cookie_count,
                                    "localStorage_keys": key_count,
                                    "payload": payload
                                })
                            } else {
                                match (
                                    std::fs::create_dir_all(&dir),
                                    std::fs::write(
                                        &path,
                                        serde_json::to_vec(&payload).unwrap_or_default(),
                                    ),
                                ) {
                                    (Ok(_), Ok(_)) => json!({
                                        "status": "ok",
                                        "state": path.display().to_string(),
                                        "cookies": cookie_count,
                                        "localStorage_keys": key_count
                                    }),
                                    (Err(e), _) | (_, Err(e)) => json!({
                                        "status": "error",
                                        "message": format!("write failed: {e}")
                                    }),
                                }
                            }
                        }
                        Err(e) => json!({
                            "status": "error",
                            "message": format!("cannot connect to {cdp}: {e}")
                        }),
                    }
                } else {
                    // The platform's vault hands the state back inline; the file is
                    // only the fallback for a worker that captured it itself. A
                    // machine that never logged in has no `state.json` at all, so
                    // requiring one would make a vault restore impossible there.
                    let payload: Value = match inline_state(&arguments) {
                        Some(state) => state,
                        None => match std::fs::read(&path) {
                            Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
                            Err(e) => {
                                return tool_error(
                                    id,
                                    &format!("no state.json at {} ({e})", path.display()),
                                );
                            }
                        },
                    };
                    match CdpBackend::connect_with_url(&cdp).await {
                        Ok(b) => {
                            let cookies = payload
                                .get("cookies")
                                .and_then(|c| c.as_array())
                                .cloned()
                                .unwrap_or_default();
                            // Discarding this error reported `cookies_restored: N`
                            // for a restore that wrote nothing, so a caller relying
                            // on the count would skip a re-login over a dead session.
                            // Fail loudly instead; `save_state` already propagates.
                            if let Err(e) = b.set_cookies(&cookies).await {
                                return tool_error(
                                    id,
                                    &format!("cannot restore {} cookies: {e}", cookies.len()),
                                );
                            }
                            let ls = payload.get("localStorage").cloned().unwrap_or_default();
                            let keys = b.evaluate(&restore_ls_js(&ls)).await.unwrap_or_default();
                            json!({
                                "status": "ok",
                                "cookies_restored": cookies.len(),
                                "localStorage_keys": keys.as_i64().unwrap_or(0)
                            })
                        }
                        Err(e) => json!({
                            "status": "error",
                            "message": format!("cannot connect to {cdp}: {e}")
                        }),
                    }
                };
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_default()}],
                        "isError": result.get("status").and_then(|v| v.as_str()) == Some("error")
                    }
                });
            }

            if backend.is_none() {
                let mut res = if let Some(url) = cdp_url {
                    CdpBackend::connect_with_url(url).await
                } else {
                    CdpBackend::connect_default().await
                };
                // A call that names its own backends in `cdp_urls` does not need the
                // shared endpoint: a box running only lightpanda containers has
                // nothing on the default 9222, and the arm used to die here before
                // the fan-out ever ran. Fall back to the first listed backend.
                if res.is_err() {
                    if let Some(first) = arguments
                        .get("cdp_urls")
                        .and_then(|v| v.as_array())
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                    {
                        res = CdpBackend::connect_with_url(first).await;
                    }
                }
                match res {
                    Ok(b) => *backend = Some(b),
                    Err(e) => {
                        return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": format!("Cannot connect to browser: {e}. Set CDP_URL or start Chrome with --remote-debugging-port=9222.")}});
                    }
                }
            }
            let result = tools::call_tool(backend.as_ref().unwrap(), &tool_name, &arguments).await;
            // Scoped no-stealth: the google serp arm armed set_no_stealth on the
            // shared session backend — clear it now so later browse/batch/scrape
            // calls in this session don't inherit the google-only no-stealth mode.
            if serp_no_stealth {
                if let Some(b) = backend.as_ref() {
                    b.set_no_stealth(false);
                }
            }
            // Reconnect: a browser kill/restart wedges the cached backend forever
            // (a dead socket fails every later write). `CdpBackend` now LATCHES the
            // death the moment the transport dies, so ask it directly — that answer
            // cannot go stale the way a message-keyword list does. The substring
            // heuristic stays only as a fallback for failures that never reach the
            // backend at all (a refused dial, an OS-level reset mid-handshake).
            // The rest of the error surface is tool-level and must NOT reset the
            // session.
            let transport_dead = backend.as_ref().is_some_and(|b| b.is_dead());
            if transport_dead || is_connection_error(&result) {
                *backend = None;
            }
            // MCP spec: tools/call result must be {content: [{type:"text",text}], isError}.
            // ponytail: JSON-encode the tool payload into one text block; mark isError
            // when the tool returned status:"error". Without the content array the
            // client JS throws "r.content is not iterable".
            let is_error = result.get("status").and_then(|v| v.as_str()) == Some("error");
            let text = serde_json::to_string(&result).unwrap_or_else(|_| "{}".to_string());
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "content": [{"type": "text", "text": text}],
                    "isError": is_error
                }
            })
        }
        _ => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": format!("Unknown method: {method}")}
        }),
    }
}

/// The caller's own CDP endpoint, when the call supplied one.
///
/// One predicate for the vault ops AND the cookie pair, so the two cannot drift
/// on what counts as an endpoint. A blank string is NOT one: a caller who sent
/// `"  "` has not given us a browser, and treating the value as a target would
/// turn a no-op into a destination. That matters beyond these ops -- the deepscrape
/// gateway clears its own-worker guard on exactly this answer, so "present" has
/// to mean "we can actually dial it".
pub(crate) fn caller_cdp(arguments: &Value) -> Option<String> {
    arguments
        .get("cdp_url")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The CDP endpoint a vault op should use.
///
/// A caller-supplied `cdp_url` WINS over `port`: that endpoint is the caller's
/// own browser, so the op acts on it instead of on a Chrome on this host. This
/// is what lets `save_state` / `restore_state` / `login` work against a browser
/// the caller brought -- a hosted CDP provider, or their own machine. Without
/// one, the loopback form built from `port` is used: the historical behaviour,
/// and the only correct target when this worker launched its own Chrome.
///
/// A caller-supplied endpoint does NOT make the surrounding op tenant-safe on a
/// shared instance. `save_state` writes `state.json` under `profiles_dir()`,
/// which is PROCESS-GLOBAL, so the file can outlive the lease that wrote it. A
/// deployment serving BYO callers has to point `WEBRAIN_PROFILES_DIR` at a
/// per-tenant path, or keep these ops refused through the gateway
/// (`WEBRAIN_NO_STATE` covers the shared-instance case).
/// Whether `save_state` should RETURN the state instead of writing it.
///
/// The `state.json` file is process-global (`profiles_dir()`), so a caller that
/// keeps the durable copy itself asks for the payload and leaves nothing here.
fn inline_state_requested(arguments: &Value) -> bool {
    arguments
        .get("inline")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// The state payload a caller supplied inline, when it is an object.
///
/// Only an object is accepted: `restore_state` reads `cookies` and `localStorage`
/// off it, so any other JSON would restore nothing and still answer `ok`.
fn inline_state(arguments: &Value) -> Option<Value> {
    arguments.get("payload").filter(|v| v.is_object()).cloned()
}

fn vault_cdp_target(arguments: &Value, port: u16) -> String {
    caller_cdp(arguments).unwrap_or_else(|| format!("http://127.0.0.1:{port}"))
}

/// True when a tool result is a dead-socket error (browser killed/restarted),
/// which warrants dropping the cached backend so the next call reconnects.
/// Tool-level errors (element not found, CDP error, JS exception) are not.
fn is_connection_error(result: &Value) -> bool {
    result.get("status").and_then(|v| v.as_str()) == Some("error")
        && result
            .get("message")
            .and_then(|v| v.as_str())
            .map(|m| {
                let m = m.to_lowercase();
                [
                    "10054",
                    "10053",
                    "10058",
                    "connection reset",
                    "connection closed",
                    "connection aborted",
                    "broken pipe",
                    "stream closed",
                    "eof",
                    "closed before message",
                    // CdpBackend::send_cmd_with bails on the *writer task* dying:
                    // `CDP write failed: {SendError}` renders as "channel closed",
                    // which shares no phrase with "connection closed" above — so a
                    // killed browser used to wedge the cached backend permanently.
                    "channel closed",
                    "cdp write failed",
                    "cdp connection closed",
                    // Unanswered command on a half-open socket
                    // (Target.getTargets timed out after 30s) — same wedge.
                    "cdp command",
                ]
                .iter()
                .any(|k| m.contains(k))
            })
            .unwrap_or(false)
}

/// An MCP client can only *display* a picture if the reply carries an `image`
/// content block. A `screenshot_b64` inside a text block is just a megabyte of
/// base64 the model has to decode by hand (write a file, then view it), and it
/// bills as text tokens. Move it into a real image block and drop it from the
/// JSON, so the text half stays readable.
fn hoist_screenshot(result: &mut Value) {
    let Some(content) = result.get_mut("content").and_then(|c| c.as_array_mut()) else {
        return;
    };
    let mut image: Option<Value> = None;
    for block in content.iter_mut() {
        if block.get("type").and_then(|t| t.as_str()) != Some("text") {
            continue;
        }
        let Some(text) = block
            .get("text")
            .and_then(|t| t.as_str())
            .map(str::to_string)
        else {
            continue;
        };
        let Ok(mut payload) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let Some(obj) = payload.as_object_mut() else {
            continue;
        };
        let Some(b64) = obj.remove("screenshot_b64") else {
            continue;
        };
        let mime = obj
            .remove("mime")
            .and_then(|m| m.as_str().map(str::to_string))
            .unwrap_or_else(|| "image/png".to_string());
        block["text"] = json!(serde_json::to_string(&payload).unwrap_or_default());
        image = Some(json!({"type": "image", "data": b64, "mimeType": mime}));
        break;
    }
    if let Some(img) = image {
        content.push(img);
    }
}

/// Estimate output token cost at the serialization choke point — one place covers
/// every tool (browser-harness tracks usage; here it's computed, no daemon state).
/// Also stamps wall-clock `ms` so the LLM sees per-tool-run cost + latency.
/// ponytail: exact BPE via tiktoken-rs (cl100k_base vocab compiled in, zero infra —
/// no runtime download, matches OpenAI/Anthropic-style billing closely).
fn with_token_cost(mut resp: Value, start: std::time::Instant) -> Value {
    // ponytail: build the BPE once, reuse for every response (~2MB vocab, don't
    // re-parse per call). encode_with_special_tokens handles <|...|> literally.
    static BPE: std::sync::OnceLock<tiktoken_rs::CoreBPE> = std::sync::OnceLock::new();
    if let Some(result) = resp.get_mut("result") {
        hoist_screenshot(result);
        let mut text = String::new();
        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    text.push_str(t);
                }
            }
        }
        let chars = text.chars().count();
        // Big replies (base64 screenshots, vision tiles, PDF pages) cost ~1M u32
        // tokens of CPU and memory to measure exactly, for a number nobody acts
        // on. Above the cap, report the chars/4 estimate the field name already
        // promises. shortcut: 32 KiB, raise it if exact counts for huge payloads
        // ever matter.
        let est_tokens = if text.len() > 32 * 1024 {
            chars / 4
        } else {
            BPE.get_or_init(|| tiktoken_rs::cl100k_base().expect("bundled cl100k_base vocab"))
                .encode_with_special_tokens(&text)
                .len()
        };
        result["tokens"] = json!({"chars": chars, "est_tokens": est_tokens});
        result["ms"] = json!(start.elapsed().as_millis() as u64);
    }
    resp
}

/// stdio transport: newline-delimited JSON-RPC on stdin, one session at a time.
pub async fn run_stdio() -> anyhow::Result<()> {
    let stdin = std::io::stdin();
    let mut reader = std::io::BufReader::new(stdin.lock());
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    let mut backend: Option<CdpBackend> = None;

    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let t0 = std::time::Instant::now();
        let response = with_token_cost(handle_rpc(msg, &mut backend, None).await, t0);
        writeln!(writer, "{}", serde_json::to_string(&response)?)?;
        writer.flush()?;
    }

    Ok(())
}

/// HTTP transport (lightpanda `mcp --port` style): POST JSON-RPC bodies with
/// `Mcp-Session-Id` header routing. Each session owns one CdpBackend, so a
/// client's navigate→extract sequence persists across separate HTTP requests.
/// ponytail: session = minted on initialize, reused via header, one backend each.
struct SessionMeta {
    backend: tokio::sync::Mutex<Option<CdpBackend>>,
    /// CDP_URL this session was opened with (None = inherited from env).
    cdp_url: Option<String>,
    /// Last time this session served a request — drives LRU eviction at the cap.
    used: std::sync::Mutex<std::time::Instant>,
}

impl Default for SessionMeta {
    fn default() -> Self {
        Self {
            backend: tokio::sync::Mutex::new(None),
            cdp_url: None,
            used: std::sync::Mutex::new(std::time::Instant::now()),
        }
    }
}

type Session = Arc<SessionMeta>;

struct HttpState {
    sessions: tokio::sync::Mutex<std::collections::HashMap<String, Session>>,
    next_id: std::sync::atomic::AtomicU64,
}

/// Max accepted HTTP request body. MCP tool args are small; anything bigger is
/// a broken/malicious client — reject it instead of buffering until OOM.
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// True when a bind address is reachable only from this machine.
///
/// Loopback is the only protection an unauthenticated listener has, so this decides
/// whether a missing token is survivable. Only an IP literal's own loopback-ness, or
/// the exact name `localhost`, counts: an unresolved name, `0.0.0.0` and `[::]` bind
/// every interface, so they are treated as exposed.
fn bind_is_loopback(addr: &str) -> bool {
    if let Ok(socket_addr) = addr.parse::<std::net::SocketAddr>() {
        return socket_addr.ip().is_loopback();
    }
    let host = addr.rsplit_once(':').map_or(addr, |(h, _)| h);
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .eq_ignore_ascii_case("localhost")
}

pub async fn run_http(addr: &str) -> anyhow::Result<()> {
    // Fail CLOSED on an unauthenticated listener that is not loopback-only. The
    // alternative is the failure the exposure plan warns about: an operator forgets
    // WEBRAIN_HTTP_TOKEN and the engine serves every route to anything that can route
    // to the host, silently. Refusing to start turns that into an immediate, explicit
    // error. Container-internal `0.0.0.0` — a published port confined to the host's
    // loopback — opts out with WEBRAIN_ALLOW_UNAUTHENTICATED=1, so the unsafe case has
    // to be *stated* rather than inherited from a default.
    let token_set = std::env::var("WEBRAIN_HTTP_TOKEN")
        .map(|t| !t.trim().is_empty())
        .unwrap_or(false);
    let allow_unauthenticated = std::env::var("WEBRAIN_ALLOW_UNAUTHENTICATED")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "on" | "true" | "yes"
            )
        })
        .unwrap_or(false);
    if !token_set && !allow_unauthenticated && !bind_is_loopback(addr) {
        anyhow::bail!(
            "refusing to serve HTTP on {addr} without authentication: an unauthenticated \
             listener on a non-loopback address is an open engine. Set WEBRAIN_HTTP_TOKEN, \
             bind 127.0.0.1, or set WEBRAIN_ALLOW_UNAUTHENTICATED=1 if this port is \
             reachable only from inside a container network"
        );
    }

    let listener = tokio::net::TcpListener::bind(addr).await?;
    let state = Arc::new(HttpState {
        sessions: tokio::sync::Mutex::new(Default::default()),
        next_id: std::sync::atomic::AtomicU64::new(1),
    });
    // In-flight connection count, so shutdown drains instead of severing a batch
    // mid-scrape. Fly sends SIGTERM on scale-in and rolling deploys.
    let inflight = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    tracing::info!("webrain-mcp HTTP listening on {addr}");

    let mut shutdown = std::pin::pin!(shutdown_signal());
    loop {
        let accepted = tokio::select! {
            r = listener.accept() => r,
            () = &mut shutdown => {
                tracing::info!("shutdown signal received — draining");
                break;
            }
        };
        // ponytail: one flaky accept (conn reset / transient) must NOT kill the
        // whole server — log + keep serving. This `?` was the exit-1 crash.
        let (socket, _) = match accepted {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("accept error: {e} — continuing");
                continue;
            }
        };
        let state = state.clone();
        let inflight = inflight.clone();
        inflight.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tokio::spawn(async move {
            let _ = handle_http_conn(socket, state).await;
            inflight.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
    }

    // Drain window. ponytail: poll loop, no new dependency; the ceiling is that
    // one hung request holds the drain until the deadline.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while inflight.load(std::sync::atomic::Ordering::Relaxed) > 0
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    tracing::info!(
        "drained — {} request(s) still in flight",
        inflight.load(std::sync::atomic::Ordering::Relaxed)
    );
    Ok(())
}

/// Resolves on SIGTERM (Unix) or Ctrl-C (any platform). SIGTERM is the one that
/// matters in production: Fly sends it on scale-in and rolling deploys.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => return,
                _ = tokio::signal::ctrl_c() => return,
            }
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

async fn handle_http_conn(
    socket: tokio::net::TcpStream,
    state: Arc<HttpState>,
) -> anyhow::Result<()> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut socket = socket;
    let mut buf = Vec::new();
    let mut tmp = [0u8; 4096];
    loop {
        let n = socket.read(&mut tmp).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&tmp[..n]);
        if let Some(head_end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..head_end]);
            // Request line first: /health is answered before any MCP parsing so
            // the autoscaler gets a cheap liveness answer.
            let request_line = head.lines().next().unwrap_or("");
            let mut rl = request_line.split_whitespace();
            let _http_method = rl.next().unwrap_or("POST");
            let http_path = rl.next().unwrap_or("/");
            // Deliberately the ONLY extra route. A /readyz would be a lie here:
            // CDP backends attach per session, so a process that is listening is
            // genuinely ready to serve — there is no separate warm-up to report.
            // Unauthenticated by design, so the probe needs no credential.
            if http_path == "/health" {
                let body = "{\"status\":\"ok\"}";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                break;
            }
            let mut content_length = 0usize;
            let mut session_id: Option<String> = None;
            let mut authorization: Option<String> = None;
            for line in head.lines() {
                let lower = line.to_lowercase();
                if let Some(rest) = lower.strip_prefix("content-length:") {
                    content_length = rest.trim().parse().unwrap_or(0);
                } else if let Some(rest) = lower.strip_prefix("mcp-session-id:") {
                    session_id = Some(rest.trim().to_string());
                } else if lower.starts_with("authorization:") {
                    // Take the VALUE from the original line: the token is
                    // case-sensitive and `lower` would destroy it.
                    authorization = line.split_once(':').map(|(_, v)| v.trim().to_string());
                }
            }
            // Optional bearer auth. Unset/empty WEBRAIN_HTTP_TOKEN keeps today's
            // behaviour, so stdio and loopback users are untouched. This is NOT
            // the tenancy boundary — arachnefly is — it only makes a
            // mis-published port survivable. It sits before the body read and
            // before dispatch, so every route below inherits it: the only
            // exemption is /health above, which answers the autoscaler and
            // therefore holds no session state to protect.
            let expected_token = std::env::var("WEBRAIN_HTTP_TOKEN").ok();
            if !bearer_allows(expected_token.as_deref(), authorization.as_deref()) {
                // ONE body for missing and wrong alike: distinguishing them would
                // turn the 401 into an oracle for probing the secret, and an empty
                // body left the caller's log with a bare status and no reason.
                // Never echoes the expected value.
                const DENY_BODY: &str = "401 Unauthorized: missing or invalid bearer token\n";
                let resp = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{DENY_BODY}",
                    DENY_BODY.len()
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                break;
            }
            // Bound the request body: a malicious/broken client can declare a
            // huge Content-Length and stream bytes — without a cap this grows
            // `buf` until memory exhaustion (the 64 KiB guard below only covers
            // the header phase, never the body read).
            if content_length > MAX_BODY_BYTES {
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 413 Payload Too Large\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    )
                    .await;
                break;
            }
            while buf.len() < head_end + 4 + content_length {
                let m = socket.read(&mut tmp).await?;
                if m == 0 {
                    break;
                }
                buf.extend_from_slice(&tmp[..m]);
            }
            let body = &buf[head_end + 4..(head_end + 4 + content_length).min(buf.len())];
            let msg: Value = serde_json::from_slice(body).unwrap_or(Value::Null);

            // Session routing: mint on initialize, reuse via Mcp-Session-Id header.
            let is_initialize = msg.get("method").and_then(|v| v.as_str()) == Some("initialize");
            let session_key = if is_initialize {
                let id = format!(
                    "sess-{}",
                    state
                        .next_id
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                );
                let mut map = state.sessions.lock().await;
                // Bound the map: a long-lived server must not leak one session
                // (and its CDP backend) per initialize forever. Evict an entry
                // when the cap is hit — cooperative clients use webrain_close_session.
                const MAX_SESSIONS: usize = 64;
                if map.len() >= MAX_SESSIONS {
                    // Evict the least-recently-used session, not an arbitrary key:
                    // HashMap order is hash order, so `keys().next()` could drop the
                    // caller's own live session while idle ones survived.
                    let stale = map
                        .iter()
                        .min_by_key(|(_, s)| {
                            s.used
                                .lock()
                                .map(|t| *t)
                                .unwrap_or_else(|_| std::time::Instant::now())
                        })
                        .map(|(k, _)| k.clone());
                    if let Some(stale) = stale {
                        map.remove(&stale);
                    }
                }
                map.insert(id.clone(), Arc::new(Default::default()));
                id
            } else {
                session_id.unwrap_or_else(|| "default".to_string())
            };

            // ── session management tools (no CDP connect, operate on HttpState) ──
            let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let id = msg.get("id").cloned();
            let t0 = std::time::Instant::now();
            let (tool_name, args) = if method == "tools/call" {
                let mut name = msg
                    .pointer("/params/name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let mut a = msg
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(Value::Null);
                // Fold the consolidated surface so session-mgmt detection below
                // (webrain_session{op:open_session} → webrain_open_session, …)
                // and the fall-through to handle_rpc both see the executor name.
                if let Some((n, fa)) = tools::map_surface(name, &a) {
                    name = n;
                    a = fa;
                }
                (name, a)
            } else {
                ("", Value::Null)
            };

            // Tool-level session routing: an optional `session_id` in the call's
            // arguments routes the backend to that session's cdp_url — so
            // webrain_open_session(cdp_url=obscura) actually switches navigate/
            // batch/setcookies to obscura, instead of everything hitting the
            // header session's default (Chrome). Management tools operate on
            // HttpState directly and stay on the header session.
            let is_session_mgmt = matches!(
                tool_name,
                "webrain_open_session" | "webrain_close_session" | "webrain_list_sessions"
            );
            let route_key = if is_session_mgmt {
                session_key.clone()
            } else {
                args.get("session_id")
                    .and_then(|v| v.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| session_key.clone())
            };

            // Grab the (routed) session Arc, drop the map lock, then await on it.
            let session = {
                let mut map = state.sessions.lock().await;
                map.entry(route_key.clone())
                    .or_insert_with(|| Arc::new(Default::default()))
                    .clone()
            };
            // Touch the stamp so cap eviction sees this session as in use.
            if let Ok(mut t) = session.used.lock() {
                *t = std::time::Instant::now();
            }
            let mut backend = session.backend.lock().await;

            let resp = match method {
                "tools/call" => match tool_name {
                    // Shared-instance guard, repeated here deliberately. These three
                    // are answered in THIS layer and never reach handle_rpc, so the
                    // guard there does not cover them: `webrain_session(op=list)`
                    // folds to `webrain_list_sessions` and was served on a shared
                    // worker, enumerating every tenant's sessions. The arm must stay
                    // ABOVE the identical unguarded arms — Rust takes the first match,
                    // so with no_state() false this falls through to them unchanged.
                    "webrain_open_session" | "webrain_close_session" | "webrain_list_sessions"
                        if no_state() =>
                    {
                        tool_error(
                            id,
                            "webrain_session is disabled on this instance: WEBRAIN_NO_STATE is set",
                        )
                    }
                    "webrain_open_session" => {
                        let sid = args
                            .get("session_id")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                            .unwrap_or_else(|| {
                                format!(
                                    "sess-{}",
                                    state
                                        .next_id
                                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                                )
                            });
                        let cdp = args
                            .get("cdp_url")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        let meta = Arc::new(SessionMeta {
                            backend: tokio::sync::Mutex::new(None),
                            cdp_url: cdp.clone(),
                            used: std::sync::Mutex::new(std::time::Instant::now()),
                        });
                        let mut map = state.sessions.lock().await;
                        let exists = map.contains_key(&sid);
                        map.insert(sid.clone(), meta);
                        json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&json!({"session_id":sid,"cdp_url":cdp,"created":!exists})).unwrap_or_default()}],"isError":false}})
                    }
                    "webrain_close_session" => {
                        let sid = args
                            .get("session_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if sid.is_empty() || sid == "default" {
                            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":r#"{"error":"cannot close default session"}"#}],"isError":true}})
                        } else {
                            let removed = state.sessions.lock().await.remove(sid).is_some();
                            json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&json!({"session_id":sid,"closed":removed})).unwrap_or_default()}],"isError":false}})
                        }
                    }
                    "webrain_list_sessions" => {
                        let map = state.sessions.lock().await;
                        let list: Vec<Value> = map
                            .iter()
                            .map(|(k, v)| json!({"session_id":k,"cdp_url":v.cdp_url}))
                            .collect();
                        json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&json!({"sessions":list})).unwrap_or_default()}],"isError":false}})
                    }
                    _ => {
                        if msg.is_null() {
                            json!({"jsonrpc": "2.0", "id": Value::Null, "error": {"code": -32700, "message": "Parse error"}})
                        } else {
                            handle_rpc(msg, &mut backend, session.cdp_url.as_deref()).await
                        }
                    }
                },
                _ => {
                    if msg.is_null() {
                        json!({"jsonrpc": "2.0", "id": Value::Null, "error": {"code": -32700, "message": "Parse error"}})
                    } else {
                        handle_rpc(msg, &mut backend, session.cdp_url.as_deref()).await
                    }
                }
            };
            let resp = with_token_cost(resp, t0);
            let resp_body = serde_json::to_string(&resp)?;
            let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close",
                resp_body.len()
            );
            if is_initialize {
                response.push_str(&format!("\r\nMcp-Session-Id: {session_key}"));
            }
            response.push_str("\r\n\r\n");
            socket.write_all(response.as_bytes()).await?;
            socket.write_all(resp_body.as_bytes()).await?;
            socket.flush().await?;
            break;
        }
        if buf.len() > 64 * 1024 {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dead_socket_detection() {
        // Socket death → drop the cached backend so the next call reconnects.
        assert!(is_connection_error(&json!(
            {"status":"error","message":"os error 10054: An existing connection was forcibly closed by the remote host"}
        )));
        assert!(is_connection_error(&json!(
            {"status":"error","message":"connection reset by peer"}
        )));
        assert!(is_connection_error(&json!(
            {"status":"error","message":"the stream closed before message completed"}
        )));
        // The two shapes webrain itself emits after a browser kill. "channel closed"
        // shares no phrase with "connection closed", which is the regression this pins.
        assert!(is_connection_error(&json!(
            {"status":"error","message":"CDP write failed: channel closed"}
        )));
        assert!(is_connection_error(&json!(
            {"status":"error","message":"CDP command Target.getTargets timed out after 30s"}
        )));
        // Tool-level errors / success → keep the session.
        assert!(!is_connection_error(&json!(
            {"status":"error","message":"no element at index 3"}
        )));
        assert!(!is_connection_error(&json!({"status":"ok","result":1})));
    }

    // ── vault CDP target ─────────────────────────────────────────────────────
    // The regression this pins: `save_state` / `restore_state` / `login` built
    // `http://127.0.0.1:{port}` unconditionally, so a caller who brought their
    // own browser got "cannot connect to http://127.0.0.1:9222" -- the ops could
    // not address the endpoint they were handed.

    // ── inline state (phase 4: the whole profile, not just the jar) ──────────

    #[test]
    fn inline_state_requested_is_opt_in_and_boolean() {
        // Opt-in: absent or any non-boolean keeps the historical file write, so
        // an existing caller is unchanged.
        assert!(!inline_state_requested(&json!({})));
        assert!(!inline_state_requested(&json!({"inline": "true"})));
        assert!(!inline_state_requested(&json!({"inline": false})));
        assert!(inline_state_requested(&json!({"inline": true})));
    }

    #[test]
    fn inline_state_accepts_only_an_object() {
        // `restore_state` reads `cookies` and `localStorage` off this value, so a
        // scalar or array would restore nothing and still answer `ok`.
        assert_eq!(
            inline_state(&json!({"payload": {"cookies": [], "localStorage": {}}})),
            Some(json!({"cookies": [], "localStorage": {}}))
        );
        assert_eq!(inline_state(&json!({"payload": "junk"})), None);
        assert_eq!(inline_state(&json!({"payload": [1, 2]})), None);
        assert_eq!(inline_state(&json!({})), None);
    }

    #[test]
    fn vault_cdp_target_prefers_the_callers_endpoint() {
        let args = json!({"cdp_url": "wss://chrome.browserless.io?token=x"});
        assert_eq!(
            vault_cdp_target(&args, 9222),
            "wss://chrome.browserless.io?token=x"
        );
    }

    #[test]
    fn vault_cdp_target_falls_back_to_loopback() {
        // Absent, blank, or a non-string is NOT an endpoint, so the historical
        // loopback form stays -- it is the only correct target when this worker
        // launched its own Chrome.
        assert_eq!(vault_cdp_target(&json!({}), 9222), "http://127.0.0.1:9222");
        assert_eq!(
            vault_cdp_target(&json!({"cdp_url": "   "}), 9222),
            "http://127.0.0.1:9222"
        );
        assert_eq!(
            vault_cdp_target(&json!({"cdp_url": 42}), 9222),
            "http://127.0.0.1:9222"
        );
    }

    #[test]
    fn vault_cdp_target_trims_a_pasted_endpoint() {
        // These arrive pasted from a provider's dashboard, whitespace included.
        assert_eq!(
            vault_cdp_target(&json!({"cdp_url": "  ws://host:9222  "}), 9222),
            "ws://host:9222"
        );
    }

    #[test]
    fn caller_cdp_treats_a_blank_value_as_no_endpoint() {
        // The gateway clears its own-worker guard on this answer, so "present"
        // has to mean "we can dial it" -- a `"  "` that counted as an endpoint
        // would turn a no-op into a destination.
        assert_eq!(caller_cdp(&json!({"cdp_url": "   "})), None);
        assert_eq!(caller_cdp(&json!({"cdp_url": 42})), None);
        assert_eq!(caller_cdp(&json!({})), None);
        assert_eq!(
            caller_cdp(&json!({"cdp_url": " ws://host:9222 "})),
            Some("ws://host:9222".to_string())
        );
    }

    // ── shared-instance guard ────────────────────────────────────────────────
    // Exercises the REAL predicate the dispatcher uses — fold the umbrella, then
    // test both names — rather than asserting on the list. The regression this
    // pins: `webrain_session(op=list)` folds to `webrain_list_sessions`, which no
    // list of stateful executor names contained, so it was served on a shared
    // worker and enumerated every tenant's sessions.

    #[test]
    fn shared_instance_guard_refuses_every_session_op() {
        let guarded = |name: &str, args: Value| {
            let folded = tools::map_surface(name, &args)
                .map(|(n, _)| n.to_string())
                .unwrap_or_else(|| name.to_string());
            is_stateful(&folded) || is_stateful(name)
        };

        // Every op of the umbrella, including the ones whose folded name is in no
        // list. Refusing the umbrella is what makes this fail closed: a ninth op
        // added upstream is guarded the day it ships.
        for op in [
            "open",
            "close",
            "list",
            "cookies",
            "setcookies",
            "save_state",
            "restore_state",
            "profiles",
            "login",
            "close_launch",
        ] {
            assert!(
                guarded("webrain_session", json!({"op": op})),
                "session(op={op}) must be refused on a shared instance"
            );
        }

        // The direct legacy spellings that this predicate itself covers. Note
        // `webrain_open_session` / `webrain_close_session` / `webrain_list_sessions`
        // are NOT here: they are answered by the outer HTTP layer and guarded by a
        // match arm there, which is verified live by
        // `scripts/probe_http_transport.ps1` rather than by this predicate.
        for name in ["webrain_profiles", "webrain_launch", "webrain_login"] {
            assert!(guarded(name, json!({})), "{name} must be refused");
        }

        // Stateless tools stay reachable, or the shared pool serves nothing.
        for name in [
            "webrain_scrape",
            "webrain_search",
            "webrain_guide",
            "webrain_pdf",
        ] {
            assert!(!guarded(name, json!({})), "{name} must stay available");
        }
    }

    // ── HTTP bind guard ───────────────────────────────────────────────────
    // The predicate decides whether an unauthenticated listener is survivable, so
    // the addresses that look loopback-ish but are not get pinned here.

    #[test]
    fn bind_is_loopback_only_for_loopback_literals() {
        for addr in [
            "127.0.0.1:9223",
            "127.0.0.5:9223",
            "[::1]:9223",
            "localhost:9223",
        ] {
            assert!(bind_is_loopback(addr), "{addr} must count as loopback");
        }

        // `0.0.0.0` and `[::]` bind EVERY interface — the exact mistake this guard
        // exists to catch, and the shape docker-compose has to opt out of explicitly.
        // An unresolved name is not assumed to be loopback for the same reason.
        for addr in [
            "0.0.0.0:9223",
            "[::]:9223",
            "192.168.1.10:9223",
            "webrain.internal:9223",
            "example.com:9223",
            "",
        ] {
            assert!(
                !bind_is_loopback(addr),
                "{addr} must NOT count as loopback (it is exposed or unresolvable)"
            );
        }
    }

    // ── HTTP bearer auth ──────────────────────────────────────────────────
    // The gate is split so the env read is the only impure part. Every test
    // below passes the expected token in; nothing here calls set_var, because
    // tests share one process and an env mutation would leak into a sibling
    // test (and flake under the parallel harness). Assertions stay on
    // truthiness/shape — a failing assertion must never print a credential.

    #[test]
    fn auth_is_open_when_no_token_is_configured() {
        // The contract this must preserve: no configuration ⇒ no auth. Blank and
        // whitespace-only count as unset, otherwise a stray `WEBRAIN_HTTP_TOKEN=`
        // line in a deploy manifest would lock out every caller.
        assert!(bearer_allows(None, None), "unset + no header must pass");
        assert!(bearer_allows(Some(""), None), "empty must pass");
        assert!(
            bearer_allows(Some("   "), None),
            "whitespace-only must pass"
        );
        // ...and a caller that sends a header anyway is not punished for it.
        assert!(bearer_allows(None, Some("Bearer whatever")));
        assert!(bearer_allows(Some(""), Some("Bearer whatever")));
    }

    #[test]
    fn auth_is_enforced_when_token_is_set() {
        let expected = "unit-test-secret";

        // Correct token, in each shape a real client emits.
        assert!(bearer_allows(
            Some(expected),
            Some("Bearer unit-test-secret")
        ));
        assert!(bearer_allows(
            Some(expected),
            Some("bearer unit-test-secret")
        ));
        assert!(bearer_allows(
            Some(expected),
            Some("BEARER unit-test-secret")
        ));
        assert!(
            bearer_allows(Some(expected), Some("Bearer   unit-test-secret ")),
            "header padding is not part of the secret"
        );
        // Unset vs blank on the CONFIG side, with a real header present.
        assert!(bearer_allows(Some("  "), Some("Bearer unit-test-secret")));

        // Rejections.
        assert!(!bearer_allows(Some(expected), None), "header missing");
        assert!(!bearer_allows(Some(expected), Some("")), "header empty");
        assert!(
            !bearer_allows(Some(expected), Some("Bearer")),
            "scheme, no credential"
        );
        assert!(
            !bearer_allows(Some(expected), Some("Bearer unit-test-secrxt")),
            "same length, one byte differs"
        );
        assert!(
            !bearer_allows(Some(expected), Some("Bearer unit-test-secret-x")),
            "longer"
        );
        assert!(
            !bearer_allows(Some(expected), Some("Bearer unit-test-secre")),
            "shorter"
        );
        assert!(
            !bearer_allows(Some(expected), Some("unit-test-secret")),
            "Bearer scheme required"
        );
        assert!(
            !bearer_allows(Some(expected), Some("Basic unit-test-secret")),
            "wrong scheme"
        );
        assert!(
            !bearer_allows(Some(expected), Some("Bearer Unit-Test-Secret")),
            "token is case-sensitive"
        );
    }

    #[test]
    fn bearer_header_parse_is_scheme_folded_but_value_exact() {
        // Length/shape only — the extracted credential is never asserted on.
        assert_eq!(bearer_token("Bearer abc").map(str::len), Some(3));
        assert_eq!(
            bearer_token("BEARER   abc ").map(str::len),
            Some(3),
            "trimmed"
        );
        assert!(bearer_token("bearer abc").is_some());
        assert!(
            bearer_token("Bearer").is_none(),
            "no credential after the scheme"
        );
        assert!(
            bearer_token("Basic abc").is_none(),
            "only the Bearer scheme"
        );
        assert!(bearer_token("").is_none());
    }

    #[test]
    fn token_compare_is_length_gated_and_byte_exact() {
        // Why this is not `==`: a plain comparison short-circuits at the first
        // differing byte, so a wrong first byte answers faster than a wrong last
        // byte and the response time leaks the shared prefix. That property is
        // structural (`token_eq` never returns early inside the loop) and cannot
        // be asserted reliably by timing in CI — so this pins the OBSERVABLE
        // contract the constant-time loop must still satisfy.
        assert!(token_eq(b"abc", b"abc"));
        assert!(token_eq(b"", b""));
        assert!(!token_eq(b"abc", b"xbc"), "first byte differs");
        assert!(!token_eq(b"abc", b"abx"), "last byte differs");
        // Length gating: a prefix must not be accepted by a truncated compare.
        assert!(
            !token_eq(b"abc", b"abcd"),
            "expected is a prefix of the token"
        );
        assert!(
            !token_eq(b"abc", b"ab"),
            "token is a prefix of the expected"
        );
        assert!(!token_eq(b"", b"a"), "empty vs non-empty");
    }
}
