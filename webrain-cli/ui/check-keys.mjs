// The composer's settable keys, checked without a browser. `keyName` is the one
// piece of real logic: the keydown handler compares against a canonical string,
// so a modifier order that differs from what the setting stored would silently
// stop matching — the shortcut would look set and never fire.
//
//   node check-keys.mjs        (from webrain-cli/ui)
import { readFileSync } from "node:fs";
import { strict as assert } from "node:assert";

const src = readFileSync(new URL("./src/App.svelte", import.meta.url), "utf8");
const start = src.indexOf("  function keyName(e) {");
const end = src.indexOf("\n  }", start);
assert.ok(start > 0 && end > start, "keyName not found in App.svelte");
const keyName = new Function(src.slice(start, end + 4) + "\n  return keyName;")();

const ev = (key, mods = {}) => ({
  key,
  ctrlKey: !!mods.ctrl,
  metaKey: !!mods.meta,
  altKey: !!mods.alt,
  shiftKey: !!mods.shift,
});

// The three defaults the Settings tab offers, typed the way a keyboard reports
// them and compared the way the handler compares them.
assert.equal(keyName(ev("Tab")), "Tab");
assert.equal(keyName(ev("Enter", { ctrl: true })), "Ctrl+Enter");
assert.equal(keyName(ev("Enter", { alt: true })), "Alt+Enter");
// Order is fixed regardless of the order the user pressed them in.
assert.equal(keyName(ev("K", { ctrl: true, shift: true })), "Ctrl+Shift+K");
assert.equal(keyName(ev("K", { shift: true, ctrl: true })), "Ctrl+Shift+K");
// A bare Enter is not Ctrl+Enter: that difference is what keeps send working.
assert.notEqual(keyName(ev("Enter")), keyName(ev("Enter", { ctrl: true })));
// Space has a name, and a one-letter key is upper-cased so the stored value is
// the same letter on every keyboard layout that reports it.
assert.equal(keyName(ev(" ")), "Space");
assert.equal(keyName(ev("k")), "K");
assert.equal(keyName(ev("k", { alt: true, ctrl: true, shift: true, meta: true })), "Ctrl+Meta+Alt+Shift+K");

// Every action in the table is read by the composer, or it is a button that
// stores a key nothing listens for.
for (const id of ["complete", "stopSend", "queue"]) {
  assert.ok(src.includes(`keys.${id}`), `keys.${id} is never compared`);
}
// Tab completes; Enter runs. The Enter guard comes first and the completion key
// is what falls through to the field, so a Tab can never execute a command.
const enterGuard = src.indexOf('if (e.key === "Enter") {');
const tabGuard = src.indexOf("if (keyName(e) === keys.complete) {");
const pickCall = src.indexOf("return pick(menuRows[palette.index]);");
const fillCall = src.indexOf("return fillIn(menuRows[palette.index]);");
assert.ok(enterGuard > 0 && pickCall > enterGuard, "the palette has no Enter→pick path");
assert.ok(tabGuard > enterGuard, "the completion key is not checked after Enter");
assert.ok(fillCall > tabGuard, "Tab does not fill the name");
assert.ok(src.includes('localStorage.setItem("webrain-chat-keys"'), "the keys are not persisted");

console.log("keys: 12 assertions passed");
