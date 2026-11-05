// Operator skills — the `.md` doctrine files `webrain_guide` serves as
// `topic="skill:<name>"`.
//
// Two sources, one shape:
//   - `webrain install skills`    writes the bundle below (offline, versioned
//                                 with the binary — no network, no skew).
//   - `webrain skills add <src>`  installs ONE `.md` from an http(s) URL or a
//                                 local path: someone else's skill, a gist, a
//                                 raw GitHub file.
//
// A file is a skill only if its frontmatter carries `name:`. That is what
// `webrain_guide` keys on, so a file without one is invisible — which is why
// every path below refuses it at install time instead of letting it sit there
// looking installed and never being found.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Same ceiling the reader enforces, in bytes. A skill bigger than this would
/// install and then be silently truncated on read, so it is refused up front.
pub const MAX_SKILL: usize = 64 * 1024;

/// The bundled tree, `(path under the skills dir, contents)`.
///
/// `include_str!` rather than a build-time copy: an install becomes a file
/// write with no tooling, and renaming a source file is a compile error instead
/// of a skill that quietly vanishes. `scripts/` stays out — it builds the
/// claude.ai zip, it is not doctrine.
pub const BUNDLED: &[(&str, &str)] = &[
    (
        "webrain/SKILL.md",
        include_str!("../../skills/webrain/SKILL.md"),
    ),
    (
        "webrain/auth-media-vision/SKILL.md",
        include_str!("../../skills/webrain/auth-media-vision/SKILL.md"),
    ),
    (
        "webrain/credentials/SKILL.md",
        include_str!("../../skills/webrain/credentials/SKILL.md"),
    ),
    (
        "webrain/evals/README.md",
        include_str!("../../skills/webrain/evals/README.md"),
    ),
    (
        "webrain/monitoring/SKILL.md",
        include_str!("../../skills/webrain/monitoring/SKILL.md"),
    ),
    (
        "webrain/pdf-extraction/SKILL.md",
        include_str!("../../skills/webrain/pdf-extraction/SKILL.md"),
    ),
    (
        "webrain/page-tiles/SKILL.md",
        include_str!("../../skills/webrain/page-tiles/SKILL.md"),
    ),
    (
        "webrain/product-market-research/SKILL.md",
        include_str!("../../skills/webrain/product-market-research/SKILL.md"),
    ),
    (
        "webrain/prompts/news-research-agent.md",
        include_str!("../../skills/webrain/prompts/news-research-agent.md"),
    ),
    (
        "webrain/references/anti-patterns.md",
        include_str!("../../skills/webrain/references/anti-patterns.md"),
    ),
    (
        "webrain/references/browser-selection.md",
        include_str!("../../skills/webrain/references/browser-selection.md"),
    ),
    (
        "webrain/references/challenges.md",
        include_str!("../../skills/webrain/references/challenges.md"),
    ),
    (
        "webrain/references/core-rules.md",
        include_str!("../../skills/webrain/references/core-rules.md"),
    ),
    (
        "webrain/references/extraction.md",
        include_str!("../../skills/webrain/references/extraction.md"),
    ),
    (
        "webrain/references/profiles.md",
        include_str!("../../skills/webrain/references/profiles.md"),
    ),
    (
        "webrain/web-research/SKILL.md",
        include_str!("../../skills/webrain/web-research/SKILL.md"),
    ),
    (
        "webrain/workflows/captcha-solve.md",
        include_str!("../../skills/webrain/workflows/captcha-solve.md"),
    ),
    (
        "webrain/workflows/feed-and-news.md",
        include_str!("../../skills/webrain/workflows/feed-and-news.md"),
    ),
    (
        "webrain/workflows/protected-site.md",
        include_str!("../../skills/webrain/workflows/protected-site.md"),
    ),
];

/// Where skills live: `WEBRAIN_SKILLS_DIR` when set, else `<engine cache>/skills`
/// so one env var (`WEBRAIN_BROWSERS_DIR`) moves engines and skills together.
///
/// The override is not optional decoration: `webrain install skills` prints it as
/// the way to pin the directory, and an agent that shares a skill tree with another
/// tool (`.agents/skills`, a container mount) has nowhere else to point.
pub fn dir() -> PathBuf {
    dir_or(std::env::var_os("WEBRAIN_SKILLS_DIR"))
}

fn dir_or(override_dir: Option<std::ffi::OsString>) -> PathBuf {
    match override_dir.filter(|v| !v.is_empty()) {
        Some(p) => PathBuf::from(p),
        None => crate::install::browsers_dir().join("skills"),
    }
}

/// First `name:` inside the YAML frontmatter. `None` when the file has none —
/// which is how a reference chapter is told apart from a skill.
pub fn frontmatter_name(text: &str) -> Option<String> {
    frontmatter(text, "name")
}

/// One scalar from the frontmatter — `name`, `description`. Only the first 40
/// lines are read and only a single-line value is returned: a skill's own
/// `name`/`description` are the only keys this project reads, and a folded
/// multi-line description would need a real YAML parser for no gain.
pub fn frontmatter(text: &str, key: &str) -> Option<String> {
    // A UTF-8 BOM would make the opening `---` invisible to `lines()`.
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for line in lines.take(40) {
        let line = line.trim();
        if line == "---" {
            break;
        }
        if let Some(v) = line
            .strip_prefix(key)
            .and_then(|rest| rest.strip_prefix(':'))
        {
            let v = v.trim().trim_matches(['"', '\'']).trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// A name that survives both a filename and `topic="skill:<name>"`: lowercase
/// alnum, `-`, `_`, ≤64 chars. Everything else (dots, slashes, colons, spaces)
/// is refused — this is the reader's own rule applied before anything is
/// written, so an installed skill is always a findable one.
pub fn slug(raw: &str) -> Option<String> {
    let s = raw.trim().trim_matches(['"', '\'']).trim().to_lowercase();
    let ok = !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    ok.then_some(s)
}

/// Every skill under `dir`, recursive, as `(name, path)` sorted by name.
///
/// The bundle nests (`webrain/web-research/SKILL.md`), so a flat `read_dir`
/// would see a single file. Name authority: frontmatter `name:`, then the parent
/// folder of a `SKILL.md`, then the stem — but only at the top level, which is
/// the old flat-file contract and has to keep working.
pub fn list(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    walk(dir, dir, 0, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn walk(dir: &Path, root: &Path, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    if depth > 4 {
        // ponytail: 4 levels — a skill tree deeper than that is a layout bug,
        // not a use case.
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, root, depth + 1, out);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        let name = match frontmatter_name(&text) {
            Some(n) => Some(n),
            None if stem.eq_ignore_ascii_case("SKILL") => path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .map(str::to_string),
            None if path.parent() == Some(root) => Some(stem.to_string()),
            None => None,
        };
        if let Some(name) = name.and_then(|n| slug(&n)) {
            out.push((name, path));
        }
    }
}

/// Resolve `name` to a file under `dir`, or `None`.
///
/// The name is validated rather than trusted: this reads a file on behalf of a
/// remote model, so a path in the argument would be an arbitrary file read.
/// Canonicalizing both sides also catches a symlink inside the directory that
/// points outside it.
pub fn resolve(dir: &Path, name: &str) -> Option<PathBuf> {
    let want = slug(name)?;
    let path = list(dir).into_iter().find(|(n, _)| *n == want)?.1;
    let base = dir.canonicalize().ok()?;
    let real = path.canonicalize().ok()?;
    // Return the plain path: `canonicalize` on Windows yields `\\?\C:\…`, which is
    // pure noise in a tool reply. The verbatim form is only needed for this check.
    real.starts_with(&base).then_some(path)
}

/// Remove every file that serves `name`. "Remove this skill" means the name, not
/// one arbitrary copy of it — a leftover second copy would silently become the
/// skill again.
pub fn remove(dir: &Path, name: &str) -> Result<Vec<PathBuf>> {
    let want = slug(name)
        .with_context(|| format!("{name:?} is not a usable skill name (a-z 0-9 _ -, max 64)"))?;
    let base = dir.canonicalize().ok();
    let hits: Vec<PathBuf> = list(dir)
        .into_iter()
        .filter(|(n, _)| *n == want)
        .map(|(_, p)| p)
        // A symlink inside the tree must never become a delete outside it.
        .filter(|p| match (&base, p.canonicalize()) {
            (Some(b), Ok(real)) => real.starts_with(b),
            _ => false,
        })
        .collect();
    if hits.is_empty() {
        anyhow::bail!("no skill named `{want}` in {}", dir.display());
    }
    for path in &hits {
        std::fs::remove_file(path).with_context(|| format!("remove {}", path.display()))?;
    }
    Ok(hits)
}

/// Write the bundled tree under `target`. Returns `(written, skipped)`; without
/// `force` an existing file is left alone, so a hand-edited skill survives an
/// upgrade.
pub fn install_bundled(target: &Path, force: bool) -> Result<(usize, usize)> {
    let mut written = 0;
    let mut skipped = 0;
    for (rel, body) in BUNDLED {
        let path = target.join(rel);
        if path.exists() && !force {
            skipped += 1;
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, body).with_context(|| format!("write {}", path.display()))?;
        written += 1;
    }
    Ok((written, skipped))
}

/// Install one skill from `http(s)://…` or a local path.
///
/// The frontmatter `name:` is authoritative and there is no rename flag on
/// purpose: the reader looks skills up by that field, so letting a caller
/// choose a different name would install a skill nobody can address.
pub fn install_one(target: &Path, src: &str) -> Result<PathBuf> {
    let text = if src.starts_with("http://") || src.starts_with("https://") {
        let resp = ureq::get(src)
            .header("User-Agent", "webrain")
            .call()
            .with_context(|| format!("GET {src}"))?;
        resp.into_body()
            .read_to_string()
            .with_context(|| format!("read body of {src}"))?
    } else {
        std::fs::read_to_string(src).with_context(|| format!("read {src}"))?
    };
    if text.len() > MAX_SKILL {
        anyhow::bail!(
            "{src} is {} KiB; the cap is {} KiB",
            text.len() / 1024,
            MAX_SKILL / 1024
        );
    }
    let declared = frontmatter_name(&text).with_context(|| {
        format!(
            "{src} has no frontmatter `name:` — webrain_guide looks skills up by it, \
             so this file would install and never be found"
        )
    })?;
    let slug = slug(&declared).with_context(|| {
        format!("{declared:?} is not a usable skill name (a-z 0-9 _ -, max 64)")
    })?;
    std::fs::create_dir_all(target)?;
    let path = target.join(format!("{slug}.md"));
    // Two files serving one name would make `skill:<name>` a coin flip, and the
    // bundle ships `web-research` nested — adding it flat must not shadow it.
    for (name, existing) in list(target) {
        if name != slug {
            continue;
        }
        let same = path.exists() && existing.canonicalize().ok() == path.canonicalize().ok();
        if !same {
            anyhow::bail!(
                "a skill named `{slug}` already lives at {} — `webrain skills rm {slug}` first",
                existing.display()
            );
        }
    }
    std::fs::write(&path, text).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skills_dir_honours_the_override_and_ignores_an_empty_one() {
        assert_eq!(
            dir_or(Some(std::ffi::OsString::from("/tmp/pinned-skills"))),
            PathBuf::from("/tmp/pinned-skills")
        );
        // An empty value is "unset", not "the current directory".
        assert_eq!(
            dir_or(Some(std::ffi::OsString::new())),
            crate::install::browsers_dir().join("skills")
        );
    }

    #[test]
    fn frontmatter_name_reads_only_the_header() {
        let t = "---\nname: \"web-search\"\ndescription: x\n---\nbody name: nope\n";
        assert_eq!(frontmatter_name(t).as_deref(), Some("web-search"));
        // No frontmatter at all -> a reference chapter, not a skill.
        assert_eq!(frontmatter_name("# Heading\nname: no\n"), None);
        // BOM must not hide the opening fence.
        assert_eq!(
            frontmatter_name("\u{feff}---\nname: bom\n---\n").as_deref(),
            Some("bom")
        );
    }

    #[test]
    fn slug_refuses_paths() {
        assert_eq!(slug("Web_Research-2").as_deref(), Some("web_research-2"));
        assert_eq!(slug("../etc/passwd"), None);
        assert_eq!(slug("a/b"), None);
        assert_eq!(slug("c:evil"), None);
        assert_eq!(slug(""), None);
    }
}
