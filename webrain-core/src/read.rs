//! `webrain read` — a page as markdown, over HTTP, with no browser and no model.
//!
//! Three things make this cheaper than scraping the same URL:
//!
//! 1. **Negotiation.** The request asks for `text/markdown` first. A docs site
//!    that already publishes markdown for its pages then sends it, and nothing
//!    has to be converted.
//! 2. **A `.md` retry.** Failing that, the same page is asked for as `.md`
//!    (Starlight, VitePress, Mintlify and friends all serve it). Still HTTP.
//! 3. **`llms.txt`.** The nearest `llms.txt` in an ancestor directory is
//!    reported, because it is the map of what else is worth reading — and
//!    `webrain read <that url>` reads it, since it is plain text.
//!
//! The last resort is the ordinary page, converted by the same `htmd` converter
//! `webrain_batch op=markdown` uses, so one page reads identically whether its
//! HTML came from a browser or from here.

use serde_json::{json, Value};

/// What the request says it accepts. Markdown first, then plain text (which is
/// what `llms.txt` and many `.md` routes are served as), then HTML — a server
/// that has none of the cheaper forms still answers.
pub const ACCEPT: &str = "text/markdown;q=1.0, text/plain;q=0.9, application/json;q=0.8, text/html;q=0.7, */*;q=0.1";

#[derive(Debug, Clone, Default)]
pub struct ReadOpts {
    /// Return the heading list instead of the body.
    pub outline_only: bool,
    /// Return only the section under the first heading containing this text.
    pub filter: Option<String>,
    /// Char cap on the returned body. 0 = no cap.
    pub max_chars: usize,
    /// Proxy URL, or `None` for the shared pool.
    pub proxy: Option<String>,
}

/// `https://` for a bare host, and no trailing `#fragment` in a candidate URL:
/// a fragment never reaches the server, and a `.md` route would inherit it.
fn normalize(raw: &str) -> anyhow::Result<String> {
    let t = raw.trim();
    let with_scheme = if t.contains("://") {
        t.to_string()
    } else {
        format!("https://{t}")
    };
    let mut u = url::Url::parse(&with_scheme)
        .map_err(|e| anyhow::anyhow!("invalid URL {raw}: {e}"))?;
    u.set_fragment(None);
    Ok(u.to_string())
}

fn host(u: &url::Url) -> String {
    match u.port() {
        Some(p) => format!("{}:{}", u.host_str().unwrap_or(""), p),
        None => u.host_str().unwrap_or("").to_string(),
    }
}

/// The `.md` forms of one page, most specific first: `<path>.md`, then the
/// directory's `index.md`. Both are real conventions (the first on Astro and
/// Next.js docs, the second on almost everything else), and a miss costs one
/// HTTP request that answers 404.
pub fn markdown_candidates(base: &str) -> Vec<String> {
    let Ok(u) = url::Url::parse(base) else {
        return Vec::new();
    };
    let path = u.path().to_string();
    let last = path.rsplit('/').next().unwrap_or("");
    let mut out = Vec::new();
    if path.ends_with('/') || last.is_empty() {
        out.push(format!("{}://{}{}index.md", u.scheme(), host(&u), path));
    } else {
        if !last.contains('.') {
            out.push(format!(
                "{}://{}{}.md",
                u.scheme(),
                host(&u),
                path.trim_end_matches('/')
            ));
        }
        // Worth one try even for a page path: both shapes exist in the wild.
        out.push(format!(
            "{}://{}{}/index.md",
            u.scheme(),
            host(&u),
            path.trim_end_matches('/')
        ));
    }
    out.dedup();
    out
}

/// Every `llms.txt` from the page's own directory up to the root, nearest
/// first. The convention is one file per *directory* describing that subtree,
/// so the walk is over path segments — not over the host.
pub fn llms_candidates(base: &str) -> Vec<String> {
    let Ok(u) = url::Url::parse(base) else {
        return Vec::new();
    };
    let mut segs: Vec<&str> = u.path().split('/').filter(|s| !s.is_empty()).collect();
    // A page path's last segment is a file, so its directory is one above it.
    segs.pop();
    let mut deepest_first = Vec::new();
    let mut prefix = String::new();
    for s in &segs {
        prefix.push('/');
        prefix.push_str(s);
        deepest_first.push(format!("{}://{}{}/llms.txt", u.scheme(), host(&u), prefix));
    }
    deepest_first.reverse();
    // The root is always the last candidate, and the only one for a bare host.
    deepest_first.push(format!("{}://{}/llms.txt", u.scheme(), host(&u)));
    deepest_first
}

/// HTML to markdown, through the converter the batch pipeline already uses.
/// Relative links stay as the page wrote them: a conversion cannot know the
/// base, and `fetched_url` tells the caller what to resolve them against.
pub fn html_to_markdown(html: &str) -> String {
    htmd::HtmlToMarkdown::builder()
        .skip_tags(vec!["script", "style", "noscript", "template", "svg"])
        .build()
        .convert(html)
        .unwrap_or_default()
}

// ------------------------------------------------------------------- outline

/// Every heading, with its level and 1-based line. This is the cheap first look
/// at a long page: 20 headings instead of 40 000 characters.
pub fn outline(md: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (i, line) in md.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let t = line.trim_start();
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            out.push(json!({
                "level": hashes,
                "title": t[hashes..].trim(),
                "line": i + 1,
            }));
        }
    }
    out
}

/// The body of the section introduced by the first heading containing `needle`
/// (case-insensitive), up to the next heading of the same or a higher level.
/// `None` when no heading matches — a caller handed an empty string would think
/// the section was empty.
pub fn section(md: &str, needle: &str) -> Option<String> {
    let needle_l = needle.to_lowercase();
    let lines: Vec<&str> = md.lines().collect();
    let mut start: Option<usize> = None;
    let mut level = 0;
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim_start();
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if !(1..=6).contains(&hashes) || !t[hashes..].starts_with(' ') {
            continue;
        }
        match start {
            None => {
                if t[hashes..].to_lowercase().contains(&needle_l) {
                    start = Some(i + 1);
                    level = hashes;
                }
            }
            Some(s) => {
                if hashes <= level {
                    return Some(lines[s..i].join("\n").trim().to_string());
                }
            }
        }
    }
    start.map(|s| lines[s..].join("\n").trim().to_string())
}

// --------------------------------------------------------------- orchestrate

fn is_markdown(ct: &str) -> bool {
    ct.contains("markdown")
}

fn is_plain(ct: &str) -> bool {
    ct.contains("text/plain")
}

fn is_html(ct: &str) -> bool {
    ct.contains("text/html") || ct.contains("application/xhtml")
}

fn is_json(ct: &str) -> bool {
    ct.contains("json")
}

/// Read one URL as markdown. Returns
/// `{url, fetched_url, status, content_type, source, bytes, chars, text, …}`
/// where `source` names how the body arrived — `markdown`, `text`, `html`,
/// `json` — because that is the number that explains the token count: a page
/// that negotiated markdown was never converted, and one that did not says so.
pub fn read_url(raw: &str, opts: &ReadOpts) -> anyhow::Result<Value> {
    let url = normalize(raw)?;
    let proxy = opts.proxy.as_deref();
    let mut tried: Vec<Value> = Vec::new();
    let mut got: Option<(String, u16, String, String, &'static str)> = None;

    for cand in std::iter::once(url.clone()).chain(markdown_candidates(&url)) {
        let md_candidate = cand != url;
        match crate::engines::http_get_accept(&cand, Some(ACCEPT), proxy) {
            Ok((status, ct, body)) if (200..300).contains(&status) && !body.trim().is_empty() => {
                let source = if is_markdown(&ct) {
                    "markdown"
                } else if is_plain(&ct) {
                    "text"
                } else if is_json(&ct) {
                    "json"
                } else if is_html(&ct) {
                    if md_candidate {
                        // An HTML body from a `.md` URL is a soft 404 page.
                        tried.push(json!({
                            "url": cand, "status": status, "skipped": "html from a .md route"
                        }));
                        continue;
                    }
                    "html"
                } else {
                    "text"
                };
                got = Some((cand, status, ct, body, source));
                break;
            }
            Ok((status, ct, _)) => {
                tried.push(json!({"url": cand, "status": status, "content_type": ct}));
            }
            Err(e) => tried.push(json!({"url": cand, "error": e.to_string()})),
        }
    }

    let Some((fetched_url, status, content_type, body, source)) = got else {
        anyhow::bail!(
            "nothing readable at {url} - tried {} form(s): {}",
            tried.len(),
            serde_json::to_string(&tried).unwrap_or_default()
        );
    };

    let bytes = body.chars().count();
    let full = if source == "html" {
        html_to_markdown(&body)
    } else {
        body
    };

    let mut out = json!({
        "url": url,
        "fetched_url": fetched_url,
        "status": status,
        "content_type": content_type,
        "source": source,
        "bytes": bytes,
        "chars": full.chars().count(),
        "truncated": false,
    });
    // The nearest llms.txt is a map, not content: report where it is and how big
    // it is (reading it is another `read`, since it is plain text).
    if let Some((u, c)) = nearest_llms(&url, proxy) {
        out["llms_txt"] = json!({"url": u, "chars": c});
    }
    if !tried.is_empty() {
        out["tried"] = Value::Array(tried);
    }

    if opts.outline_only {
        let items = outline(&full);
        out["headings"] = json!(items.len());
        out["outline"] = Value::Array(items);
        return Ok(out);
    }
    // Selection happens on the WHOLE body, and only then is `--max` applied: a
    // cap applied first would silently hide the section a caller asked for, and
    // `--filter x --max 200` would answer "no heading contains x" on a page that
    // does contain it.
    let selected = match &opts.filter {
        Some(needle) => match section(&full, needle) {
            Some(s) => {
                out["section"] = json!(needle);
                s
            }
            None => {
                let titles: Vec<String> = outline(&full)
                    .iter()
                    .map(|h| h["title"].as_str().unwrap_or("").to_string())
                    .collect();
                anyhow::bail!(
                    "no heading contains {needle:?} - this page has: {}",
                    if titles.is_empty() {
                        "(no headings)".to_string()
                    } else {
                        titles.join(" | ")
                    }
                );
            }
        },
        None => full,
    };
    let total = selected.chars().count();
    let truncated = opts.max_chars > 0 && total > opts.max_chars;
    out["chars"] = json!(total);
    out["truncated"] = json!(truncated);
    out["text"] = json!(if truncated {
        selected.chars().take(opts.max_chars).collect::<String>()
    } else {
        selected
    });
    Ok(out)
}

/// The nearest non-empty `llms.txt` above the page, as `(url, chars)`.
fn nearest_llms(base: &str, proxy: Option<&str>) -> Option<(String, usize)> {
    for cand in llms_candidates(base) {
        if let Ok((status, _ct, body)) = crate::engines::http_get_accept(&cand, None, proxy) {
            if (200..300).contains(&status) && !body.trim().is_empty() {
                return Some((cand, body.chars().count()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_candidates_follow_the_two_real_conventions() {
        assert_eq!(
            markdown_candidates("https://docs.example.com/guide/intro"),
            vec![
                "https://docs.example.com/guide/intro.md",
                "https://docs.example.com/guide/intro/index.md",
            ]
        );
        assert_eq!(
            markdown_candidates("https://docs.example.com/guide/"),
            vec!["https://docs.example.com/guide/index.md"]
        );
        // A path that already carries an extension is not a page slug, so the
        // only sensible guess is the directory form.
        assert_eq!(
            markdown_candidates("https://x.dev/a/file.html"),
            vec!["https://x.dev/a/file.html/index.md"]
        );
    }

    #[test]
    fn llms_walks_ancestors_nearest_first_and_ends_at_the_root() {
        assert_eq!(
            llms_candidates("https://docs.example.com/a/b/page"),
            vec![
                "https://docs.example.com/a/b/llms.txt",
                "https://docs.example.com/a/llms.txt",
                "https://docs.example.com/llms.txt",
            ]
        );
        assert_eq!(
            llms_candidates("https://example.com/"),
            vec!["https://example.com/llms.txt"]
        );
    }

    #[test]
    fn html_becomes_markdown_with_structure_and_no_script() {
        let html = r#"<html><head><script>let x=1;</script><style>b{}</style></head><body>
            <h1>Title</h1>
            <p>Some <strong>bold</strong> and <a href="/x">a link</a>.</p>
            <ul><li>one</li><li>two</li></ul>
            <pre><code>fn main() {}</code></pre>
        </body></html>"#;
        let md = html_to_markdown(html);
        assert!(md.contains("# Title"), "{md}");
        assert!(md.contains("**bold**"), "{md}");
        assert!(md.contains("[a link](/x)"), "relative link kept: {md}");
        assert!(md.contains("*   one"), "{md}");
        assert!(md.contains("fn main() {}"), "{md}");
        assert!(!md.contains("let x=1"), "script leaked: {md}");
        assert!(!md.contains("b{}"), "style leaked: {md}");
    }

    #[test]
    fn outline_skips_headings_inside_a_fence() {
        let md = "# One\ntext\n```sh\n# not a heading\n```\n## Two\n";
        let o = outline(md);
        assert_eq!(o.len(), 2, "{o:?}");
        assert_eq!(o[0]["title"], "One");
        assert_eq!(o[1]["level"], 2);
        assert_eq!(o[1]["line"], 6);
    }

    #[test]
    fn section_stops_at_the_next_heading_of_equal_or_higher_level() {
        let md = "# A\na body\n## B\nb body\n### C\nc body\n## D\nd body\n";
        assert_eq!(section(md, "b").as_deref(), Some("b body\n### C\nc body"));
        assert_eq!(
            section(md, "A").as_deref(),
            Some("a body\n## B\nb body\n### C\nc body\n## D\nd body")
        );
        assert_eq!(section(md, "nope"), None);
    }

    #[test]
    fn accept_asks_for_markdown_before_html() {
        assert!(ACCEPT.find("text/markdown").unwrap() < ACCEPT.find("text/html").unwrap());
    }

    #[test]
    fn normalize_adds_a_scheme_and_drops_the_fragment() {
        assert_eq!(normalize("example.com/a#x").unwrap(), "https://example.com/a");
        assert_eq!(
            normalize("http://example.com/a/").unwrap(),
            "http://example.com/a/"
        );
    }
}
