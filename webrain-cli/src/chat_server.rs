//! `webrain chat --serve`: the same turn, in a browser.
//!
//! Everything the runtime knows is already on the wire — `session.subscribe()`
//! hands back typed events, and `chat_session::run` flattens them into a pipe.
//! This module keeps the events and gives each one a place on a page:
//!
//! ```text
//! GET  /            the page (include_str!, no build step)
//! GET  /events      SSE: {"type":"<runtime event>","data":{…}} + permission asks
//! POST /turn        {"prompt":"…"}  -> 202, one turn at a time
//! POST /permission  {"id":"…","allow":true}
//! ```
//!
//! Loopback only, like `webrain mcp --http` without a token: a UI that can
//! approve a shell command must not be reachable from the LAN by default.
//!
//! The event *names* are the runtime's own (`assistant.message_delta`,
//! `tool.execution_start`, …) rather than a translated dialect: the page maps
//! them to elements, and a name we do not recognise still prints instead of
//! vanishing. `webrain.permission` is the one event this file adds.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use github_copilot_sdk::handler::{PermissionHandler, PermissionResult};
use github_copilot_sdk::types::{PermissionRequestData, RequestId, SessionId};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Mutex, broadcast, oneshot};

use super::chat::Args;
use super::chat_session;
use webrain_core::byok::ChatProvider;

/// One frame on the event bus. SSE needs a string, so the JSON is built once
/// and cloned to every client.
type Bus = broadcast::Sender<String>;

const PAGE: &str = include_str!("../assets/chat.html");

/// How long a permission prompt waits for a human before it is denied. A turn
/// that hangs on a modal nobody saw is worse than a turn that says no.
const PERMISSION_WAIT: Duration = Duration::from_secs(120);

pub async fn run(args: &Args, provider: &ChatProvider) -> anyhow::Result<()> {
    let addr = format!("127.0.0.1:{}", args.port);
    let (bus, _) = broadcast::channel::<String>(256);
    let permission = Arc::new(WebPermission {
        bus: bus.clone(),
        pending: Mutex::new(HashMap::new()),
        seq: AtomicU64::new(1),
    });

    let (client, session) = chat_session::connect(args, provider, Some(permission.clone())).await?;
    // The client must outlive the accept loop: dropping it stops the runtime.
    let _client = client;

    // One task owns the subscription and fans it out; every SSE client taps the
    // bus. Subscribing here (not per request) is what makes a delta emitted
    // during session creation reach the page.
    let mut events = session.subscribe();
    let out = bus.clone();
    // Detached on purpose: it ends when the process (and the subscription) does.
    let _reader = tokio::spawn(async move {
        while let Ok(event) = events.recv().await {
            let frame = json!({
                "type": event.event_type,
                "data": event.data,
            });
            // A closed bus just means nobody is watching.
            let _ = out.send(frame.to_string());
        }
    });

    let listener = TcpListener::bind(&addr).await?;
    eprintln!("webrain chat · serving on http://{addr}/ · Ctrl-C to stop");
    let state = Arc::new(State {
        bus,
        permission,
        session: Arc::new(session),
        timeout: Duration::from_secs(args.timeout_secs),
        busy: AtomicBool::new(false),
    });

    loop {
        let (stream, _) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = serve_conn(stream, state).await {
                eprintln!("· http · {e}");
            }
        });
    }
}

struct State {
    bus: Bus,
    permission: Arc<WebPermission>,
    session: Arc<github_copilot_sdk::session::Session>,
    timeout: Duration,
    busy: AtomicBool,
}

/// Answers a permission prompt from the page instead of the CLI's policy.
struct WebPermission {
    bus: Bus,
    pending: Mutex<HashMap<String, oneshot::Sender<bool>>>,
    seq: AtomicU64,
}

#[async_trait::async_trait]
impl PermissionHandler for WebPermission {
    async fn handle(
        &self,
        _session: SessionId,
        _request: RequestId,
        data: PermissionRequestData,
    ) -> PermissionResult {
        let id = format!("p{}", self.seq.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);
        // The page needs the exact request, not a summary: approving here is
        // the same authority as --yes.
        let ask = json!({
            "type": "webrain.permission",
            "data": {
                "id": id,
                "kind": format!("{:?}", data.kind),
                "request": format!("{data:?}"),
                "managed": data.managed_approval_required.unwrap_or(false),
            },
        });
        let _ = self.bus.send(ask.to_string());

        let answer = tokio::time::timeout(PERMISSION_WAIT, rx).await;
        self.pending.lock().await.remove(&id);
        match answer {
            Ok(Ok(true)) => PermissionResult::approve_once(),
            Ok(Ok(false)) => PermissionResult::reject(Some("denied in the web UI".into())),
            _ => PermissionResult::reject(Some("no answer from the web UI".into())),
        }
    }
}

async fn serve_conn(mut stream: TcpStream, state: Arc<State>) -> anyhow::Result<()> {
    let (head, body) = read_request(&mut stream).await?;
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or_default();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();

    match (method, path) {
        ("GET", "/") => respond(&mut stream, 200, "text/html; charset=utf-8", PAGE).await,
        ("GET", "/events") => stream_events(&mut stream, state.bus.subscribe()).await,
        ("POST", "/turn") => {
            let prompt = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| v["prompt"].as_str().map(str::to_string))
                .unwrap_or_default();
            if prompt.trim().is_empty() {
                return respond(
                    &mut stream,
                    400,
                    "application/json",
                    r#"{"error":"prompt"}"#,
                )
                .await;
            }
            // 409 rather than a queue: the runtime can run turns concurrently,
            // but a page with one composer would interleave two answers.
            if state.busy.swap(true, Ordering::SeqCst) {
                return respond(&mut stream, 409, "application/json", r#"{"error":"busy"}"#).await;
            }
            let st = state.clone();
            tokio::spawn(async move {
                let wait = github_copilot_sdk::MessageOptions::new(prompt.as_str())
                    .with_wait_timeout(st.timeout);
                if let Err(e) = st.session.send_and_wait(wait).await {
                    let _ = st.bus.send(
                        json!({"type": "webrain.error", "data": {"message": e.to_string()}})
                            .to_string(),
                    );
                }
                st.busy.store(false, Ordering::SeqCst);
            });
            respond(&mut stream, 202, "application/json", r#"{"started":true}"#).await
        }
        ("POST", "/permission") => {
            let v = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);
            let id = v["id"].as_str().unwrap_or_default().to_string();
            let allow = v["allow"].as_bool().unwrap_or(false);
            let sent = match state.permission.pending.lock().await.remove(&id) {
                Some(tx) => tx.send(allow).is_ok(),
                None => false,
            };
            let body = json!({ "answered": sent }).to_string();
            respond(&mut stream, 200, "application/json", &body).await
        }
        _ => respond(&mut stream, 404, "text/plain", "not found").await,
    }
}

async fn stream_events(
    stream: &mut TcpStream,
    mut rx: broadcast::Receiver<String>,
) -> anyhow::Result<()> {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n",
        )
        .await?;
    // A page that reloads reconnects; lagging behind means the tab was asleep,
    // which is worth a `: skip` comment rather than a dropped connection.
    loop {
        match rx.recv().await {
            Ok(frame) => {
                if stream
                    .write_all(format!("data: {frame}\n\n").as_bytes())
                    .await
                    .is_err()
                {
                    return Ok(());
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {
                if stream.write_all(b": skip\n\n").await.is_err() {
                    return Ok(());
                }
            }
            Err(broadcast::error::RecvError::Closed) => return Ok(()),
        }
    }
}

/// Read headers, then exactly `Content-Length` body bytes. Anything longer is
/// refused by the cap: this listener takes one small JSON object per request.
async fn read_request(stream: &mut TcpStream) -> anyhow::Result<(String, String)> {
    const MAX_BODY: usize = 256 * 1024;
    let mut buf = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(end) = find(&buf, b"\r\n\r\n") {
            let head = String::from_utf8_lossy(&buf[..end]).to_string();
            let want = head
                .lines()
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0)
                .min(MAX_BODY);
            let start = end + 4;
            while buf.len() < start + want {
                let n = stream.read(&mut chunk).await?;
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body =
                String::from_utf8_lossy(&buf[start..(start + want).min(buf.len())]).to_string();
            return Ok((head, body));
        }
        anyhow::ensure!(buf.len() <= MAX_BODY, "request head too large");
    }
    anyhow::bail!("empty request")
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

async fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
) -> anyhow::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n\r\n",
        reason(status),
        body.len()
    );
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(body.as_bytes()).await?;
    Ok(())
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_header_break_and_tolerates_absence() {
        assert_eq!(find(b"GET / HTTP/1.1\r\n\r\nbody", b"\r\n\r\n"), Some(14));
        assert_eq!(find(b"GET / HTTP/1.1\r\n", b"\r\n\r\n"), None);
    }

    #[test]
    fn reasons_cover_every_status_the_server_sends() {
        for s in [200u16, 202, 400, 404, 409] {
            assert_ne!(reason(s), "Error");
        }
        assert_eq!(reason(500), "Error");
    }
}
