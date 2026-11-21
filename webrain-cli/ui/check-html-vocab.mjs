// The html-mode prompt and the stylesheet must name the same classes, or the
// model is told to build something the page cannot paint. `style`/`<style>` are
// stripped by the sanitiser, so this list *is* the vocabulary — drift in either
// direction is a silent regression: a class the CSS lost renders as bare text,
// and a shape the prompt never mentions is never used.
//
//   node check-html-vocab.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");

// The hint is a chain of string literals; decode it the way the browser will.
const hStart = src.indexOf("const HTML_HINT =");
const hEnd = src.indexOf("const SUGGEST_HINT", hStart);
assert.ok(hStart > 0 && hEnd > hStart, "HTML_HINT not found in App.svelte");
const hintParts = src
  .slice(hStart, hEnd)
  .match(/"((?:[^"\\]|\\.)*)"/g)
  .map((q) => JSON.parse(q));
const hint = hintParts.join("");
assert.ok(hint.startsWith("[render: html5]"), "hint still opens with its marker");

// The html5 block is the only place these classes are allowed to live.
const cStart = src.indexOf("/* ── html5 replies");
const cEnd = src.indexOf("/* A floating action", cStart);
assert.ok(cStart > 0 && cEnd > cStart, "html5 style block not found");
const css = src.slice(cStart, cEnd);

// Every shape the prompt promises, and every class it names, must exist.
const VOCAB = [
  "card",
  "grid",
  "wide",
  "stat",
  "pill",
  "ok",
  "warn",
  "bad",
  "num",
  "table-wrap",
];
for (const name of VOCAB) {
  assert.ok(hint.includes(name), `prompt never mentions .${name}`);
  assert.ok(css.includes(`.${name}`), `stylesheet has no rule for .${name}`);
}

// The two hazards the prompt must keep stating, because they are how a reply
// breaks the page instead of laying it out.
assert.ok(/<style>/i.test(hint) && /<script>/i.test(hint), "prompt names the stripped tags");
assert.ok(hint.includes('style=\\"\\"') || hint.includes('style=""'), "prompt forbids inline style");

// The copyable skeleton is the part models actually follow — assert it survives
// as markup rather than prose, and keep the whole instruction inside a budget a
// standing session instruction can afford.
const skeleton = hint.slice(hint.indexOf("A dashboard skeleton"));
assert.ok(skeleton.includes('class="grid"'), "skeleton shows a grid");
assert.ok(skeleton.includes('class="stat"'), "skeleton shows a stat");
assert.ok(skeleton.includes("<table>"), "skeleton shows a table");
assert.ok(skeleton.includes("<progress"), "skeleton shows a bar");
assert.ok(hint.length < 2200, `hint is ${hint.length} chars — trim before it is a per-session tax`);

console.log(`check-html-vocab: ${VOCAB.length + 6} assertions, hint ${hint.length} chars`);
