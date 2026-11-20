// The boundary finder behind incremental streaming, checked without a browser.
// `splitStream` decides where a half-written reply can be cut so the part above
// the cut is parsed once and only the tail is re-rendered. Getting it wrong cuts
// inside a code fence and the rest of the answer renders as prose.
//
//   node check-stream-split.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");
const start = src.indexOf("  function splitStream(text) {");
const end = src.indexOf("\n  }", start);
assert.ok(start > 0 && end > start, "splitStream not found in App.svelte");
const splitStream = new Function(src.slice(start, end + 4) + "\n  return splitStream;")();

// Nothing to cut yet.
assert.equal(splitStream(""), null, "empty");
assert.equal(splitStream("one line"), null, "no blank line");
assert.equal(splitStream(null), null, "null");

// The ordinary case: the last paragraph is the tail, everything before it is head.
assert.deepEqual(splitStream("one\n\ntwo"), ["one\n\n", "two"]);
assert.deepEqual(splitStream("a\n\nb\n\nc"), ["a\n\nb\n\n", "c"], "last boundary wins");

// Inside an open fence there is no boundary: parse whole.
assert.equal(splitStream("text\n\n```js\ncode"), null, "open fence");
assert.equal(splitStream("```\ncode\n\nmore"), null, "blank line inside a fence");

// A closed fence, then prose: the boundary after it is safe.
assert.deepEqual(splitStream("```js\nx\n```\n\nprose"), ["```js\nx\n```\n\n", "prose"]);
// A boundary before the fence and no boundary after it is not safe.
assert.equal(splitStream("a\n\n```js\nx\n```"), null, "only candidate is before the fence");

// Two fences and prose after them.
const two = "```\na\n```\n\nmid\n\n```\nb\n```\n\nend";
assert.deepEqual(splitStream(two), ["```\na\n```\n\nmid\n\n```\nb\n```\n\n", "end"]);

console.log("stream split: 10 checks passed");
