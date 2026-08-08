// logo-nav.js — bump the Mintlify navbar logo from h-7 (28px) to h-10 (40px).
// Loaded via deployment-level customScripts (online admin setting), not docs.json
// (the docs.json schema strips/rejects custom CSS+JS). Runs on every page load.
(function () {
  var s = document.createElement("style");
  s.textContent = ".nav-logo { height: 2.5rem !important; }";
  document.head.appendChild(s);
})();

// Auto-select the install OS tab on the landing page (first load).
// Re-applies until React hydration settles, stops once the user clicks a tab.
(function () {
  function detectOS() {
    var ua = navigator.userAgent || "";
    if (/windows|win32|win64/i.test(ua)) return "windows";
    if (/linux|crOS|ubuntu|debian/i.test(ua) && !/android/i.test(ua)) return "linux";
    return "macos";
  }
  function apply(os) {
    var blk = document.querySelector(".landing .install-block");
    if (!blk) return;
    var code = blk.querySelector(".term-cmd");
    var tabs = blk.querySelectorAll(".install-os");
    var cmd = code ? code.getAttribute("data-cmd-" + os) : null;
    for (var i = 0; i < tabs.length; i++) {
      var on = tabs[i].getAttribute("data-os") === os;
      tabs[i].classList.toggle("active", on);
      tabs[i].setAttribute("aria-selected", on);
    }
    if (code && cmd) {
      code.setAttribute("data-cmd", cmd);
      code.textContent = cmd;
    }
  }
  var os = detectOS();
  var userClicked = false;
  document.addEventListener("click", function (e) {
    if (e.target && e.target.closest && e.target.closest(".install-os")) userClicked = true;
  }, true);
  var n = 0;
  (function loop() {
    if (!userClicked) apply(os);
    if (++n < 50) setTimeout(loop, 150); // ~7.5s ceiling
  })();
  window.addEventListener("load", function () { if (!userClicked) apply(os); });
})();

