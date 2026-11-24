// The composer's mention tint, checked without a browser. Two things can break
// it silently, and both did:
//
//  * the parser — `/` only counts at the start of the draft, `#`/`@` only at a
//    word boundary, and a picked command was the one kind that never got tinted;
//  * the size — the field and the tint layer had their font-size set apart, so
//    every painted box was a pixel-per-word narrower than its glyphs and slid
//    into the next word. The wrapper owns the size now; this asserts nobody
//    takes it back.
//
//   node check-mentions.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");
const start = src.indexOf("  function markInput(s) {");
const end = src.indexOf("\n  }", start);
assert.ok(start > 0 && end > start, "markInput not found");
const esc = (s) => String(s).replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);
const markInput = new Function("esc", src.slice(start, end + 4) + "\n  return markInput;")(esc);

// The exact draft the report used. Three sigils, three classes, in order.
const draft = "/model #webrain_navigate and @auth-media-vision plus a / not-a-command";
const html = markInput(draft);
const classes = [...html.matchAll(/<mark class="(\w\w)">([^<]*)<\/mark>/g)].map((m) => [m[1], m[2]]);
assert.deepEqual(classes, [
  ["mc", "/model"],
  ["mt", "#webrain_navigate"],
  ["ms", "@auth-media-vision"],
]);

// A `/` mid-sentence is a path, not a command — the palette's own rule.
assert.ok(!markInput("plus a / not-a-command").includes("<mark"), "a mid-sentence slash was tinted");
// `@` inside a word is an address, not a mention.
assert.ok(!markInput("mail me@host").includes("<mark"), "an inline @ was tinted");
// A token keeps working after a newline and after a bracket.
assert.ok(markInput("a\n#tool").includes('<mark class="mt">#tool</mark>'));
// HTML in the draft is escaped before it is marked, never after.
assert.ok(markInput("#a<b").includes("&lt;b"));

// One owner for the composer's type size: the wrapper. Two owners is the bug.
const marksRule = /\.marks \{[^}]*\}/.exec(src)?.[0] ?? "";
assert.ok(!/font-size/.test(marksRule), ".marks sets its own font-size again");
assert.ok(/\.markwrap \{[^}]*font-size/.test(src), ".markwrap does not own the type size");
assert.ok(src.includes("textarea { min-height: 3rem; }"), "the scale pass still overrides the field's size");

// Three sigil hues, in all four palettes: a theme that misses one falls back to
// the mint theme's colour and two sigils look identical.
for (const hue of ["--m-cmd", "--m-tool", "--m-skill"]) {
  const seen = src.split(hue).length - 1;
  assert.ok(seen >= 5, `${hue} is defined in one palette but not the rest (${seen})`);
}

console.log("mentions: 12 assertions passed");
