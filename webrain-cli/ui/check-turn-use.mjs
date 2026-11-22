// The turn accumulator, checked without a browser. The panel's label is "this
// turn" and a turn is several model calls, so `addUse` is what makes the numbers
// grow instead of jumping backwards every time the runtime reports a new call —
// the exact bug that made the stats flash while the model was thinking.
//
//   node check-turn-use.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");
const start = src.indexOf("  function addUse(acc, u) {");
const end = src.indexOf("\n  }", start);
assert.ok(start > 0 && end > start, "addUse not found in App.svelte");
const addUse = new Function(src.slice(start, end + 4) + "\n  return addUse;")();

// The first call seeds the total; nothing is invented for fields it lacks.
const one = addUse(null, { model: "m1", inputTokens: 10, outputTokens: 5, duration: 100 });
assert.deepEqual(one, {
  model: "m1",
  inputTokens: 10,
  outputTokens: 5,
  cacheReadTokens: 0,
  duration: 100,
  calls: 1,
});

// A second call adds to it. This is the assertion the bug would have failed:
// reading the last call instead of the sum went 10 → 7, which reads as a refund.
const two = addUse(one, { model: "m1", inputTokens: 7, outputTokens: 3, cacheReadTokens: 4, duration: 50 });
assert.equal(two.inputTokens, 17);
assert.equal(two.outputTokens, 8);
assert.equal(two.cacheReadTokens, 4);
assert.equal(two.duration, 150);
assert.equal(two.calls, 2);

// A call with no numbers of its own adds nothing rather than resetting the sum.
const quiet = addUse(two, {});
assert.equal(quiet.inputTokens, 17);
assert.equal(quiet.duration, 150);
assert.equal(quiet.calls, 3);
assert.equal(quiet.model, "m1", "a call without a model keeps the last one");

// A turn that switched models is described by where it ended.
assert.equal(addUse(two, { model: "m2", outputTokens: 1 }).model, "m2");

console.log("turn accumulator: 4 checks passed");
