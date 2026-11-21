// The one duration formatter, checked without a browser. `ms()` is used for a
// tool row, a reasoning block, a turn's total and the ledger's api column, so a
// scale bug shows up in four places at once — and "180.0s" for a three-minute
// crawl is exactly the kind of thing nobody notices until it is 3600s.
//
//   node check-duration.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");
const start = src.indexOf("  function ms(n) {");
const end = src.indexOf("\n  }", start);
assert.ok(start > 0 && end > start, "ms not found in App.svelte");
const ms = new Function(src.slice(start, end + 4) + "\n  return ms;")();

const S = 1000;
const M = 60 * S;
const H = 60 * M;
const D = 24 * H;

// Below a second stays in ms; nothing prints a fraction it cannot show.
assert.equal(ms(5), "5ms");
assert.equal(ms(999), "999ms");
assert.equal(ms(1500), "1.5s");

// Seconds, then minutes with seconds zero-padded so the shape does not jump.
assert.equal(ms(59 * S), "59.0s");
assert.equal(ms(60 * S), "1m 00s");
assert.equal(ms(100 * S), "1m 40s");
assert.equal(ms(180 * S), "3m 00s", "a three-minute crawl is not 180.0s");
assert.equal(ms(59 * M + 59 * S), "59m 59s");

// Hours, days, then the coarse two: months of 30 days, years of 12 of those.
assert.equal(ms(H), "1h 00m");
assert.equal(ms(H + 5 * M), "1h 05m");
assert.equal(ms(D), "1d 0h");
assert.equal(ms(D + 3 * H), "1d 3h");
assert.equal(ms(30 * D), "1mo 0d");
assert.equal(ms(45 * D), "1mo 15d");
assert.equal(ms(365 * D), "1y 0mo");
assert.equal(ms(400 * D), "1y 1mo");

// Never negative, never NaN, never an empty unit.
for (const n of [null, undefined, NaN, -5000, "x"]) {
  const out = ms(n);
  assert.ok(/^\d+(\.\d+)?(ms|s|m |h |d |mo|y )/.test(out), `clean: ${n} -> ${out}`);
  assert.ok(!/null|NaN|undefined/.test(out), `no placeholders: ${n} -> ${out}`);
}

// The session summary in the right rail: sums of the same ledger rows, and money
// that says "at least" instead of inventing a total when a turn was unpriced.
const tStart = src.indexOf("  function sessionTotals(rows) {");
const tEnd = src.indexOf("\n  }", tStart);
assert.ok(tStart > 0 && tEnd > tStart, "sessionTotals not found in App.svelte");
const sessionTotals = new Function(src.slice(tStart, tEnd + 4) + "\n  return sessionTotals;")();

const empty = sessionTotals([]);
assert.deepEqual(empty, { turns: 0, input: 0, output: 0, tools: 0, sources: 0, usd: 0, partial: false });
// `tools`/`sources` are running counts per row, not per-turn deltas: the total is
// the last one, not the sum (summing them counted turn 1 once per turn after it —
// five turns of three calls read as 120 steps).
const two = sessionTotals([
  { input: 100, output: 20, tools: 3, sources: 2, usd: 0.001 },
  { input: 50, output: 10, tools: 5, sources: 4, usd: 0.002 },
]);
assert.deepEqual(two, { turns: 2, input: 150, output: 30, tools: 5, sources: 4, usd: 0.003, partial: false });
assert.equal(sessionTotals([{ tools: 9 }, { tools: 2 }]).tools, 9, "running count: max, not sum");
// A row written before `usd` existed still carries its formatted cost, which is
// ours and parses back; only a genuinely unpriced row marks the total partial.
const legacy = sessionTotals([{ input: 10, cost: "$0.00019" }, { input: 10, usd: 0.001 }]);
assert.ok(Math.abs(legacy.usd - 0.00119) < 1e-9, "legacy string costs still sum");
assert.equal(legacy.partial, false, "a parseable string is not partial");
const mixed = sessionTotals([{ input: 10, usd: 0.001 }, { input: 10, cost: "n/a" }]);
assert.equal(mixed.usd, 0.001, "unpriced rows add nothing");
assert.equal(mixed.partial, true, "but they mark the total partial");
assert.deepEqual(sessionTotals(null), empty, "a missing ledger is an empty session");
assert.equal(sessionTotals([{ input: "40", output: "2" }]).input, 40, "string numbers still count");

console.log("numbers: 34 checks passed");
