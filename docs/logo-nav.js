// logo-nav.js — bump the Mintlify navbar logo from h-7 (28px) to h-10 (40px).
// Loaded via deployment-level customScripts (online admin setting), not docs.json
// (the docs.json schema strips/rejects custom CSS+JS). Runs on every page load.
(function () {
  const s = document.createElement("style");
  s.textContent = ".nav-logo { height: 2.5rem !important; }";
  // document.head may be null if this script runs before <head> is parsed.
  (document.head || document.documentElement).appendChild(s);
})();

// Auto-select the install OS tab on the landing page (first load).
// Re-applies until React hydration settles, stops once the user interacts
// with a tab (click OR keyboard) or the block is populated.
(function () {
  function detectOS() {
    const ua = navigator.userAgent || "";
    if (/windows|win32|win64/i.test(ua)) return "windows";
    if (/linux|crOS|ubuntu|debian/i.test(ua) && !/android/i.test(ua)) return "linux";
    return "macos";
  }
  function apply(os) {
    const blk = document.querySelector(".landing .install-block");
    if (!blk) return;
    const code = blk.querySelector(".term-cmd");
    const tabs = blk.querySelectorAll(".install-os");
    const cmd = code ? code.getAttribute("data-cmd-" + os) : null;
    for (let i = 0; i < tabs.length; i++) {
      const on = tabs[i].getAttribute("data-os") === os;
      tabs[i].classList.toggle("active", on);
      tabs[i].setAttribute("aria-selected", on);
    }
    if (code && cmd) {
      code.setAttribute("data-cmd", cmd);
      code.textContent = cmd;
    }
  }
  const os = detectOS();
  let userClicked = false;
  // Stop on click OR keyboard (ARIA tabs: arrows/Enter) so the poll never
  // fights a non-click user choice.
  const onTab = (e) => {
    if (e.target && e.target.closest && e.target.closest(".install-os")) userClicked = true;
  };
  document.addEventListener("click", onTab, true);
  document.addEventListener("keydown", onTab, true);
  // Settled = the block exists and data-cmd is populated — early-exit the
  // poll then, instead of ~7.5s of DOM churn (re-writing textContent every
  // 150ms can disrupt a user mid-copy).
  const settled = () => {
    const blk = document.querySelector(".landing .install-block");
    const code = blk ? blk.querySelector(".term-cmd") : null;
    return !!(blk && code && code.getAttribute("data-cmd"));
  };
  let n = 0;
  (function loop() {
    if (!userClicked) apply(os);
    if (settled() || userClicked || ++n >= 50) return;
    setTimeout(loop, 150);
  })();
  window.addEventListener("load", function onload() {
    if (!userClicked) apply(os);
    window.removeEventListener("load", onload);
  });
})();

