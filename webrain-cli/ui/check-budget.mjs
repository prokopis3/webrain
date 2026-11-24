// A size budget for the chat bundle, checked the way the other gates are: an
// assertion with a number, run by `bun run check`.
//
// The page is one 7.9k-line component that ships three assets, and nothing else
// in the repo notices when it grows. Gzip bytes are the number a reader pays for
// on first paint, so they are the number that is gated — a raw-size ceiling would
// be met by renaming variables.
//
//   node check-budget.mjs        (from webrain-cli/ui, after a build)
import { readFileSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { strict as assert } from "node:assert";

// Ceilings are the measured size at the time of writing plus ~5%: enough room for
// a feature, not enough for a second framework.
const BUDGET = [
  // 82 since the composer field became draggable from its top edge (vertical
  // handle, clamped height, a corner expand/shrink button); it was 81 with the
  // history-search dialog and 80 before that with the results in the rail.
  ["../assets/chat.js", 82, "the app bundle"],
  ["../assets/chat.css", 15, "the stylesheet"],
  ["../assets/chunk-markdown.js", 29, "marked + DOMPurify, deferred"],
];

let total = 0;
for (const [file, ceilingKb, what] of BUDGET) {
  const bytes = readFileSync(new URL(file, import.meta.url));
  const gz = gzipSync(bytes, { level: 9 }).length;
  const kb = gz / 1024;
  total += kb;
  assert.ok(
    kb <= ceilingKb,
    `${what} is ${kb.toFixed(1)} KB gzipped, over its ${ceilingKb} KB budget — ` +
      `either trim it or raise the ceiling in check-budget.mjs with a reason`,
  );
  console.log(`budget: ${file.split("/").pop()} ${kb.toFixed(1)} KB gz ≤ ${ceilingKb}`);
}
console.log(`budget: ${total.toFixed(1)} KB total`);
