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
      function play() {
        var h1 = document.querySelector('.landing .hero h1');
        if (!h1) return;

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

      play();

      // Mintlify's React shell can re-render the custom page shortly after
      // hydration and wipe DOM-injected word spans. Re-apply (bounded) if so;
      // production never re-renders, so this is just insurance.
      var reapplied = 0;
      var guard = setInterval(function () {
        var h1 = document.querySelector('.landing .hero h1');
        if (!h1 || reapplied >= 2) { clearInterval(guard); return; }
        if (h1.querySelectorAll('.w').length === 0 && h1.textContent.indexOf('Browser') !== -1) {
          reapplied++;
          play();
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
  // permanently (so it never fights a user click on another OS later).
  var tries = 0;
  ensureInstallDefault();
  var installer = setInterval(function () {
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
    { sel: '.landing .benchmark-visual', anim: 'fade' },
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
    var type = el.getAttribute('data-anim') || 'rise';
    anime(Object.assign({ targets: el }, animator(type, i)));
    if (type === 'check') {
      var p = el.querySelector('.tick svg path');
      if (p) {
        var len = p.getTotalLength();
        p.style.strokeDasharray = len;
        p.style.strokeDashoffset = len;
        anime({ targets: p, strokeDashoffset: [len, 0], duration: 480, delay: i * 70 + 220, easing: 'easeOutCubic' });
      }
    }
    if (el.classList.contains('bench-stats')) runCounters(el);
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
    var fresh = targets.filter(function (el) { return !revealed.has(el); });
    if (fresh.length) anime.set(fresh, { opacity: 0 });
    if (scrollIo) scrollIo.disconnect();
    scrollIo = new IntersectionObserver(function (entries) {
      entries.forEach(function (en) {
        if (!en.isIntersecting) return;
        revealed.add(en.target);
        reveal(en.target, Number(en.target.getAttribute('data-i') || 0));
        scrollIo.unobserve(en.target);
      });
    }, { threshold: 0.12 });
    targets.forEach(function (el) { scrollIo.observe(el); });
  }

  if (!('IntersectionObserver' in window)) {
    // No IO support: reveal everything immediately (page stays fully visible).
    var allTargets = [];
    scrollGroups.forEach(function (g) {
      Array.prototype.forEach.call(document.querySelectorAll(g.sel), function (el) {
        allTargets.push(el);
      });
    });
    allTargets.forEach(function (el, i) { reveal(el, i); });
  } else {
    setupScrollReveals();
    [800, 2000, 4000].forEach(function (ms) { setTimeout(setupScrollReveals, ms); });
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

  // Count up a .bench-stat number (e.g. "42 products in 1.4s").
  function runCounters(scope) {
    Array.prototype.forEach.call(scope.querySelectorAll('.bench-stat b[data-count]'), function (el) {
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
        { k: 'cmd', t: 'webrain_navigate', out: '→ challenge: turnstile → solved · chrome sidecar' },
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

  function streamLines(linesEl, lines, i) {
    if (i >= lines.length) return;
    var ln = lines[i];
    var el = buildLine(linesEl, ln.k, ln.t, ln.out || '');
    anime({
      targets: el, opacity: [0, 1], translateX: [-10, 0], duration: 420, easing: 'easeOutCubic',
      complete: function () {
        setTimeout(function () { streamLines(linesEl, lines, i + 1); }, ln.gap || 210);
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

    var typeEl = buildLine(linesEl, 'ask', '', '');
    var st = { len: 0 };
    anime({
      targets: st, len: prompt.length,
      duration: Math.min(1300, 30 * prompt.length),
      easing: 'linear',
      update: function () { typeEl.textContent = prompt.slice(0, Math.round(st.len)); }
    }).finished.then(function () {
      streamLines(linesEl, demo.lines, 0);
      setTimeout(function () { showCard(cardEl, demo.card); }, demo.lines.length * 480 + 320);
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
