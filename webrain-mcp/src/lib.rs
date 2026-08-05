// webrain-mcp: Model Context Protocol server for AI model browser automation.
//
// 25 tools: webrain_navigate, webrain_eval (JS→JSON), webrain_screenshot,
// webrain_click/type/scroll (index-based), webrain_get_html, webrain_spider
// (BFS/DFS), webrain_snapshot (D1 skip), webrain_pixel (PixelRAG tiles),
// webrain_extract_json (CSS/XPath schema), webrain_extract_regex, webrain_pdf,
// webrain_tab (multi-tab), webrain_a11y (accessibility tree),
// webrain_semantic_tree (text snapshot), webrain_batch
// (fetch/extract/screenshot over URLs), webrain_download (http+ytdlp),
// webrain_search (4 engines), webrain_nav, webrain_press, webrain_get_images,
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
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

            // No-browser tools: serve without a CDP backend, so they work
            // on a fresh install before any browser is up (any-LLM portability).
            if tool_name == "webrain_guide" {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&json!({"status": "ok", "guide": tools::AGENT_GUIDE})).unwrap_or_else(|_| "{}".into())}],
                        "isError": false
                    }
                });
            }
            // ponytail: fetch_http uses ureq (no browser), so skip CDP connect too.
            if tool_name == "webrain_fetch_http" {
                let url = arguments.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if url.is_empty() {
                    return json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "content": [{"type": "text", "text": serde_json::to_string(&json!({"status": "error", "message": "url required"})).unwrap_or_else(|_| "{}".into())}],
                            "isError": true
                        }
                    });
                }
                let result = match webrain_core::engines::http_fetch(url) {
                    Ok(v) => json!({"status": "ok", "result": v}),
                    Err(e) => json!({"status": "error", "message": e.to_string()}),
                };
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
                return json!({
                    "jsonrpc": "2.0", "id": id,
                    "result": {
                        "content": [{"type": "text", "text": serde_json::to_string(&result).unwrap_or_else(|_| "{}".into())}],
                        "isError": false
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
                let encoded = q.replace(' ', "+");
                let url = match engine {
                    "bing" => format!("https://www.bing.com/search?q={encoded}"),
                    "brave" => format!("https://search.brave.com/search?q={encoded}"),
                    "google" => format!("https://www.google.com/search?q={encoded}"),
                    _ => format!("https://html.duckduckgo.com/html/?q={encoded}"),
                };
                let result = match webrain_core::engines::http_fetch(&url) {
                    Ok(v) => {
                        json!({"status":"ok","engine":engine,"url":url,"text":v.get("text").cloned().unwrap_or_default()})
                    }
                    Err(e) => json!({"status":"error","message":e.to_string()}),
                };
                return json!({"jsonrpc":"2.0","id":id,"result":{"content":[{"type":"text","text":serde_json::to_string(&result).unwrap_or_default()}],"isError":result.get("status").and_then(|v|v.as_str())==Some("error")}});
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
                let result = match webrain_core::engines::pdf_images(path, pages.as_deref()) {
                    Ok(v) => json!({"status":"ok","result":v}),
                    Err(e) => json!({"status":"error","message":e.to_string()}),
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
                let result = match webrain_core::vault::list() {
                    Ok(profiles) => {
                        json!({"status": "ok", "count": profiles.len(), "profiles": profiles})
                    }
                    Err(e) => json!({"status": "error", "message": e.to_string()}),
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
                    .map(|n| n as u16)
                    .unwrap_or(9222);
                let result = match webrain_core::launch::launch_chrome(
                    &service, &profile, port, !headless,
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
                    .map(|n| n as u16)
                    .unwrap_or(9222);
                // Creds: vault first (in-process decrypt), else env — never argv/logs.
                let (user, pass, totp) = match webrain_core::vault::get(&service, &profile) {
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
                let cdp = format!("http://127.0.0.1:{port}");
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
                    .map(|n| n as u16)
                    .unwrap_or(9222);
                let dir = webrain_core::launch::profiles_dir()
                    .join(service)
                    .join(profile);
                let path = dir.join("state.json");
                let cdp = format!("http://127.0.0.1:{port}");
                let result = if tool_name == "webrain_save_state" {
                    match CdpBackend::connect_with_url(&cdp).await {
                        Ok(b) => {
                            let cookies = b.cookies().await.unwrap_or_default();
                            let ls = b.evaluate(SAVE_LS_JS).await.unwrap_or_default();
                            let payload = json!({"cookies": cookies, "localStorage": ls});
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
                                    "cookies": cookies.len(),
                                    "localStorage_keys": ls.as_object().map(|o| o.len()).unwrap_or(0)
                                }),
                                (Err(e), _) | (_, Err(e)) => json!({
                                    "status": "error",
                                    "message": format!("write failed: {e}")
                                }),
                            }
                        }
                        Err(e) => json!({
                            "status": "error",
                            "message": format!("cannot connect to {cdp}: {e}")
                        }),
                    }
                } else {
                    let payload: Value = match std::fs::read(&path) {
                        Ok(b) => serde_json::from_slice(&b).unwrap_or_default(),
                        Err(e) => {
                            return tool_error(
                                id,
                                &format!("no state.json at {} ({e})", path.display()),
                            );
                        }
                    };
                    match CdpBackend::connect_with_url(&cdp).await {
                        Ok(b) => {
                            let cookies = payload
                                .get("cookies")
                                .and_then(|c| c.as_array())
                                .cloned()
                                .unwrap_or_default();
                            let _ = b.set_cookies(&cookies).await;
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
                let res = if let Some(url) = cdp_url {
                    CdpBackend::connect_with_url(url).await
                } else {
                    CdpBackend::connect_default().await
                };
                match res {
                    Ok(b) => *backend = Some(b),
                    Err(e) => {
                        return json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": format!("Cannot connect to browser: {e}. Set CDP_URL or start Chrome with --remote-debugging-port=9222.")}});
                    }
                }
            }
            let result = tools::call_tool(backend.as_ref().unwrap(), tool_name, &arguments).await;
            // Reconnect: a browser kill/restart wedges the cached backend forever
            // (dead socket → "os error 10054" on every write). Drop it on
            // connection-level errors so the next call connects fresh. The rest
            // of the error surface is tool-level and must NOT reset the session.
            if is_connection_error(&result) {
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
                ]
                .iter()
                .any(|k| m.contains(k))
            })
            .unwrap_or(false)
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
        let mut text = String::new();
        if let Some(content) = result.get("content").and_then(|c| c.as_array()) {
            for block in content {
                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                    text.push_str(t);
                }
            }
        }
        let chars = text.chars().count();
        // vocab is compiled in via include_bytes! — this cannot fail at runtime.
        let est_tokens = BPE
            .get_or_init(|| tiktoken_rs::cl100k_base().expect("bundled cl100k_base vocab"))
            .encode_with_special_tokens(&text)
            .len();
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
}

impl Default for SessionMeta {
    fn default() -> Self {
        Self {
            backend: tokio::sync::Mutex::new(None),
            cdp_url: None,
        }
    }
}

type Session = Arc<SessionMeta>;

struct HttpState {
    sessions: tokio::sync::Mutex<std::collections::HashMap<String, Session>>,
    next_id: std::sync::atomic::AtomicU64,
}

pub async fn run_http(addr: &str) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let state = Arc::new(HttpState {
        sessions: tokio::sync::Mutex::new(Default::default()),
        next_id: std::sync::atomic::AtomicU64::new(1),
    });
    tracing::info!("webrain-mcp HTTP listening on {addr}");
    loop {
        // ponytail: one flaky accept (conn reset / transient) must NOT kill the
        // whole server — log + keep serving. This `?` was the exit-1 crash.
        let (socket, _) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("accept error: {e} — continuing");
                continue;
            }
        };
        let state = state.clone();
        tokio::spawn(async move {
            let _ = handle_http_conn(socket, state).await;
        });
    }
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
            let mut content_length = 0usize;
            let mut session_id: Option<String> = None;
            for line in head.lines() {
                let lower = line.to_lowercase();
                if let Some(rest) = lower.strip_prefix("content-length:") {
                    content_length = rest.trim().parse().unwrap_or(0);
                } else if let Some(rest) = lower.strip_prefix("mcp-session-id:") {
                    session_id = Some(rest.trim().to_string());
                }
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
                state
                    .sessions
                    .lock()
                    .await
                    .insert(id.clone(), Arc::new(Default::default()));
                id
            } else {
                session_id.unwrap_or_else(|| "default".to_string())
            };

            // ── session management tools (no CDP connect, operate on HttpState) ──
            let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
            let id = msg.get("id").cloned();
            let t0 = std::time::Instant::now();
            let (tool_name, args) = if method == "tools/call" {
                (
                    msg.pointer("/params/name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    msg.pointer("/params/arguments")
                        .cloned()
                        .unwrap_or(Value::Null),
                )
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
            let mut backend = session.backend.lock().await;

            let resp = match method {
                "tools/call" => match tool_name {
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
        // Tool-level errors / success → keep the session.
        assert!(!is_connection_error(&json!(
            {"status":"error","message":"no element at index 3"}
        )));
        assert!(!is_connection_error(&json!({"status":"ok","result":1})));
    }
}
