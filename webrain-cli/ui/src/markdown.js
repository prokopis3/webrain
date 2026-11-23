// Markdown rendering, in its own chunk.
//
// `marked` + DOMPurify are the two biggest things in the bundle and nothing on
// the page needs them until the first answer arrives, so they are imported
// dynamically and this module is what lands in `chunk-markdown.js`. Model output
// is untrusted — it can carry HTML and this page can approve shell commands — so
// every render is sanitised, on every call, with no fast path.
import { marked } from "marked";
import DOMPurify from "dompurify";

// Escapes for text **and** attribute position. The quotes matter: a renderer
// builds markup by hand, so a `"` left in a url closes the `href` it sits in
// before DOMPurify ever sees the string — the sanitiser is the second gate, not
// the first, and it cannot repair an attribute that no longer exists.
const esc = (s) =>
  String(s).replace(
    /[&<>"']/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c],
  );

marked.setOptions({ gfm: true, breaks: true });

// The fenced language as the card should say it. `rs` and `rust` are one thing to
// a reader, and a fence with no language is still a code block — without this a
// card said `rs`, `Rust` or `code` depending on who wrote the fence.
const LANG = {
  js: "js", javascript: "js", mjs: "js", cjs: "js", jsx: "jsx",
  ts: "ts", typescript: "ts", tsx: "tsx",
  rs: "rust", rust: "rust", go: "go", golang: "go",
  py: "python", python: "python", rb: "ruby", ruby: "ruby",
  sh: "shell", bash: "shell", zsh: "shell", ps1: "shell", powershell: "shell", console: "shell",
  json: "json", jsonc: "json", json5: "json",
  yml: "yaml", yaml: "yaml", toml: "toml", ini: "ini", env: "env",
  xml: "xml", html: "html", svg: "svg", css: "css", scss: "css", less: "css",
  sql: "sql", md: "markdown", markdown: "markdown",
  c: "c", h: "c", cpp: "cpp", "c++": "cpp", hpp: "cpp", cc: "cpp",
  cs: "c#", csharp: "c#", java: "java", kt: "kotlin", kotlin: "kotlin",
  swift: "swift", php: "php", lua: "lua", dart: "dart", vue: "vue", svelte: "svelte",
  diff: "diff", patch: "diff", text: "text", plaintext: "text",
};

const langLabel = (raw) => {
  const tag = String(raw || "").trim().toLowerCase();
  return LANG[tag] || tag || "code";
};

// The host of a url, for a destination card. A relative path has none, and a
// string that does not parse is left alone rather than guessed at.
function hostOf(href) {
  try {
    return new URL(href).host.replace(/^www\./, "");
  } catch {
    return "";
  }
}

// A fenced block becomes a card with its language and its own copy button. The
// code text is re-escaped here because a renderer receives it raw.
marked.use({
  renderer: {
    code(token) {
      const { text, lang } = typeof token === "string" ? { text: token } : token;
      const tag = (lang || "").split(/\s+/)[0];
      return `<div class="code"><div class="bar"><span>${esc(langLabel(tag))}</span><button class="code-copy" type="button">copy</button></div><pre><code>${esc(text ?? "")}</code></pre></div>`;
    },

    // A picture arrives as a frame you can click, with its own words under it —
    // the alt text is the only caption the model gave it, so it is not thrown
    // away. Both attributes are re-escaped: a renderer sees raw source and the
    // sanitiser runs afterwards, so nothing here may be trusted twice.
    image(token) {
      const href = token.href || "";
      const alt = token.text || "";
      const cap = alt || token.title || "";
      const tag = cap ? `<figcaption>${esc(cap)}</figcaption>` : "";
      return `<figure class="fig"><a href="${esc(href)}" target="_blank" rel="noopener noreferrer"><img src="${esc(href)}" alt="${esc(alt)}" loading="lazy"></a>${tag}</figure>`;
    },

    // A link whose text *is* the url is a destination, not a word in a sentence:
    // it gets a card with the host read apart from the path. A normal `[label]`
    // stays an inline link, because a card mid-paragraph breaks the line.
    link(token) {
      const href = token.href || "";
      const bare = !!href && token.text === href;
      const label =
        bare || !token.tokens?.length ? esc(token.text || href) : this.parser.parseInline(token.tokens);
      const title = token.title ? ` title="${esc(token.title)}"` : "";
      if (!bare) {
        return `<a href="${esc(href)}"${title}>${label}</a>`;
      }
      const host = hostOf(href);
      const path = host ? href.slice(href.indexOf(host) + host.length) || "/" : href;
      return `<a class="card" href="${esc(href)}"${title}><b>${esc(host || label)}</b><span>${esc(path)}</span></a>`;
    },
  },
});

// Model-written links open beside the app, never in it.
DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  if (node.tagName === "A") {
    node.setAttribute("target", "_blank");
    node.setAttribute("rel", "noopener noreferrer");
  }
  // A reply can point an image at any host. Without this, reopening a thread
  // fetches every one of them at paint time — from third parties, for rows the
  // reader has not scrolled to. Lazy loading is the platform's own answer: the
  // request happens when the row enters the viewport, including inside a
  // collapsed `details`, which fetches nothing until it is opened.
  if (node.tagName === "IMG") {
    node.setAttribute("loading", "lazy");
    node.setAttribute("decoding", "async");
  }
});

// Everything forbidden here is already dropped by DOMPurify's defaults; naming it
// is what stops a dependency bump from quietly widening the page.
//
// `style` the *attribute* is the one that is NOT dropped by default, and this page
// can approve shell commands: `position:fixed;inset:0;z-index:9999` in a reply is
// enough to paint a fake approval dialog over the real one. `<style>` elements are
// dropped with their CSS text, which is why a rendered block needs no shadow root
// and no scoped selectors — allow them and that scoping becomes mandatory.
const CLEAN = {
  FORBID_TAGS: ["style", "script", "iframe", "object", "embed", "form", "base", "link", "meta"],
  FORBID_ATTR: ["style"],
};

export function render(text) {
  try {
    // The wrap goes on *after* sanitising: a table is the one block a reply can
    // make wider than the column, so it needs its own scroller, and a tag this
    // module emits itself is the only `<table>` left in the string by then.
    return DOMPurify.sanitize(marked.parse(text ?? "", { async: false }), CLEAN)
      .replace(/<table>/g, '<div class="table-wrap"><table>')
      .replace(/<\/table>/g, "</table></div>");
  } catch {
    return `<pre>${esc(text ?? "")}</pre>`;
  }
}

// The same sanitiser and the same table wrap, with the parser left out. Some
// models answer in HTML — Gemini and several OpenRouter endpoints write a
// document rather than markdown — and for those, parsing is what mangles the
// reply: a `<div>` on one line becomes paragraph text around a block. The
// setting chooses which of the two a reply is put through; both are sanitised
// with the same list, so the choice can never widen what the page will run.
export function renderRaw(text) {
  try {
    return DOMPurify.sanitize(String(text ?? ""), CLEAN)
      .replace(/<table>/g, '<div class="table-wrap"><table>')
      .replace(/<\/table>/g, "</table></div>");
  } catch {
    return `<pre>${esc(text ?? "")}</pre>`;
  }
}
