/* webrain landing — anime.js motion layer (v4).
   Loaded after the anime.js CDN script.
   Content is visible by default (SEO/AI-safe); hidden states are ONLY ever
   set by THIS script, right before it animates, and only when anime is
   present and the user has NOT requested reduced motion. If JS or anime
   fail, every section stays fully visible.
   Motion is motivated: hero = storytelling entrance, sections = hierarchy
   reveals, playground = the scraper-LLM demo. GPU-safe (transform/opacity). */
(function () {
  'use strict';

  var REDUCE = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  var HAS = !!window.anime;

  // Tiny API for inline React handlers in the landing page (install tabs + playground).
  window.__webrain = window.__webrain || {};
  window.__webrain.reduce = REDUCE;

  if (!HAS) return; // nothing animates; the page stays fully visible

  /* ---------------- hero entrance (timeline) ---------------- */
  if (!REDUCE) {
    (function hero() {
      // Gate: the full hide-and-enter sequence runs EXACTLY once. Mintlify's
      // React shell re-renders the custom page after hydration and wipes the
      // DOM-injected word spans; re-running play() on every pass re-hides the
      // whole hero and re-animates it, which reads as a page reload / flash on
      // first load. Later passes only re-wrap the words and restore the
      // visible state (no hide -> no flash).
      var played = false;

      function play() {
        var h1 = document.querySelector('.landing .hero h1');
        if (!h1 || played) return;
        played = true;

        // Word-split the headline (spaces kept as .ws, <em> preserved).
        if (!h1.querySelector('.w')) wrapWords(h1);

        // Set the initial hidden state here (only with anime present).
        anime.set([
          '.landing .hero .eyebrow',
          '.landing .hero h1 .w',
          '.landing .hero .lede',
          '.landing .hero .cta-row',
          '.landing .hero .install-block',
          '.landing .hero-visual'
        ], { opacity: 0 });

        var tl = anime.timeline({ easing: 'easeOutExpo' });
        tl.add({ targets: '.landing .hero .eyebrow', opacity: [0, 1], translateY: [16, 0], duration: 480 })
          .add({
            targets: '.landing .hero h1 .w',
            opacity: [0, 1], translateY: [34, 0], rotateX: [-80, 0],
            delay: anime.stagger(40), duration: 760, easing: 'easeOutCubic'
          }, '-=260')
          .add({ targets: '.landing .hero .lede', opacity: [0, 1], translateY: [18, 0], duration: 540 }, '-=420')
          .add({ targets: '.landing .hero .cta-row', opacity: [0, 1], translateY: [14, 0], duration: 470 }, '-=330')
          .add({ targets: '.landing .hero .install-block', opacity: [0, 1], translateY: [20, 0], duration: 640 }, '-=340')
          .add({ targets: '.landing .hero-visual', opacity: [0, 1], translateY: [26, 0], duration: 820 }, '-=700');
      }

      // Restore the hero to its fully visible state without animating (used
      // after a re-render wiped the entrance). transform/opacity only.
      function restoreHero() {
        anime.set([
          '.landing .hero .eyebrow',
          '.landing .hero h1 .w',
          '.landing .hero .lede',
          '.landing .hero .cta-row',
          '.landing .hero .install-block',
          '.landing .hero-visual'
        ], { opacity: 1, translateY: 0, rotateX: 0 });
      }

      play();

      // Mintlify's React shell can re-render the custom page shortly after
      // hydration and wipe DOM-injected word spans. If the entrance already
      // ran, silently re-wrap + restore visible state (no re-animation, no
      // flash); if it never ran (h1 missing at boot), run the real entrance.
      var reapplied = 0;
      var guard = setInterval(function () {
        var h1 = document.querySelector('.landing .hero h1');
        if (!h1) { clearInterval(guard); return; }
        // Steady state (entrance played + words present) — nothing left to fix.
        if (played && h1.querySelectorAll('.w').length > 0) { clearInterval(guard); return; }
        if (reapplied >= 3) { clearInterval(guard); return; }
        if (h1.querySelectorAll('.w').length === 0 && h1.textContent.indexOf('Browser') !== -1) {
          reapplied++;
          if (!played) { play(); return; }
          wrapWords(h1);
          restoreHero();
        }
      }, 1300);

      // After the install block is on screen, type its command once.
      setTimeout(function () {
        var code = document.querySelector('.landing .install-block .term-cmd');
        if (code) window.__webrain.typeCmd(code, code.getAttribute('data-cmd'));
      }, 2100);
    })();
  }

  // Enforce the intended install default (macOS) even if React hydration (or a
  // stale dev build) leaves another tab active. Runs a few times: hydration can
  // complete after this script loads, so correct the state once it settles.
  function ensureInstallDefault() {
    var tabs = document.querySelector('.landing .install-tabs');
    if (!tabs) return;
    var mac = tabs.querySelector('.install-os[data-os="macos"]');
    var active = tabs.querySelector('.install-os.active');
    var code = document.querySelector('.landing .install-block .term-cmd');
    var want = code ? code.getAttribute('data-cmd-macos') : null;
    if (active !== mac) {
      tabs.querySelectorAll('.install-os').forEach(function (b) {
        var on = b === mac;
        b.classList.toggle('active', on);
        b.setAttribute('aria-selected', on ? 'true' : 'false');
      });
    }
    if (code && want && code.getAttribute('data-cmd') !== want) {
      code.setAttribute('data-cmd', want);
      code.textContent = want;
    }
  }
  // Bounded poll: correct the default until hydration settles, then stop
  // permanently (so it never fights a user click on another OS later). Stop
  // IMMEDIATELY on any user tab interaction — the poll must not re-select
  // macOS over a user's non-mac choice made before hydration settles.
  var tries = 0;
  var userPicked = false;
  var tabsEl = document.querySelector('.landing .install-tabs');
  if (tabsEl) {
    tabsEl.addEventListener('click', function () { userPicked = true; }, true);
    tabsEl.addEventListener('keydown', function () { userPicked = true; }, true);
  }
  ensureInstallDefault();
  var installer = setInterval(function () {
    if (userPicked) { clearInterval(installer); return; }
    var mac = document.querySelector('.landing .install-os[data-os="macos"]');
    var active = document.querySelector('.landing .install-os.active');
    if (mac && active === mac) { clearInterval(installer); return; }
    ensureInstallDefault();
    if (++tries >= 6) clearInterval(installer);
  }, 1800);

  /* ---------------- scroll reveals (all sections) ---------------- */
  var scrollGroups = [
    { sel: '.landing .logos', anim: 'fade' },
    { sel: '.landing .section-head', anim: 'rise' },
    { sel: '.landing .step', anim: 'cell' },
    { sel: '.landing .shell', anim: 'cell' },
    { sel: '.landing .engine', anim: 'cell' },
    { sel: '.landing .ussay-row', anim: 'slideL' },
    { sel: '.landing .check', anim: 'check' },
    { sel: '.landing .compare', anim: 'slideL' },
    { sel: '.landing .faq-item', anim: 'rise' },
    { sel: '.landing .bench-stats', anim: 'rise' },
    { sel: '.landing .bench-bars', anim: 'rise' },
    { sel: '.landing .benchmark-visual', anim: 'fade' },
    { sel: '.landing .serp-cap', anim: 'cell' },
    { sel: '.landing .serp-term', anim: 'fade' },
    { sel: '.landing .loop', anim: 'fade' },
    { sel: '.landing .steps-flow', anim: 'fade' },
    { sel: '.landing .cta-band', anim: 'rise' }
  ];

  var revealed = new WeakSet();
  var scrollIo = null;

  function animator(type, i) {
    var base = { delay: i * 70, duration: 620, easing: 'easeOutCubic' };
    switch (type) {
      case 'cell':   return Object.assign({ opacity: [0, 1], translateY: [22, 0], scale: [0.965, 1] }, base);
      case 'slideL': return Object.assign({ opacity: [0, 1], translateX: [-26, 0] }, base);
      case 'fade':   return Object.assign({ opacity: [0, 1], translateY: [18, 0] }, base);
      case 'check':  return Object.assign({ opacity: [0, 1], translateY: [18, 0] }, base);
      default:       return Object.assign({ opacity: [0, 1], translateY: [24, 0] }, base);
    }
  }

  function reveal(el, i) {
    if (REDUCE) return;
    var type = el.getAttribute('data-anim') || 'rise';
    anime(Object.assign({ targets: el }, animator(type, i)));
    if (el.classList.contains('shell')) revealCellInner(el, i);
    if (el.classList.contains('loop')) playLoopCircuit(el);
    if (el.classList.contains('steps-flow')) playStepsFlow(el);
    if (el.classList.contains('serp-term')) playSerpTerm(el);
    if (el.querySelector('.route-map')) playRouteMap(el);
    if (type === 'check') {
      var p = el.querySelector('.tick svg path');
      if (p) {
        var len = p.getTotalLength();
        p.style.strokeDasharray = len;
        p.style.strokeDashoffset = len;
        anime({ targets: p, strokeDashoffset: [len, 0], duration: 480, delay: i * 70 + 220, easing: 'easeOutCubic' });
      }
    }
    if (el.querySelector('.bench-stat, .serp-stat')) runCounters(el);
    if (el.classList.contains('bench-bars')) runBenchBars(el);
  }

  // Layered entrance inside a bento cell: icon pop, then a cascade of the
  // tool/cap chips and the agent-flow segments, then the crawl spiderweb draws.
  function revealCellInner(el, i) {
    var base = i * 70;
    var ico = el.querySelector('.chip-ico');
    if (ico) {
      anime.set(ico, { scale: 0.4, rotate: -90 });
      anime({ targets: ico, scale: [0.4, 1], rotate: [-90, 0], duration: 520, delay: base + 130, easing: 'easeOutBack' });
    }
    var chips = el.querySelectorAll('.tool-chip, .am-seg');
    if (chips.length) {
      anime.set(chips, { opacity: 0, translateY: 8 });
      anime({
        targets: chips, opacity: [0, 1], translateY: [8, 0], duration: 420,
        delay: anime.stagger(45, { start: base + 230 }), easing: 'easeOutCubic'
      });
    }
    var web = el.querySelectorAll('.crawl-ring, .crawl-ray');
    if (web.length) {
      Array.prototype.forEach.call(web, function (p) {
        var len = p.getTotalLength();
        p.style.strokeDasharray = len;
        p.style.strokeDashoffset = len;
        anime({ targets: p, strokeDashoffset: [len, 0], duration: 800, delay: base + 260, easing: 'easeOutCubic' });
      });
    }
  }

  // Draw an SVG path from a hidden state: dasharray = full length, then ease
  // the dashoffset to 0 so it appears to draw itself.
  function drawPath(p, duration, delay) {
    if (!p) return;
    var len = p.getTotalLength();
    p.style.strokeDasharray = len;
    p.style.strokeDashoffset = len;
    anime({ targets: p, strokeDashoffset: [len, 0], duration: duration || 700, delay: delay || 0, easing: 'easeInOutCubic' });
  }

  /* ---------------- the agent loop (living SVG circuit) ---------------- */
  var loopRunner = null; // travelling-particle animation, rebuilt on React re-render
  function playLoopCircuit(shell) {
    if (REDUCE || !shell) return;
    var svg = shell.querySelector('.loop-circuit');
    if (!svg || !window.anime) return;
    var arcs = Array.prototype.slice.call(svg.querySelectorAll('.loop-arc'));
    arcs.forEach(function (p, i) { drawPath(p, 640, i * 170); });
    drawPath(svg.querySelector('.loop-inner'), 900, 320);
    Array.prototype.slice.call(svg.querySelectorAll('.loop-hub-ring')).forEach(function (p, i) {
      drawPath(p, 700, 520 + i * 220);
    });
    // once the ring is drawn, start the travelling particles
    setTimeout(function () {
      if (REDUCE) return;
      startLoopParticles(shell);
    }, arcs.length * 170 + 720);
  }
  function startLoopParticles(shell) {
    var svg = shell.querySelector('.loop-circuit');
    if (!svg) return;
    var dot = svg.querySelector('.loop-particle');
    var dot2 = svg.querySelector('.loop-particle-2');
    var track = svg.querySelector('.loop-track');
    var track2 = svg.querySelectorAll('.loop-track')[1];
    var nodes = Array.prototype.slice.call(shell.querySelectorAll('.loop-node'));
    if (loopRunner) { loopRunner.pause(); loopRunner = null; }
    nodes.forEach(function (n) { n.classList.remove('lit'); });
    if (track && dot) {
      var path = anime.path(track);
      var litIndex = -1;
      loopRunner = anime({
        targets: dot,
        translateX: path('x'), translateY: path('y'), rotate: path('angle'),
        duration: 5400, easing: 'linear', loop: true,
        update: function (a) {
          if (!nodes.length) return;
          var seg = Math.floor((a.progress / 100) * 5);   // which outer arc (0..4)
          var target = (seg + 1) % nodes.length;          // light the arriving node
          if (target !== litIndex) {
            if (litIndex >= 0 && nodes[litIndex]) nodes[litIndex].classList.remove('lit');
            if (nodes[target]) nodes[target].classList.add('lit');
            litIndex = target;
          }
        }
      });
    }
    if (track2 && dot2) {
      var path2 = anime.path(track2);
      anime({ targets: dot2, translateX: path2('x'), translateY: path2('y'), duration: 3800, easing: 'linear', loop: true });
    }
  }

  // The setup-steps connector: fade the dashed line in, then run one packet
  // across Install -> Connect -> Ask.
  function playStepsFlow(el) {
    if (REDUCE || !el) return;
    var line = el.querySelector('.steps-line');
    var packet = el.querySelector('.steps-packet');
    anime({ targets: el, opacity: [0, 1], duration: 500 });
    if (line && packet) {
      var path = anime.path(line);
      anime({
        targets: packet,
        translateX: path('x'), translateY: path('y'),
        duration: 1900, delay: 700, easing: 'easeInOutCubic',
        complete: function () { anime({ targets: packet, opacity: 0, duration: 300 }); }
      });
    }
  }

  /* ---------------- the SERP terminal (structured search showcase) ----------------
     Runs once per page load (serpPlayed guard, like the hero's `played`): hides
     the animated internals, then streams command lines, cascades typed result
     rows, pops the JSON card, and counts up the stat chips. Reduced-motion / no
     anime / post-render passes leave the static markup fully visible. */
  var serpPlayed = false;
  function playSerpTerm(shell) {
    if (REDUCE || !shell || !window.anime || serpPlayed) return;
    serpPlayed = true;
    var body = shell.querySelector('.serp-term-body') || shell;
    var lines = body.querySelectorAll('.serp-lines .term-line');
    var rows = body.querySelectorAll('.serp-row');
    var json = body.querySelector('.serp-json');
    var stats = body.querySelector('.serp-stats');
    var animated = [].slice.call(lines).concat(Array.prototype.slice.call(rows));
    if (json) animated.push(json);
    if (stats) animated.push(stats);
    anime.set(animated, { opacity: 0, translateY: 8 });
    // stream the command lines, then cascade the results, then JSON + counters
    anime({
      targets: lines, opacity: [0, 1], translateY: [6, 0], duration: 380,
      delay: anime.stagger(240), easing: 'easeOutCubic',
      complete: function () {
        anime({
          targets: rows, opacity: [0, 1], translateY: [12, 0], duration: 420,
          delay: anime.stagger(110), easing: 'easeOutCubic',
          complete: function () {
            if (json) {
              anime({
                targets: json, opacity: [0, 1], translateY: [10, 0], scale: [0.985, 1],
                duration: 480, easing: 'easeOutCubic',
                complete: function () {
                  if (stats) {
                    anime({ targets: stats, opacity: [0, 1], translateY: [10, 0], duration: 480, easing: 'easeOutCubic' });
                    runCounters(stats);
                  }
                }
              });
            } else if (stats) {
              anime({ targets: stats, opacity: [0, 1], translateY: [10, 0], duration: 480, easing: 'easeOutCubic' });
              runCounters(stats);
            }
          }
        });
      }
    });
  }

  // Mintlify's React shell can rebuild the landing subtree after hydration
  // (wiping DOM-injected tags). This is idempotent and re-runnable: it re-tags
  // current nodes, pre-hides only ones not yet revealed, and re-observes. The
  // scheduled re-runs cover the post-hydration rebuild; reveals stay additive.
  function setupScrollReveals() {
    if (REDUCE) return;
    var targets = [];
    scrollGroups.forEach(function (g) {
      var nodes = document.querySelectorAll(g.sel);
      for (var i = 0; i < nodes.length; i++) {
        nodes[i].setAttribute('data-anim', g.anim);
        nodes[i].setAttribute('data-i', i); // per-group index: stagger within a section, not across the page
        targets.push(nodes[i]);
      }
    });
    // Fresh = not yet revealed. A node already inside the viewport at this
    // pass was revealed before a Mintlify re-render replaced the DOM node, so
    // never re-hide it (that re-hide is the first-load flash); reveal it
    // immediately instead.
    var vh = window.innerHeight || document.documentElement.clientHeight;
    var fresh = [];
    targets.forEach(function (el) {
      if (revealed.has(el)) return;
      var r = el.getBoundingClientRect();
      if (r.top < vh && r.bottom > 0) {
        revealed.add(el);
        reveal(el, Number(el.getAttribute('data-i') || 0));
      } else {
        fresh.push(el);
      }
    });
    if (fresh.length) anime.set(fresh, { opacity: 0 });
    if (scrollIo) scrollIo.disconnect();
    scrollIo = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (!en.isIntersecting) return;
        // IO fires an initial callback for every target right after observe()
        // — an in-viewport element already revealed by the immediate branch
        // above would flash (opacity 0→1) a second time without this guard.
        if (revealed.has(en.target)) { scrollIo.unobserve(en.target); return; }
        revealed.add(en.target);
        reveal(en.target, Number(en.target.getAttribute('data-i') || 0));
        scrollIo.unobserve(en.target);
      });
    }, { threshold: 0.12 });
    targets.forEach(function (el) { scrollIo.observe(el); });
  }

  if (!('IntersectionObserver' in window)) {
    // No IO support: leave everything visible. (The old fallback called reveal()
    // on already-visible targets, animating opacity 0→1 and flashing the whole
    // page — and without the pre-hide step it gained nothing.)
  } else {
    setupScrollReveals();
    [800, 2000, 4000].forEach(function (ms) { setTimeout(setupScrollReveals, ms); });
  }

  // Cosmic background, self-healing the same way (idempotent per canvas node).
  setupGalaxy();
  [800, 2000, 4000].forEach(function (ms) { setTimeout(setupGalaxy, ms); });

  // Hero neural web + TOOLS rails + CTA stream, self-healing after React
  // re-renders (idempotent per node, same pattern as the galaxy).
  playHeroWeb();
  setupToolsSides();
  setupCtaStream();
  [1200, 2600, 4800].forEach(function (ms) {
    setTimeout(function () {
      playHeroWeb();
      setupToolsSides();
      setupCtaStream();
    }, ms);
  });

  /* ---------------- backdrop glow parallax ----------------
     Ported from the marketing page: the neon wash drifts a few px with the
     pointer. rAF + lerp on transform only (no layout paint), disabled for
     reduced motion and coarse pointers. `.mesh` is oversized by 80px so the
     travel never exposes an edge. */
  (function parallaxGlow() {
    if (REDUCE) return;
    if (!window.matchMedia || !window.matchMedia('(pointer: fine)').matches) return;
    var target = { x: 0, y: 0 }, cur = { x: 0, y: 0 }, has = false;
    var move = function (e) {
      target.x = (e.clientX / window.innerWidth - 0.5) * 2;
      target.y = (e.clientY / window.innerHeight - 0.5) * 2;
      has = true;
    };
    window.addEventListener('mousemove', move, { passive: true });
    (function frame() {
      var g = document.querySelector('.landing .mesh-glow');
      if (!g) {
        // Landing view is gone (SPA navigation) — stop the rAF loop AND the
        // listener so we don't burn frames/listeners for the whole session.
        window.removeEventListener('mousemove', move);
        return;
      }
      if (has) {
        cur.x += (target.x - cur.x) * 0.08;
        cur.y += (target.y - cur.y) * 0.08;
        g.style.transform = 'translate3d(' + (cur.x * 18).toFixed(1) + 'px,' + (cur.y * 18).toFixed(1) + 'px,0)';
      }
      requestAnimationFrame(frame);
    })();
  })();

  /* ---------------- benchmark bars (proven-on-real-jobs) ----------------
     Marketing-page port: fills grow from 0 to data-w% and the leading number
     counts up. Triggered from the shared scroll reveal (`.bench-bars` row). */
  function runBenchBars(scope) {
    if (REDUCE || !window.anime) return;
    var root = scope || document;
    Array.prototype.forEach.call(root.querySelectorAll('.bench-bars .bar-fill'), function (f) {
      var w = f.getAttribute('data-w');
      anime({ targets: f, width: [0, w + '%'], duration: 1400, easing: 'easeOutExpo', delay: 140 });
    });
    Array.prototype.forEach.call(root.querySelectorAll('.bench-bars .bar-val b[data-tv]'), function (el) {
      var target = parseInt(el.getAttribute('data-tv'), 10) || 0;
      var st = { v: 0 };
      anime({
        targets: st, v: target, duration: 1500, easing: 'easeOutExpo',
        update: function () { el.textContent = st.v.toLocaleString('en-US'); }
      });
    });
  }

  /* ---------------- live CLI session terminal (hero) ----------------
     Typewriter port from the marketing page. Reads the static `.term-line`
     transcript in the markup, re-types each command at 34ms/char with a
     fading caret, then fades in the output line. One-shot (sessionPlayed
     guard) so React re-renders never replay/duplicate; reduced-motion /
     no anime / no IntersectionObserver leave the static transcript visible. */
  var sessionPlayed = false;
  function playSessionTerm(shell) {
    if (REDUCE || !shell || !window.anime || sessionPlayed) return;
    sessionPlayed = true;
    var body = shell.querySelector('.session-body');
    if (!body) return;
    var src = Array.prototype.slice.call(body.querySelectorAll('.term-line'));
    body.innerHTML = '';
    var i = 0;

    function addCaret() {
      var el = document.createElement('div');
      el.className = 'term-line';
      el.innerHTML = '<span class="term-prompt">❯</span><span class="caret"></span>';
      body.appendChild(el);
    }

    function step() {
      if (i >= src.length) { addCaret(); return; }
      var s = src[i];
      var kind = s.getAttribute('data-kind') || 'cmd';
      var el = document.createElement('div');
      el.className = 'term-line';
      if (kind === 'out') {
        el.innerHTML = '<span class="term-prompt" style="color:var(--faint)">→</span><span class="term-out"></span>';
        var out = s.querySelector('.term-out');
        el.querySelector('.term-out').innerHTML = out ? out.innerHTML : '';
        body.appendChild(el);
        anime({
          targets: el, opacity: [0, 1], translateY: [6, 0], duration: 240, delay: 140, easing: 'easeOutQuad',
          complete: function () { i++; setTimeout(step, 220); }
        });
      } else {
        el.innerHTML = '<span class="term-prompt">❯</span><code class="session-cmd"></code><span class="caret"></span>';
        var p = s.querySelector('.term-prompt');
        el.querySelector('.term-prompt').textContent = p ? p.textContent : '❯';
        body.appendChild(el);
        var code = s.querySelector('code');
        var cmd = code ? (code.getAttribute('data-cmd') || code.textContent) : '';
        var caret = el.querySelector('.caret');
        var cmdEl = el.querySelector('.session-cmd');
        var st = { len: 0 };
        anime({ targets: el, opacity: [0, 1], translateY: [8, 0], duration: 250, easing: 'easeOutQuad' });
        var int = setInterval(function () {
          if (st.len <= cmd.length) { cmdEl.textContent = cmd.slice(0, st.len); st.len++; }
          else { clearInterval(int); if (caret) caret.remove(); i++; setTimeout(step, 200); }
        }, 34);
      }
    }

    step();
  }

  function initSessionTerm() {
    var shell = document.querySelector('.landing .session-term');
    if (!shell || sessionPlayed) return;
    if (HAS && !REDUCE && 'IntersectionObserver' in window) {
      var io = new IntersectionObserver(function (entries) {
        entries.forEach(function (en) {
          if (en.isIntersecting) { io.disconnect(); setTimeout(function () { playSessionTerm(shell); }, 400); }
        });
      }, { threshold: 0.25 });
      io.observe(shell);
    }
  }
  initSessionTerm();
  // Re-attach if a Mintlify re-render replaces the hero subtree before first play.
  [1200, 3000].forEach(function (ms) {
    setTimeout(function () { if (!sessionPlayed) initSessionTerm(); }, ms);
  });

  /* ---------------- cosmic background (stars.js galaxy, re-tuned) ----------------
     A light starfield behind the whole page, built from DOM spans so the
     Mintlify sanitizer keeps it (a <canvas> gets stripped, plain <i>/<span>
     don't). Twinkling white + electric-cyan stars, three slow orbit "node"
     dots, and a rare shooting star. All motion is CSS (opacity/rotate) gated
     behind prefers-reduced-motion: no-preference, so there is zero per-frame
     JS. Never runs on phones (the mesh carries those); self-healing like the
     rest of the layer (idempotent per container, re-runs after React rebuilds). */
  function setupGalaxy() {
    var g = document.querySelector('.landing .galaxy');
    if (!g || !HAS) return;
    if (g.__galaxy) return;                // already built on this node
    if (window.matchMedia && !window.matchMedia('(min-width: 768px)').matches) {
      g.style.display = 'none';
      return;
    }
    g.__galaxy = true;
    var frag = document.createDocumentFragment();
    var N = REDUCE ? 40 : 120;
    var i, s, size;
    for (i = 0; i < N; i++) {
      s = document.createElement('i');
      s.className = 'g-star' + (Math.random() < 0.16 ? ' g-cyan' : '');
      size = 1 + Math.random() * 1.4;
      s.style.left = (Math.random() * 100).toFixed(2) + '%';
      s.style.top = (Math.random() * 100).toFixed(2) + '%';
      s.style.width = size + 'px';
      s.style.height = size + 'px';
      s.style.setProperty('--d', (2 + Math.random() * 3).toFixed(1) + 's');
      s.style.setProperty('--p', (Math.random() * 6).toFixed(1) + 's');
      frag.appendChild(s);
    }
    if (!REDUCE) {
      var orbits = [
        { r: 150, dur: 30, size: 3.4, c: 'rgba(127,231,255,0.85)' },
        { r: 240, dur: 44, size: 2.4, c: 'rgba(56,240,255,0.6)' },
        { r: 330, dur: 60, size: 1.8, c: 'rgba(127,231,255,0.45)' }
      ];
      for (var k = 0; k < orbits.length; k++) {
        var o = orbits[k];
        var orbit = document.createElement('span');
        orbit.className = 'g-orbit';
        orbit.style.setProperty('--dur', o.dur + 's');
        orbit.style.animationDelay = (-Math.random() * o.dur).toFixed(1) + 's';
        var node = document.createElement('i');
        node.className = 'g-node';
        node.style.width = o.size + 'px';
        node.style.height = o.size + 'px';
        node.style.background = o.c;
        node.style.top = (-o.r) + 'px';
        orbit.appendChild(node);
        frag.appendChild(orbit);
      }
    }
    g.appendChild(frag);
  }

  /* ---------------- hero neural web (v4.3) ----------------
     Draw the spider-network rings/spokes on reveal, then run two glowing
     packets around the rings via anime.path (same pattern as the agent-loop
     circuit). Self-healing/idempotent per node; reduced-motion / no-anime keep
     the web as a static hairline constellation (never hidden). */
  var heroWebRun = null;
  function playHeroWeb() {
    if (REDUCE || !window.anime) return;
    var web = document.querySelector('.landing .hero-web');
    if (!web) return;
    var svg = web.querySelector('svg');
    if (!svg || svg.__web) return;
    svg.__web = true;
    var rings = Array.prototype.slice.call(svg.querySelectorAll('.hw-ring, .hw-spoke'));
    rings.forEach(function (p, i) { drawPath(p, 700, i * 90); });
    var track = svg.querySelector('.hw-track');
    var track2 = svg.querySelector('.hw-track-2');
    var dot = svg.querySelector('.hw-packet');
    var dot2 = svg.querySelector('.hw-packet-2');
    if (track && dot) {
      var path = anime.path(track);
      if (heroWebRun) heroWebRun.pause();
      heroWebRun = anime({
        targets: dot, translateX: path('x'), translateY: path('y'),
        duration: 8600, easing: 'linear', loop: true, delay: 1500
      });
    }
    if (track2 && dot2) {
      var path2 = anime.path(track2);
      anime({
        targets: dot2, translateX: path2('x'), translateY: path2('y'),
        duration: 12400, easing: 'linear', loop: true, delay: 2400
      });
    }
  }

  /* ---------------- TOOLS sidebar (v4.3) ----------------
     Inject the brand-identity TOOLS rail into each hero workflow terminal,
     lighting the tools that card's transcript uses (from data-tools). Built
     as static DOM so no-JS / reduced-motion still get the rail; idempotent. */
  function setupToolsSides() {
    var cards = document.querySelectorAll('.landing .wf-card3d');
    for (var i = 0; i < cards.length; i++) {
      var card = cards[i];
      var term = card.querySelector('.workflow-term');
      if (!term || term.__tools) continue;
      term.__tools = true;
      var on = (card.getAttribute('data-tools') || '').split(/\s+/).filter(Boolean);
      var rail = document.createElement('div');
      rail.className = 'tools-side';
      rail.setAttribute('aria-hidden', 'true');
      var head = document.createElement('span');
      head.className = 'ts-head';
      head.textContent = 'TOOLS';
      rail.appendChild(head);
      ['navigate', 'observe', 'interact', 'extract', 'crawl', 'watch', 'session', 'vision'].forEach(function (t) {
        var s = document.createElement('span');
        s.className = 'ts-tool' + (on.indexOf(t) !== -1 ? ' on' : '');
        s.textContent = t;
        rail.appendChild(s);
      });
      term.appendChild(rail);
      term.classList.add('has-tools');
    }
  }

  /* ---------------- CTA particle stream (v4.3) ----------------
     A converging stream of glowing dots rising into the CTA band (pure CSS
     keyframes on transform/opacity, sanitizer-safe <i> spans). Idempotent;
     reduced-motion leaves the band clean. */
  function setupCtaStream() {
    var band = document.querySelector('.landing .cta-band');
    if (!band) return;
    var stream = band.querySelector('.cta-stream');
    if (!stream || stream.__stream) return;
    stream.__stream = true;
    if (REDUCE) return;
    var n = window.innerWidth < 768 ? 10 : 22;
    var frag = document.createDocumentFragment();
    for (var i = 0; i < n; i++) {
      var p = document.createElement('i');
      p.style.left = (Math.random() * 100).toFixed(1) + '%';
      p.style.setProperty('--d', (5 + Math.random() * 6).toFixed(1) + 's');
      p.style.setProperty('--p', (Math.random() * 8).toFixed(1) + 's');
      p.style.setProperty('--dx', ((Math.random() - 0.5) * 160).toFixed(1) + 'px');
      frag.appendChild(p);
    }
    stream.appendChild(frag);
  }

  /* ---------------- playbook route map (v4.3) ----------------
     Draw the prompt -> decide -> browser/extractor route lines, then run one
     packet down each path in sequence (storytelling: how the skill routes). */
  function playRouteMap(shell) {
    if (REDUCE || !shell || !window.anime) return;
    var map = shell.querySelector('.route-map');
    if (!map) return;
    var lines = map.querySelectorAll('.route-line');
    var packets = map.querySelectorAll('.route-packet');
    Array.prototype.forEach.call(lines, function (p, i) { drawPath(p, 600, i * 160); });
    Array.prototype.forEach.call(packets, function (p, i) {
      var line = lines[i] || lines[lines.length - 1];
      var path = anime.path(line);
      anime({
        targets: p, translateX: path('x'), translateY: path('y'),
        duration: 900, delay: 520 + i * 420, easing: 'easeInOutCubic',
        complete: function () { anime({ targets: p, opacity: 0, duration: 300 }); }
      });
    });
  }

  /* ---------------- helpers ---------------- */

  // Wrap a heading's words in inline-block spans so they can stagger-rise
  // without touching the text (SEO sees the original text). Whitespace is
  // emitted as plain text nodes BETWEEN the spans so the spacing renders.
  function wrapWords(el) {
    var frag = document.createDocumentFragment();
    function walk(node) {
      Array.prototype.slice.call(node.childNodes).forEach(function (cn) {
        if (cn.nodeType === 3) {
          cn.textContent.split(/(\s+)/).forEach(function (p) {
            if (p === '') return;
            if (/^\s+$/.test(p)) { frag.appendChild(document.createTextNode(p)); return; }
            var s = document.createElement('span');
            s.className = 'w';
            s.textContent = p;
            frag.appendChild(s);
          });
        } else if (cn.nodeType === 1 && cn.tagName === 'EM') {
          var em = cn.cloneNode(false);
          var inner = document.createDocumentFragment();
          Array.prototype.slice.call(cn.childNodes).forEach(function (c2) {
            if (c2.nodeType === 3) {
              c2.textContent.split(/(\s+)/).forEach(function (p) {
                if (p === '') return;
                if (/^\s+$/.test(p)) { inner.appendChild(document.createTextNode(p)); return; }
                var s = document.createElement('span');
                s.className = 'w';
                s.textContent = p;
                inner.appendChild(s);
              });
            } else inner.appendChild(c2.cloneNode(true));
          });
          em.appendChild(inner);
          frag.appendChild(em);
        } else frag.appendChild(cn.cloneNode(true));
      });
    }
    walk(el);
    el.replaceChildren(frag);
  }

  // Count up a .bench-stat / .serp-stat number (e.g. "42 products in 1.4s").
  function runCounters(scope) {
    Array.prototype.forEach.call(scope.querySelectorAll('.bench-stat b[data-count], .serp-stat b[data-count]'), function (el) {
      var target = parseInt(el.getAttribute('data-count'), 10) || 0;
      var num = el.querySelector('.num') || el;
      var st = { v: 0 };
      anime({
        targets: st, v: target, duration: 1500, easing: 'easeOutExpo', round: 1,
        update: function () { num.textContent = st.v; }
      });
    });
  }

  /* ---------------- the scraper-LLM playground ---------------- */
  var DEMOS = {
    prices: {
      prompt: 'scrape product prices from shop.example.com',
      lines: [
        { k: 'cmd', t: 'webrain_navigate', out: '→ ok · 0 challenges' },
        { k: 'cmd', t: 'webrain_observe · what=state', out: '→ 24 product cards detected' },
        { k: 'cmd', t: 'webrain_extract · mode=autoschema', out: '→ schema: {title, price, url}' },
        { k: 'cmd', t: 'webrain_extract · mode=schema', out: '→ 42 rows' }
      ],
      card: {
        title: 'result.json · 42 items',
        body: '[ { "title": "USB-C Hub", "price": 42.90 }, { "title": "Wireless Charger", "price": 29.99 }, … 40 more ]'
      }
    },
    auth: {
      prompt: 'extract prices from 12 auth pages behind Turnstile',
      lines: [
        { k: 'cmd', t: 'webrain_session · op=login', out: '→ vault AES-256-GCM + TOTP ok' },
        { k: 'cmd', t: 'webrain_navigate', out: '→ challenge: turnstile → cleared · native login' },
        { k: 'cmd', t: 'webrain_batch · op=extract', out: '→ 12 urls · 214 items' }
      ],
      card: {
        title: 'result.json · 214 items',
        body: '{ "pages": 12, "items": 214, "auth": "TOTP + cf_clearance", "ok": true }'
      }
    },
    batch: {
      prompt: 'collect specs from 40 public pages, no auth',
      lines: [
        { k: 'cmd', t: 'webrain_batch · op=interact', out: '→ 40 tabs in parallel' },
        { k: 'cmd', t: '… clicked specs tab · read rows per page', out: '→ 312 rows · 0 logins' },
        { k: 'cmd', t: 'webrain_extract · mode=schema', out: '→ 312 rows' }
      ],
      card: {
        title: 'result.json · 312 rows',
        body: '{ "pages": 40, "rows": 312, "logins": 0, "ok": true }'
      }
    },
    watch: {
      prompt: 'summarize this video, fully offline',
      lines: [
        { k: 'cmd', t: 'webrain_watch · source=https://youtu.be/…', out: '→ bundled ffmpeg + yt-dlp + whisper' },
        { k: 'cmd', t: '… transcript · 47 timestamped segments', out: '→ t=00:00 → 14:32' },
        { k: 'cmd', t: '… 12 frames · vision=local', out: '→ captions + visual summary' }
      ],
      card: {
        title: 'result.json · video',
        body: '{ "transcript": 47, "frames": 12, "vision": "Qwen3-VL-2B", "ok": true }'
      }
    },
    serp: {
      prompt: 'search "tokio rust" on duckduckgo',
      lines: [
        { k: 'cmd', t: 'webrain_serp · engine=duckduckgo · limit=5', out: '→ duckduckgo · plain HTTP · no browser' },
        { k: 'cmd', t: '… deduped', out: '→ 5 unique · 0.9s · request_id serp-…' },
        { k: 'cmd', t: 'webrain_serp · engine=brave', out: '→ rendered in the connected CDP engine' }
      ],
      card: {
        title: 'result.json · 5 items',
        body: '[ { "position": 1, "title": "Tokio", "url": "https://tokio.rs", "domain": "tokio.rs", "snippet": "…" }, … 4 more ]'
      }
    },
    drone: {
      prompt: 'how do I build my own drone: parts, sources, code?',
      lines: [
        { k: 'cmd', t: 'webrain_serp · "build FPV drone guide" · limit=10', out: '→ oscarliang · kretfpv · dronesgator' },
        { k: 'cmd', t: 'webrain_serp · "open source drone firmware github"', out: '→ ArduPilot · Betaflight · INAV · PX4' },
        { k: 'cmd', t: 'webrain_batch · op=fetch · 6 urls · concurrency=6', out: '→ 6 sources · one call' },
        { k: 'cmd', t: '… synthesize across sources', out: '→ 12-part list · $500-650 · build order' }
      ],
      card: {
        title: 'result.md · build guide',
        body: 'parts: frame · 2207 motors · 4-in-1 ESC · F722 FC · VTX · ELRS\nfirmware: Betaflight · INAV · ArduPilot\nwhere: GetFPV · Pyrodrone · AliExpress\nreal run · 1m43s · DeepSeek'
      }
    }
  };

  function currentDemoKey() {
    var act = document.querySelector('.landing .try-preset.active');
    return act ? act.getAttribute('data-demo') : 'prices';
  }

  function buildLine(linesEl, k, t, out) {
    var el = document.createElement('div');
    el.className = 'try-line' + (k === 'ask' ? ' try-ask' : '');
    if (k === 'ask') {
      el.innerHTML = '<span class="term-prompt">❯</span><span class="try-type"></span>';
      linesEl.appendChild(el);
      return el.querySelector('.try-type');
    }
    el.innerHTML = '<span class="term-prompt">❯</span><code></code><span class="try-ok"></span>';
    el.querySelector('code').textContent = t;
    el.querySelector('.try-ok').textContent = out;
    linesEl.appendChild(el);
    return el;
  }

  function streamLines(linesEl, lines, i, gen) {
    if (gen !== playgroundGen) return; // a newer run started — drop this stale chain
    if (i >= lines.length) return;
    var ln = lines[i];
    var el = buildLine(linesEl, ln.k, ln.t, ln.out || '');
    anime({
      targets: el, opacity: [0, 1], translateX: [-10, 0], duration: 420, easing: 'easeOutCubic',
      complete: function () {
        setTimeout(function () { streamLines(linesEl, lines, i + 1, gen); }, ln.gap || 210);
      }
    });
  }

  function showCard(cardEl, card) {
    cardEl.innerHTML = '';
    var head = document.createElement('div');
    head.className = 'try-card-head';
    var idx = card.title.indexOf(' · ');
    head.innerHTML =
      '<span></span><span class="try-card-tag"></span>';
    head.querySelector('span').textContent = idx > -1 ? card.title.slice(0, idx) : card.title;
    head.querySelector('.try-card-tag').textContent = idx > -1 ? card.title.slice(idx + 3) : '';
    var pre = document.createElement('pre');
    pre.textContent = card.body;
    cardEl.appendChild(head);
    cardEl.appendChild(pre);
    anime({
      targets: cardEl, opacity: [0, 1], scale: [0.965, 1], translateY: [8, 0],
      duration: 520, easing: 'easeOutCubic'
    });
  }

  var playgroundGen = 0; // bumps on every run; stale runs bail out
  function runPlayground(key, custom) {
    if (REDUCE || !HAS) return;
    var shell = document.querySelector('.landing .try');
    if (!shell) return;
    var linesEl = shell.querySelector('.try-lines');
    var cardEl = shell.querySelector('.try-card');
    var demo = DEMOS[key] || DEMOS.prices;
    var prompt = (custom && String(custom).trim()) ? String(custom).trim() : demo.prompt;

    linesEl.innerHTML = '';
    anime.set(cardEl, { opacity: 0 });

    // Generation token: a re-run (preset click while the previous run is still
    // streaming) must not let the OLD run's continuations append lines / set
    // the card over the new run's cleared terminal.
    var gen = ++playgroundGen;
    var typeEl = buildLine(linesEl, 'ask', '', '');
    var st = { len: 0 };
    anime({
      targets: st, len: prompt.length,
      duration: Math.min(1300, 30 * prompt.length),
      easing: 'linear',
      update: function () { if (gen === playgroundGen) typeEl.textContent = prompt.slice(0, Math.round(st.len)); }
    }).finished.then(function () {
      if (gen !== playgroundGen) return;
      streamLines(linesEl, demo.lines, 0, gen);
      setTimeout(function () { if (gen === playgroundGen) showCard(cardEl, demo.card); }, demo.lines.length * 480 + 320);
    });
  }

  function initPlayground() {
    var shell = document.querySelector('.landing .try');
    if (!shell) return;
    var linesEl = shell.querySelector('.try-lines');
    var cardEl = shell.querySelector('.try-card');
    if (HAS && !REDUCE) {
      // Replace the static fallback with a ready prompt; the first run streams it.
      linesEl.innerHTML = '<div class="try-line try-idle"><span class="term-prompt">❯</span><span class="try-type">agent ready · pick a demo or type a prompt</span></div>';
      anime.set(cardEl, { opacity: 0 });
    }
    var ran = false;
    function go() {
      if (ran) return;
      ran = true;
      runPlayground(currentDemoKey(), null);
    }
    if ('IntersectionObserver' in window && !REDUCE) {
      var io = new IntersectionObserver(function (entries) {
        entries.forEach(function (en) {
          if (en.isIntersecting) { io.disconnect(); go(); }
        });
      }, { threshold: 0.22 });
      io.observe(shell);
    } else if (HAS && !REDUCE) {
      go();
    }
  }
  initPlayground();

  /* ---------------- public API (React onClick in the landing page) ---------------- */
  window.__webrain.run = function (demo, custom) {
    if (REDUCE || !HAS) return;
    var key = demo || currentDemoKey();
    runPlayground(key, custom || null);
  };
  window.__webrain.typeCmd = function (el, text) {
    if (!el) return;
    var full = text != null ? String(text) : (el.getAttribute('data-cmd') || '');
    if (REDUCE || !HAS) { el.textContent = full; return; }
    var st = { len: 0 };
    el.textContent = '';
    anime({
      targets: st, len: full.length, duration: Math.min(900, 16 * full.length),
      easing: 'linear',
      update: function () { el.textContent = full.slice(0, Math.round(st.len)); }
    });
  };
})();
