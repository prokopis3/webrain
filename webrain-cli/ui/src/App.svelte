<script>
  // webrain chat · Svelte 5 runes.
  // Wire contract: GET /events (SSE), POST /turn, POST /permission,
  // GET /sessions (+ /sessions/<id>). Unknown runtime events are shown, never
  // dropped: the SDK's catalogue grows and the page should not have to know it.
  const esc = (s) =>
    String(s).replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" })[c]);

  // Markdown is a dynamic import. `marked` + DOMPurify are the largest things in
  // the bundle and nothing needs them until the first answer arrives, so they
  // live in their own chunk: warmed when the browser is idle, awaited before the
  // first render. Until then a message renders as plain text, which is correct,
  // just unformatted.
  let render = null; // the resolved `render(text)` from the markdown chunk
  let mdReady = $state(false);
  let mdLoading = null;

  function loadMd() {
    mdLoading ??= import("./markdown.js")
      .then((m) => {
        render = m.render;
        mdReady = true;
      })
      .catch(() => {
        mdLoading = null; // a failed chunk load retries on the next render
      });
    return mdLoading;
  }

  const whenIdle = (fn) =>
    typeof requestIdleCallback === "function" ? requestIdleCallback(fn) : setTimeout(fn, 1200);
  whenIdle(loadMd);

  let items = $state([]); // finished transcript
  let streamText = $state(""); // the reply being written, rendered as plain text
  // What the DOM actually renders. `md()` is marked + DOMPurify over the whole
  // accumulated reply, and a fast model emits deltas faster than the screen
  // refreshes — parsing per delta was O(n²) in reply length and the main reason
  // a long answer janked. One parse per frame is indistinguishable to read.
  let streamView = $state("");
  let streamRaf = 0;
  function paintStream() {
    if (streamRaf) return;
    streamRaf = requestAnimationFrame(() => {
      streamRaf = 0;
      streamView = streamText;
      follow();
    });
  }
  function clearStream() {
    if (streamRaf) cancelAnimationFrame(streamRaf);
    streamRaf = 0;
    streamText = "";
    streamView = "";
  }
  let past = $state(null); // history being read, or null for live
  let viewing = $state(""); // which conversation `past` belongs to
  let convos = $state([]);
  let current = $state("");
  let turns = $state([]); // per-turn ledger of the live conversation, newest first
  let pastTurns = $state([]); // the ledger of the conversation being read
  let seededFor = ""; // which conversation the live ledger was restored for
  let busy = $state(false);
  let state = $state("ready"); // ready | running | error | ended
  let asks = $state([]); // permission requests, first one shown
  let asksIn = $state([]); // ask_user questions, first one shown
  let freeAnswer = $state("");
  let call = $state(null); // the model call in flight, from `assistant.usage`
  // Restored, because an accidental reload mid-prompt should not cost the prompt.
  let draft = $state(recall("draft", ""));
  let copiedId = $state(-1);
  let pinned = $state(true);
  let logEl = $state(null);
  let raf = 0;
  let seq = 0;
  const cards = new Map(); // toolCallId -> item index

  // Composer menus: `/` everything this page can reach, `@` the skills the binary
  // ships, `#` the tools it exposes. One flat list from GET /catalog, each row
  // carrying its own kind and hint, so a menu can never offer something this
  // build does not have — and can offer all of it from one place.
  let catalog = $state({ items: [] });
  let palette = $state(null); // { trigger, query, index }
  const SIGIL = { cmd: "/", skill: "@", tool: "#", builtin: "#" };
  const options = $derived.by(() => {
    if (!palette) return [];
    const q = palette.query.toLowerCase();
    const want = palette.trigger === "/" ? null : palette.trigger === "@" ? "skill" : "tool";
    const rows =
      want === null
        ? catalog.items
        : catalog.items.filter((r) => r.kind === want || (want === "tool" && r.kind === "builtin"));
    return rows.filter((r) => r.name.toLowerCase().includes(q));
  });
  // The header counts what matched, the list shows the first screenful of it —
  // a menu that says "14" when 27 matched is a menu that hides the rest.
  const menuRows = $derived(options.slice(0, 14));

  // The session panel is a column on a desktop and a drawer on a phone, so it
  // starts open only where there is room for it.
  let panelOpen = $state(typeof window !== "undefined" && window.innerWidth > 760);
  // How wide the two rails are, in px, dragged by the handles between them. The
  // grid, the composer's own offset and the handles all read these two vars, so
  // there is one number per side rather than three that can disagree.
  let railW = $state(readJson("rail-w", 256));
  let instW = $state(readJson("inst-w", 240));
  const W_MIN = 160;
  const W_MAX = 480; // shortcut: clamped here, a rail dragged to the window edge eats the transcript
  $effect(() => {
    const s = document.documentElement.style;
    s.setProperty("--rail-w", `${railW}px`);
    s.setProperty("--inst-w", `${instW}px`);
  });
  let gripDrag = null;
  function gripDown(which, e) {
    if (e.button !== 0) return;
    e.preventDefault();
    gripDrag = { which, x: e.clientX, start: which === "rail" ? railW : instW };
    addEventListener("pointermove", gripMove);
    addEventListener("pointerup", gripUp, { once: true });
    addEventListener("pointercancel", gripUp, { once: true });
  }
  function gripMove(e) {
    if (!gripDrag) return;
    // The rail grows to the right, the instruments panel grows to the left.
    const dx = e.clientX - gripDrag.x;
    const raw = gripDrag.which === "rail" ? gripDrag.start + dx : gripDrag.start - dx;
    const w = Math.min(W_MAX, Math.max(W_MIN, raw));
    if (gripDrag.which === "rail") railW = w;
    else instW = w;
  }
  function gripUp() {
    removeEventListener("pointermove", gripMove);
    removeEventListener("pointercancel", gripUp);
    gripDrag = null;
  }
  // A separator that only answers to a mouse is not a separator: arrows move it.
  function gripKey(which, e) {
    const d = e.key === "ArrowLeft" ? -16 : e.key === "ArrowRight" ? 16 : 0;
    if (!d) return;
    e.preventDefault();
    const cur = which === "rail" ? railW : instW;
    const w = Math.min(W_MAX, Math.max(W_MIN, cur + d));
    if (which === "rail") railW = w;
    else instW = w;
  }
  let q = $state("");
  let hits = $state([]);
  let searchSeq = 0;
  let searchTimer = 0;
  let attachSeq = 0;
  let fileInput = $state(null);

  // A setting that outlives the tab: read once at start, written by the effect
  // below. Storage is allowed to be unavailable (private mode), so a failure
  // means the default, never an exception on load.
  function recall(key, fallback) {
    try {
      return localStorage.getItem(`webrain-chat-${key}`) || fallback;
    } catch {
      return fallback;
    }
  }

  function readJson(key, fallback) {
    try {
      return JSON.parse(localStorage.getItem(`webrain-chat-${key}`) || "null") ?? fallback;
    } catch {
      return fallback;
    }
  }

  // The "Agent" control and the model button share one bar above the composer.
  let mode = $state(recall("mode", "agent")); // agent | plan | ask
  // Settings that are not composer controls. Both are sent with every turn, so
  // they live here and are read by `send()` — nothing has to restart.
  let instructions = $state(recall("instructions", ""));
  let language = $state(recall("language", ""));
  let suggest = $state(readJson("suggest", true));
  let modelOpen = $state(false);
  let providers = $state([]);
  let provTab = $state(0);
  // The provider row is wider than the dialog once there are a few of them. The
  // scrollbar is hidden, so dragging and the two edge buttons are the ways to
  // reach the rest — and both disappear when there is nothing to reach.
  let tabsEl = $state(null);
  let tabsAt = $state({ left: false, right: false });
  let tabDrag = null; // {x, left, moved} while a pointer is down
  let tabSwallow = false; // a drag ends on a tab: that click is not a pick
  function tabsSync() {
    if (!tabsEl) return;
    const max = tabsEl.scrollWidth - tabsEl.clientWidth;
    tabsAt = { left: tabsEl.scrollLeft > 2, right: tabsEl.scrollLeft < max - 2 };
  }
  function tabsScroll(dir) {
    tabsEl?.scrollBy({ left: dir * Math.max(120, tabsEl.clientWidth * 0.7), behavior: "smooth" });
  }
  function tabsDown(e) {
    if (!tabsEl || e.button !== 0) return;
    tabDrag = { x: e.clientX, left: tabsEl.scrollLeft, moved: 0 };
    // Window listeners, never `setPointerCapture`: capture retargets the
    // `pointerup` — and the `click` derived from it — to the capturing element,
    // so the tab under the cursor stopped receiving clicks entirely.
    addEventListener("pointermove", tabsMove);
    addEventListener("pointerup", tabsUp, { once: true });
    addEventListener("pointercancel", tabsUp, { once: true });
  }
  function tabsMove(e) {
    if (!tabDrag || !tabsEl) return;
    const dx = e.clientX - tabDrag.x;
    tabDrag.moved = Math.max(tabDrag.moved, Math.abs(dx));
    // 4px is under a click's wobble and over a deliberate drag.
    if (tabDrag.moved > 4) tabsEl.scrollLeft = tabDrag.left - dx;
  }
  function tabsUp() {
    removeEventListener("pointermove", tabsMove);
    removeEventListener("pointercancel", tabsUp);
    // A press that travelled is a drag, not a pick: that click is dropped.
    tabSwallow = (tabDrag?.moved ?? 0) > 4;
    tabDrag = null;
  }
  // The row exists only while the dialog is open, and its overflow only exists
  // once its children are appended — which happens in the same flush, one tick
  // after this action runs. ResizeObserver does not help: the container's own box
  // never changes when tabs are added, only `scrollWidth` does, so the first
  // measure has to be deferred past the flush. A frame is too early; a macrotask
  // is not.
  const measureTabs = (node) => {
    tabsEl = node;
    const first = setTimeout(tabsSync, 0);
    const ro = new ResizeObserver(tabsSync);
    ro.observe(node);
    return {
      destroy() {
        clearTimeout(first);
        ro.disconnect();
        if (tabsEl === node) tabsEl = null;
      },
    };
  };
  // Publishing the field's height as a variable is what lets the transcript end
  // above it. It changes as you type a long prompt, so it is observed, not assumed.
  const measureComposer = (node) => {
    const set = () =>
      document.documentElement.style.setProperty("--composer-h", `${node.offsetHeight}px`);
    set();
    const ro = new ResizeObserver(set);
    ro.observe(node);
    return { destroy() { ro.disconnect(); } };
  };
  let models = $state({}); // provider id → live catalogue, fetched on first view
  let modelsErr = $state({});
  let modelsBusy = $state("");
  let modelFilter = $state("");
  // What the provider itself charges for this model, in USD per token. `null`
  // means it publishes no price — the page then says n/a rather than guessing.
  let price = $state(null);
  // Where the figure came from matters as much as the figure: a vendor doc is a
  // date to check, a reseller's catalogue is a list price that may not be what
  // the vendor charges your own key.
  const priceNote = $derived(
    price?.as_of
      ? ` · vendor docs, ${price.as_of}`
      : price?.source === "openrouter"
        ? " · OpenRouter list price"
        : "",
  );
  let ctx = $state(null); // {used, limit} from session.usage_info
  let adding = $state(false);
  let fresh = $state({ id: "", base_url: "", model: "", key_env: "WEBRAIN_CHAT_API_KEY" });
  // A key typed into the model dialog. It goes to the vault, never to a store.
  let keyIn = $state("");
  let effortOpen = $state(false);
  let switching = $state(""); // the model a switch is in flight for
  // The model dialog's staged pick: `{provider, model}`. Choosing only stages —
  // the session changes on Save, so browsing the catalogue cannot switch it.
  let pickDraft = $state(null);
  let settingsOpen = $state(false);
  let setTab = $state("connection");
  let editing = $state(null); // conversation id being renamed
  let editTitle = $state("");
  let think = $state(""); // reasoning text, if the runtime streams one
  let listening = $state(false);
  let effort = $state(recall("effort", "medium"));
  // The tools chooser is a dialog, not a strip of chips: a dozen tools is a list,
  // and a list you cannot search or fold by server is a list you scroll.
  let toolsOpen = $state(false);
  let toolsDraft = $state([]); // staged — Save commits, Close drops
  let toolsQuery = $state("");
  let searchOpen = $state(false);
  let toolsClosed = $state([]); // servers whose checklist is folded away
  // Agent / plan / ask was three buttons taking a third of the dock; it is one
  // choice, so it reads as one control that opens.
  let modeOpen = $state(false);
  let modeEl = $state(null);
  // A popover that only closes by picking something is a trap: any press outside
  // dismisses it, the same as every dialog on the page.
  $effect(() => {
    if (!modeOpen || !modeEl) return;
    const away = (e) => {
      if (!modeEl.contains(e.target)) modeOpen = false;
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  });
  // Four palettes, one token set: every theme is the same M3 role map with
  // different colours, so nothing below `:root` knows which one is on. `mint` is
  // the default because it is webrain's own scheme.
  const THEMES = [
    { id: "mint", label: "webrain", hint: "mint on near-black" },
    { id: "dark", label: "dark", hint: "VS Code Dark+ greys and blue" },
    { id: "monokai", label: "monokai", hint: "charcoal, lime and pink" },
    { id: "light", label: "light", hint: "VS Code Light+, for daylight" },
  ];
  let theme = $state(recall("theme", "mint"));
  // Applied here, not only in the effect below, so the saved palette is on the
  // document before the first paint instead of flashing the default.
  document.documentElement.dataset.theme = theme;
  let themeOpen = $state(false);
  let themeEl = $state(null);
  $effect(() => {
    document.documentElement.dataset.theme = theme;
    try {
      localStorage.setItem("webrain-chat-theme", theme);
    } catch {
      /* private mode — the theme simply does not stick */
    }
  });
  // Same dismiss rule as the mode menu: any press outside closes it.
  $effect(() => {
    if (!themeOpen || !themeEl) return;
    const away = (e) => {
      if (!themeEl.contains(e.target)) themeOpen = false;
    };
    document.addEventListener("pointerdown", away);
    return () => document.removeEventListener("pointerdown", away);
  });
  const MODES = [
    { id: "agent", hint: "act — browse, extract, call tools" },
    { id: "plan", hint: "think first — say what you would do" },
    { id: "ask", hint: "answer from what is already here" },
  ];
  let toolsSel = $state(readJson("tools", [])); // restored with the rest of the composer
  // Built-ins are on unless they are listed here. Storing the *off* names rather
  // than the on ones means a tool added to the binary later is available without
  // anyone re-ticking it, and the default state needs no seeding.
  let builtinOff = $state(readJson("tools-builtin-off", []));
  const builtinOn = (name) => !builtinOff.includes(name);
  function toggleBuiltin(name) {
    builtinOff = builtinOn(name)
      ? [...builtinOff, name]
      : builtinOff.filter((n) => n !== name);
  }
  let attached = $state([]); // uploads the next prompt refers to
  let recSec = $state(0); // seconds elapsed in the current recording
  let thinkMs = $state(null); // how long the reasoning block took, when known
  let thinkAt = 0; // wall clock of the first reasoning chunk, not rendered

  // The session's own provider is whichever row the server flagged, not whatever
  // happens to be first after an add.
  const activeProvider = $derived(providers.find((p) => p.active) || providers[0] || {});

  // What this model's window is, in the order the sources are trustworthy: the
  // vendor's own catalogue for that exact model, then a figure its docs publish
  // for the row, then nothing — which is printed as "not published", never as 0.
  const ctxOf = $derived(
    (models[activeProvider.id] || []).find((m) => m.id === activeProvider.model)?.context ??
      activeProvider.context ??
      null,
  );

  function ctxFor(p) {
    const m = (models[p.id] || []).find((x) => x.id === p.model);
    return m?.context ?? p.context ?? null;
  }

  function ctxText(n) {
    if (!n) return "not published by this provider";
    return n >= 1e6 ? `${(n / 1e6).toFixed(n % 1e6 ? 1 : 0)}M tokens` : `${Math.round(n / 1000)}k tokens`;
  }

  // DeepSeek bills half price outside its peak windows; this reads the rule the
  // server sends rather than restating it here.
  function offPeakNow(op) {
    const d = new Date();
    const utc = `${String(d.getUTCHours()).padStart(2, "0")}:${String(d.getUTCMinutes()).padStart(2, "0")}`;
    const h = d.getUTCHours() + d.getUTCMinutes() / 60;
    const peak =
      (op.days || []).includes(d.getUTCDay()) && (op.peak || []).some(([a, b]) => h >= a && h < b);
    return { peak, utc };
  }

  const pad2 = (n) => String(n).padStart(2, "0");

  const shown = $derived(past ?? items);
  // Which ledger the panel is reading: a saved conversation shows its own, so
  // the numbers stop changing to the live one's the moment you open history.
  const ledger = $derived(past ? pastTurns : turns);
  const stats = $derived.by(() => {
    // While a call is in flight `assistant.usage` is exact — including cache
    // reads — where the 900 ms sample is only approximate. Cost is computed from
    // the provider's published per-token price when there is one; the SDK's own
    // `nano_aiu` is 0 on BYOK turns, where the key pays the provider.
    if (call) {
      const speed = call.duration > 0 ? (call.outputTokens / (call.duration / 1000)).toFixed(1) : "—";
      return {
        model: call.model || "—",
        tok: `${call.inputTokens} / ${call.outputTokens}`,
        speed,
        cost: money(spend(call.inputTokens, call.outputTokens, call.cacheReadTokens ?? 0)),
      };
    }
    const t = ledger[0];
    return t
      ? { model: t.model, tok: `${t.input} / ${t.output}`, speed: t.speed, cost: t.cost }
      : { model: "—", tok: "— / —", speed: "—", cost: "—" };
  });
  // Nothing measured yet: the header and the panel say so once, instead of
  // printing six em-dashes and calling it data.
  const measured = $derived(!!call || ledger.length > 0);

  // What the model is doing right now, in one line, read off the events already
  // on screen: the tool that is running, the reasoning stream, or the wait for
  // the first token. VS Code shows the same thing; deriving it from the spine
  // below rather than from a second event model is what keeps the two honest.
  const liveStep = $derived.by(() => {
    if (!busy) return "";
    const last = [...items].reverse().find((i) => i.kind === "tool");
    if (last?.status === "run") return `running ${last.name}`;
    if (think) return "thinking";
    if (last) return `${last.name} returned · writing`;
    return state === "running" ? "working" : "waiting";
  });

  // One frame at a time: reading scrollHeight on every delta is what makes a
  // long reply stutter.
  function follow() {
    if (!pinned || raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      if (logEl) logEl.scrollTop = logEl.scrollHeight;
    });
  }

  function push(item) {
    // `t` is not decoration: a tool row's duration is the difference between
    // when the runtime announced the call and when it answered it.
    items = [...items, { id: ++seq, t: Date.now(), ...item }];
    follow();
  }

  function text(d) {
    return d.delta ?? d.deltaContent ?? d.text ?? "";
  }

  // Any event the page does not model still lands somewhere — but a turn emits a
  // dozen lifecycle events (mcp_server_status_changed, model.call_start, …) before
  // the first token, and one row each buried the answer they were announcing.
  // They collapse into a single `runtime` row that counts them and keeps the last
  // 200 inside a disclosure, so nothing is dropped and nothing shouts.
  function activity(type, data) {
    const line = `${type}  ${JSON.stringify(data ?? {}).slice(0, 200)}`;
    const last = items[items.length - 1];
    if (last?.kind === "act") {
      items = [
        ...items.slice(0, -1),
        { ...last, n: (last.n || 1) + 1, lines: [...(last.lines || []).slice(-200), line] },
      ];
      return follow();
    }
    push({ kind: "act", type: "runtime", n: 1, lines: [line] });
  }

  function handle(f) {
    const d = f.data || {};
    switch (f.type) {
      case "assistant.message_delta":
        streamText += text(d);
        return paintStream();
      case "assistant.message": {
        // The deltas were the same characters: settle the bubble, do not add one.
        push({ kind: "assistant", text: d.content || d.message || d.text || streamText });
        clearStream();
        // How long the reasoning block ran, from its first chunk to the answer.
        thinkMs = thinkAt ? Date.now() - thinkAt : null;
        thinkAt = 0;
        think = "";
        return;
      }
      case "tool.execution_start": {
        const id = ++seq;
        cards.set(d.toolCallId || d.toolName, id);
        return push({
          id,
          kind: "tool",
          name: (d.toolName || d.tool_name || d.name || "tool").replace(/^webrain-/, ""),
          args: d.arguments ? JSON.stringify(d.arguments, null, 1) : "",
          status: "run",
        });
      }
      case "tool.execution_complete":
      case "tool.execution_end": {
        const id = cards.get(d.toolCallId || d.toolName);
        if (id == null) return activity(f.type, d);
        const failed = d.success === false || !!d.error;
        // The result is what the tool actually returned — JSON, text, or the
        // model's error. It is shown verbatim, because a source list the page
        // re-invented would be a made-up citation.
        const raw = d.result ?? d.output ?? d.content ?? d.error ?? "";
        // The runtime hands an MCP call back as `{ content, detailedContent }`,
        // and for most servers both carry the same text — printing the envelope
        // printed the payload twice. The inner text is the result.
        const body = typeof raw === "object" && raw ? (raw.detailedContent ?? raw.content ?? raw) : raw;
        const result = typeof body === "string" ? body : JSON.stringify(body, null, 1);
        items = items.map((i) =>
          i.id === id
            ? {
                ...i,
                status: failed ? "fail" : "ok",
                note: failed ? d.error || "failed" : "",
                ms: Date.now() - (i.t ?? Date.now()),
                result: result.slice(0, 20000),
                links: linksOf(result),
              }
            : i,
        );
        return;
      }
      case "webrain.model":
        return; // the 900 ms live sample; superseded by assistant.usage when it lands
      case "assistant.usage":
        // Exact per-call numbers, straight from the runtime: tokens, cache reads,
        // reasoning tokens, TTFT, wall duration.
        call = d;
        return;
      case "webrain.usage":
        return usage(d);
      // The runtime reports the live context budget every turn. It is the one
      // number that tells you a conversation is about to be compacted.
      case "session.usage_info":
        ctx = { used: d.currentTokens ?? 0, limit: d.tokenLimit ?? 0 };
        return;
      case "webrain.permission": {
        // "Always allow this kind" is remembered here rather than in the
        // runtime: the page answers that request itself, so the modal stops
        // interrupting. It is this tab's memory only, and dies with the tab.
        if (autoKinds.includes(d.kind)) return answerPermission(d.id, true);
        // Queue: the runtime can ask twice before the first modal is answered,
        // and dropping the second would deny it after the timeout.
        asks = [...asks, d];
        return;
      }
      case "webrain.ask":
        // ask_user is a question, not a gate: the turn waits up to five minutes
        // for an answer, so the page must never swallow one.
        asksIn = [...asksIn, d];
        return;
      case "webrain.error":
      case "model.call_failure":
      case "session.error":
        return push({ kind: "err", text: d.message || f.type });
      case "assistant.turn_end":
        call = null;
        return;
      case "session.shutdown":
        state = "ended";
        return push({ kind: "err", text: "the runtime shut down — restart `webrain chat --serve`" });
      case "assistant.idle":
      case "session.idle":
        busy = false;
        state = "ready";
        call = null;
        return loadSessions();
      default:
        // A reasoning/thinking stream, if this build emits one. Matched by name
        // because the exact event id is not part of anything we can verify — and
        // an unknown event must still land somewhere, never vanish.
        if (/reason|think|thought/i.test(f.type)) {
          const chunk =
            text(d) || d.reasoningText || d.reasoning || d.thought || d.summary || d.content || "";
          if (chunk) {
            if (!thinkAt) thinkAt = Date.now();
            // shortcut: the reasoning block is re-parsed per chunk, which is fine
            // while thoughts stay a few KB; if a provider streams book-length ones,
            // buffer it through paintStream's rAF like the answer.
            think += chunk;
            return follow();
          }
        }
        // session.usage_info, session.compaction_*, tool.execution_progress, …
        return activity(f.type, d);
    }
  }

  // The route this turn took: every tool that ran since the prompt that started
  // it, in order. Each call already has its own row — this is the one line that
  // makes the *shape* of the turn legible (the strip VS Code shows above an
  // answer) without keeping a second event model in sync with the first.
  function turnSteps() {
    let from = 0;
    items.forEach((i, k) => {
      if (i.kind === "user") from = k + 1;
    });
    return items
      .slice(from)
      .filter((i) => i.kind === "tool")
      .map((i) => ({ name: i.name, status: i.status, ms: i.ms }));
  }

  function usage(u) {
    // Real money when the provider publishes a price; the runtime's own AIU
    // counter otherwise; n/a when neither exists. Never a guess.
    const usd = spend(u.input, u.output, u.cached ?? 0);
    const cost = usd != null ? money(usd) : u.nano_aiu > 0 ? `${(u.nano_aiu / 1e9).toFixed(6)} AIU` : "n/a";
    const speed = u.tokens_per_sec ? u.tokens_per_sec.toFixed(1) : "0.0";
    const turn = {
      n: turns.length + 1,
      model: u.model,
      input: u.input,
      output: u.output,
      cached: u.cached ?? 0,
      speed,
      api: `${(u.api_ms / 1000).toFixed(1)}s`,
      cost,
      // Running totals, so a saved conversation can still say how many tools it
      // called and how many sources it found without storing every row.
      tools: items.filter((i) => i.kind === "tool").length,
      sources: sources.length,
    };
    turns = [turn, ...turns];
    // Same numbers, on disk: reopening this conversation from history replays
    // them instead of showing a column of blanks.
    fetch("/ledger", { method: "POST", body: JSON.stringify({ n: turn.n, turn }) }).catch(() => {});
    const steps = turnSteps();
    if (steps.length) push({ kind: "steps", list: steps });
    push({
      kind: "usage",
      text: `${u.model} · ${u.input} in / ${u.output} out · ${u.requests} request(s) · ${speed} tok/s · ${cost}`,
    });
  }

  async function loadSessions() {
    try {
      const r = await fetch("/sessions");
      if (!r.ok) return;
      const j = await r.json();
      current = j.current || "";
      convos = j.sessions || [];
      // The live ledger survives a reload the same way the transcript does;
      // without this the next turn would be numbered 1 again and overwrite a
      // row that is already on disk. A new session (`/switch`) re-seeds too.
      if (current && current !== seededFor) {
        seededFor = current;
        const r2 = await fetch(`/sessions/${encodeURIComponent(current)}`);
        if (r2.ok) turns = (await r2.json()).turns || [];
        // First sight of this conversation: the mode, effort and tool picks it
        // was last held with come back with it.
        restoreSetup(current);
      }
    } catch {
      /* history is a convenience: a failed read leaves the menu as it was */
    }
  }

  // Leaving history is not just a view change: `viewing` is what the persist
  // effect keys the saved setup on, and the picks on screen are the ones the
  // thread you were reading had. Both have to be put back, or the live
  // conversation silently runs in the mode of the thread you just closed.
  function backToLive() {
    past = null;
    viewing = "";
    restoreSetup(current);
  }

  async function openConvo(id) {
    if (id === current) {
      backToLive();
      return follow();
    }
    const r = await fetch(`/sessions/${encodeURIComponent(id)}`);
    if (!r.ok) return;
    const j = await r.json();
    past = (j.messages || []).map((m) => ({
      kind: m.role === "user" ? "user" : "assistant",
      text: m.body,
      files: m.role === "user" ? filesOf(m.body) : undefined,
    }));
    pastTurns = j.turns || [];
    viewing = id;
    // The setup this conversation was held with comes back with it, so reading
    // an old thread does not leave the composer armed for a different one.
    restoreSetup(id);
    if (logEl) logEl.scrollTop = 0;
  }

  // MediaRecorder, not Web Speech: the bytes are the useful artifact — a file the
  // agent can read, with no third-party transcriber in the path. A cancelled take
  // is an in-memory Blob that is dropped, so nothing is written to the temp dir to
  // clean up afterwards.
  let recorder = null;
  let chunks = [];
  let cancelled = false;
  let recTimer = 0;

  async function startRecording() {
    if (recorder) return;
    if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
      return push({ kind: "err", text: "this browser cannot record audio" });
    }
    let stream;
    try {
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    } catch (e) {
      return push({ kind: "err", text: `microphone: ${e.message}` });
    }
    chunks = [];
    recSec = 0;
    recTimer = setInterval(() => (recSec += 1), 1000);
    recorder = new MediaRecorder(stream);
    recorder.ondataavailable = (e) => {
      if (e.data.size) chunks.push(e.data);
    };
    recorder.onstop = async () => {
      clearInterval(recTimer);
      for (const track of stream.getTracks()) track.stop();
      const type = recorder?.mimeType || "audio/webm";
      const keep = !cancelled;
      recorder = null;
      cancelled = false;
      listening = false;
      if (!keep) return push({ kind: "note", text: "recording discarded" });
      const ext = type.includes("ogg") ? "ogg" : type.includes("mp4") ? "m4a" : "webm";
      const blob = new Blob(chunks, { type });
      chunks = [];
      if (blob.size) await dictate(blob, ext);
    };
    recorder.start();
    listening = true;
  }

  function stopRecording(discard) {
    if (!recorder) return;
    cancelled = !!discard;
    if (recorder.state !== "inactive") recorder.stop();
  }

  // Dictation. The take goes to the local recognizer and only text comes back —
  // it is not attached, not stored, and not sent anywhere. When no recognizer is
  // installed the recording is attached as a file instead, which is what this
  // control did before and is still the useful fallback.
  let dictating = $state(false);
  async function dictate(blob, ext) {
    dictating = true;
    try {
      const r = await fetch("/transcribe", {
        method: "POST",
        headers: { "x-name": `dict.${ext}`, "x-lang": navigator.language || "" },
        body: blob,
      });
      const j = await r.json().catch(() => ({}));
      if (r.ok && j.text) {
        draft = draft.trim() ? `${draft.trim()} ${j.text}` : j.text;
        document.querySelector("textarea")?.focus();
        return;
      }
      push({ kind: "err", text: j.error || `dictation unavailable (${r.status})` });
      await attach([new File([blob], `voice-${Date.now()}.${ext}`, { type: blob.type })]);
    } catch (e) {
      push({ kind: "err", text: `dictation: ${e.message}` });
    } finally {
      dictating = false;
    }
  }

  // Off → max is what providers actually name this knob, and the levels that
  // mean something are sent to the runtime as `reasoning_effort` on the model
  // switch. `off` sends nothing, which is the only honest way to ask for the
  // model's own default.
  // Off → max is what providers actually name this knob, and the levels that
  // mean something are sent to the runtime as `reasoning_effort` on the model
  // switch. `off` sends nothing, which is the only honest way to ask for the
  // model's own default.
  const EFFORTS = ["off", "low", "medium", "high", "max"];

  // Icons are SVG, never font characters: `⛁` and `◍` are not in every font, and
  // where they are they render at a different weight on every OS. Two families —
  // one stroke set for controls, and the effort scale, which is a circle with
  // more of itself filled because that is what the levels mean.
  const stroke = (body) =>
    `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" ` +
    `stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;
  const ICON = {
    plus: stroke('<path d="M12 5v14M5 12h14"/>'),
    back: stroke('<path d="M14.5 5.5L8 12l6.5 6.5"/>'),
    forward: stroke('<path d="M9.5 5.5L16 12l-6.5 6.5"/>'),
    down: stroke('<path d="M5.5 9.5L12 16l6.5-6.5"/>'),
    right: stroke('<path d="M9.5 5.5L16 12l-6.5 6.5"/>'),
    arrowDown: stroke('<path d="M12 4.8v14.4"/><path d="M6.4 13.6L12 19.2l5.6-5.6"/>'),
    gear: stroke(
      '<circle cx="12" cy="12" r="3.1"/><path d="M12 3.4v2.3M12 18.3v2.3M3.4 12h2.3M18.3 12h2.3M5.9 5.9l1.6 1.6M16.5 16.5l1.6 1.6M18.1 5.9l-1.6 1.6M7.5 16.5l-1.6 1.6"/>',
    ),
    info: stroke('<circle cx="12" cy="12" r="8.2"/><path d="M12 11.2v5.1M12 7.9v.2"/>'),
    palette: stroke(
      '<path d="M12 3.6c-4.7 0-8.4 3.4-8.4 7.6S7.3 18.9 12 18.9c1.2 0 1.8-.8 1.8-1.7 0-1.4-1.2-1.6-1.2-2.7 0-.9.7-1.5 1.6-1.5h1.4a4.4 4.4 0 004.4-4.4c0-2.8-3.7-5-8-5z"/><circle cx="8.3" cy="9.2" r=".9"/><circle cx="12" cy="7.4" r=".9"/><circle cx="15.7" cy="9.2" r=".9"/>',
    ),
    pencil: stroke('<path d="M4 20l.9-3.7L15.6 5.6a2 2 0 012.8 2.8L7.7 19.1z"/>'),
    // Filled, not stroked: three dots at 1.8 stroke read as three rings.
    more: '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><circle cx="12" cy="5.2" r="1.85"/><circle cx="12" cy="12" r="1.85"/><circle cx="12" cy="18.8" r="1.85"/></svg>',
    x: stroke('<path d="M6.6 6.6l10.8 10.8M17.4 6.6L6.6 17.4"/>'),
    refresh: stroke('<path d="M19.4 12a7.4 7.4 0 11-2.2-5.2"/><path d="M19.7 4.7v4.4h-4.4"/>'),
    tools: stroke('<path d="M9 3.6v3.8M15 3.6v3.8"/><path d="M7 7.4h10v2.9a5 5 0 01-10 0z"/><path d="M12 15.3v5.1"/>'),
    seal: stroke('<path d="M4.6 7.4l7.4-3.7 7.4 3.7v9.2L12 20.3l-7.4-3.7z"/><path d="M12 11.2v4.3"/>'),
    // One glyph per kind of thing a turn does, so a row of tool calls reads as
    // "went to a page, read a table, ran a command" before you read a word.
    wrench: stroke('<path d="M15.4 4.6a4.4 4.4 0 00-4 6L4.7 17.3a1.9 1.9 0 002.7 2.7l6.7-6.7a4.4 4.4 0 005.5-6.1l-2.4 2.4-2.5-.6-.6-2.5z"/>'),
    globe: stroke('<circle cx="12" cy="12" r="8.2"/><path d="M3.9 12h16.2"/><path d="M12 3.8c2.4 2.5 2.4 13.9 0 16.4"/><path d="M12 3.8c-2.4 2.5-2.4 13.9 0 16.4"/>'),
    table: stroke('<rect x="4" y="5" width="16" height="14" rx="1.6"/><path d="M4 10h16M9.6 10v9"/>'),
    search: stroke('<circle cx="11" cy="11" r="6.2"/><path d="M15.6 15.6L20 20"/>'),
    file: stroke('<path d="M6.5 3.6h7.4l4.1 4.1v12.7H6.5z"/><path d="M13.9 3.6v4.1h4.1"/>'),
    image: stroke('<rect x="4" y="5.4" width="16" height="13.2" rx="1.8"/><circle cx="9" cy="10" r="1.4"/><path d="M5.6 17l4.6-4.6 3 3 2.4-2.4 3.2 3.2"/>'),
    layers: stroke('<path d="M12 4l8 4-8 4-8-4z"/><path d="M4.6 12.4L12 16l7.4-3.6"/><path d="M4.6 16.4L12 20l7.4-3.6"/>'),
    terminal: stroke('<rect x="3.8" y="5.4" width="16.4" height="13.2" rx="1.8"/><path d="M7.6 10l2.4 2.2-2.4 2.2M12.6 14.4h4"/>'),
    spark: stroke('<path d="M10 4.2l1.6 4.4 4.4 1.6-4.4 1.6L10 16.2l-1.6-4.4L4 10.2l4.4-1.6z"/><path d="M18 14.6l.7 1.9 1.9.7-1.9.7-.7 1.9-.7-1.9-1.9-.7 1.9-.7z"/>'),
    pulse: stroke('<path d="M3.6 12.4h3.2l2.4-5.6 3.2 11.2 2.4-5.6h5.6"/>'),
    check: stroke('<path d="M4.8 12.6l4.6 4.6L19.2 6.4"/>'),
    // Material "check-circle", filled: the provider this session is running now.
    current:
      '<svg viewBox="0 0 24 24" fill="currentColor" fill-rule="evenodd" aria-hidden="true"><path d="M10.6,16.6l7.05-7.05l-1.4-1.4L10.6,13.8L7.75,10.95l-1.4,1.4L10.6,16.6ZM12,22q-2.07,0-3.9-0.79q-1.82-0.79-3.17-2.14Q3.58,17.73 2.79,15.9T2,12Q2,9.92 2.79,8.1Q3.58,6.27 4.93,4.93Q6.28,3.57 8.1,2.79Q9.93,2 12,2t3.9,0.79t3.17,2.14q1.35,1.35 2.14,3.17Q22,9.92 22,12t-0.79,3.9t-2.14,3.17q-1.35,1.35-3.17,2.14T12,22Zm0-2q3.35,0 5.68-2.32T20,12T17.68,6.32T12,4T6.33,6.32T4,12t2.33,5.68T12,20Zm0-8Z"/></svg>',
    warn: stroke('<path d="M12 4.4l8.4 15H3.6z"/><path d="M12 10v4.4M12 16.9v.2"/>'),
  };

  // Which glyph a tool row gets. Matched on the name, not a table of every tool,
  // because the surface is the model's to choose and a new tool must still land
  // somewhere sensible instead of an empty slot.
  function toolIcon(name = "") {
    const n = String(name).toLowerCase();
    if (/shell|bash|command|terminal|exec/.test(n)) return ICON.terminal;
    if (/read_file|write_file|list_dir|file/.test(n)) return ICON.file;
    if (/search|serp|find/.test(n)) return ICON.search;
    if (/extract|table|scrape|csv|autoschema/.test(n)) return ICON.table;
    if (/vision|screenshot|image|pdf|watch|video/.test(n)) return ICON.image;
    if (/navigate|goto|interact|session|cookie|login|observe/.test(n)) return ICON.globe;
    if (/batch|crawl|spider|tile/.test(n)) return ICON.layers;
    if (/guide|stats|think/.test(n)) return ICON.spark;
    return ICON.wrench;
  }
  // A mode is a promise about what the turn will do, so the glyph says the
  // promise: a bolt for act, a written plan, a question. The old three were a
  // filled diamond, an outline diamond and a circle — the same shape at three
  // fills, which named nothing.
  const MODE_ICON = {
    agent:
      '<svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d="M13.5 2.3L5.1 13.6h5l-1.2 8.1L18.9 10h-5z"/></svg>',
    plan: stroke(
      '<path d="M9.6 7.1h9.2M9.6 12h9.2M9.6 16.9h9.2"/><circle cx="5.3" cy="7.1" r="1.15" fill="currentColor" stroke="none"/><circle cx="5.3" cy="12" r="1.15" fill="currentColor" stroke="none"/><circle cx="5.3" cy="16.9" r="1.15" fill="currentColor" stroke="none"/>',
    ),
    ask: stroke(
      '<path d="M20.2 12.1c0 3.5-3.7 6.3-8.2 6.3-1 0-2-.1-2.9-.4L4.4 19.7l1.4-3.3a6 6 0 01-1.9-4.3c0-3.5 3.7-6.3 8.2-6.3s8.1 2.8 8.1 6.3z"/>',
    ),
  };
  // One circle, filled to the level: off is the ring, max is the disc.
  const pie = (d) =>
    '<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="7" fill="none" ' +
    `stroke="currentColor" stroke-width="1.6"/>${d}</svg>`;
  const EFFORT_ICON = {
    off: pie(""),
    low: pie('<path d="M12 12V5a7 7 0 017 7z" fill="currentColor"/>'),
    medium: pie('<path d="M12 5a7 7 0 010 14z" fill="currentColor"/>'),
    high: pie('<path d="M12 12V5a7 7 0 010 14 7 7 0 010-7z" fill="currentColor"/>'),
    max: pie('<circle cx="12" cy="12" r="7" fill="currentColor"/>'),
  };
  const EFFORT_HINT = {
    off: "send no reasoning_effort — the model's own default",
    low: "the model thinks briefly",
    medium: "the runtime's usual balance",
    high: "more thinking, more tokens",
    max: "as much as the model allows",
  };

  // Real vendor marks: one SVG file per provider id, imported raw and inlined, so
  // the page still makes no request for its own chrome and adding a provider is
  // adding a file. The marks are drawn in currentColor, which is what lets a tab's
  // own colour rules apply to someone else's logo.
  const LOGOS = import.meta.glob("./logos/*.svg", { query: "?raw", eager: true, import: "default" });
  const logoOf = (id) => LOGOS[`./logos/${id}.svg`] ?? LOGOS["./logos/env.svg"] ?? "";

  const toolRows = $derived(catalog.items.filter((r) => r.kind === "tool"));

  // Tools by the server that owns them, so the checklist folds. `webrain-navigate`
  // is server `webrain`, tool `navigate`; a second server gets its own section
  // without this code changing.
  const toolGroups = $derived.by(() => {
    const q = toolsQuery.trim().toLowerCase();
    const groups = new Map();
    for (const { name: full } of toolRows) {
      // Tool names come off the MCP server as `webrain_navigate`; the checklist
      // reads better without the server's own prefix inside its own section.
      const bare = full.replace(/^webrain[-_]/, "");
      const cut = bare.indexOf("-");
      const server = cut > 0 ? bare.slice(0, cut) : "webrain";
      const name = cut > 0 ? bare.slice(cut + 1) : bare;
      if (q && !full.toLowerCase().includes(q) && !server.toLowerCase().includes(q)) continue;
      if (!groups.has(server)) groups.set(server, []);
      groups.get(server).push({ full, name });
    }
    return [...groups].map(([server, tools]) => ({ server, tools }));
  });

  const draftCount = $derived(toolsDraft.length);

  // Every builtin this build installs, unfiltered — the footer count has to add
  // them to the MCP ones or "18 of 18" contradicts the 9 boxes below it.
  const builtinAll = $derived(
    catalog.items.filter((r) => r.kind === "builtin").map((r) => r.name),
  );
  // Same search box filters both lists.
  const builtinShown = $derived.by(() => {
    const q = toolsQuery.trim().toLowerCase();
    return builtinAll.filter((n) => !q || n.toLowerCase().includes(q));
  });
  const builtinOnCount = $derived(builtinAll.filter(builtinOn).length);

  // The search field opens from a click, so it should already have the caret.
  // `autofocus` on the element would also fire on first paint, which is the
  // wrong moment and an a11y complaint; an action runs where it is used.
  const focusNow = (node) => node.focus();

  function openTools() {
    toolsDraft = [...toolsSel];
    toolsQuery = "";
    searchOpen = false;
    toolsOpen = true;
  }

  function toggleDraft(full) {
    toolsDraft = toolsDraft.includes(full)
      ? toolsDraft.filter((t) => t !== full)
      : [...toolsDraft, full];
  }

  // A server row ticks all of its children, or clears them — the same two
  // gestures VS Code's tool picker has, because that is the one people know.
  function allOf(server, on) {
    const names = toolGroups.find((g) => g.server === server)?.tools.map((t) => t.full) || [];
    toolsDraft = on
      ? [...new Set([...toolsDraft, ...names])]
      : toolsDraft.filter((t) => !names.includes(t));
  }

  // The builtin list is the inverse of `builtinOff`, and it is a fold like any
  // server: `toolsClosed` holds server names, so "builtin" is the one key that
  // is not a server.
  function allBuiltin(on) {
    builtinOff = on
      ? builtinOff.filter((n) => !builtinShown.includes(n))
      : [...new Set([...builtinOff, ...builtinShown])];
  }

  function toggleClosed(server) {
    toolsClosed = toolsClosed.includes(server)
      ? toolsClosed.filter((s) => s !== server)
      : [...toolsClosed, server];
  }

  const lastUser = $derived([...shown].reverse().find((i) => i.kind === "user")?.text || "");

  function retry() {
    if (!lastUser || busy) return;
    draft = lastUser;
    send();
  }

  // `delivery` is the runtime's own three-way choice, which is why the page
  // offers exactly three: a new turn (the only one refused while a turn runs),
  // a queued message that waits for it, and a steer that makes it yield now.
  async function send(delivery = "turn") {
    const prompt = draft.trim();
    if (!prompt) return;
    if (delivery === "turn" && busy) return;
    // Snapshotted before the field is cleared: what you attached is part of the
    // message, not decoration on the composer. `src` is the server's copy, so the
    // picture still renders after a reload and can always be downloaded.
    const files = attached.map((a) => ({
      name: a.name,
      bytes: a.bytes,
      src: `/attach/${encodeURIComponent(current)}/${encodeURIComponent(baseOf(a.path))}`,
    }));
    for (const a of attached) if (a.url) URL.revokeObjectURL(a.url);
    attached = [];
    draft = "";
    if (delivery === "turn") {
      past = null; // a new turn belongs to the live conversation
      viewing = "";
      busy = true;
      state = "running";
    }
    push({ kind: "user", text: prompt, files, mode });
    const r = await fetch("/turn", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ prompt, mode, effort, tools: toolsSel, tools_off: builtinOff, delivery, instructions, language }),
    });
    if (!r.ok) {
      const j = await r.json().catch(() => ({}));
      push({
        kind: "err",
        text: j.error === "busy" ? "a turn is already running" : `rejected (${r.status})`,
      });
      if (delivery === "turn") {
        busy = false;
        state = "error";
      }
      return;
    }
    if (delivery !== "turn") {
      push({
        kind: "note",
        text:
          delivery === "queue"
            ? "queued — it runs as soon as this turn finishes"
            : "steering — the current turn yields to this message",
      });
    }
  }

  // Stop is the runtime's abort; the page clears `busy` when the turn's own
  // wait returns, so a slow abort cannot leave the button dead.
  async function stopTurn() {
    if (!busy) return;
    try {
      await fetch("/stop", { method: "POST" });
    } catch (e) {
      push({ kind: "err", text: `stop: ${e.message}` });
    }
  }

  async function answer(allow) {
    if (!asks.length) return;
    const id = asks[0].id;
    asks = asks.slice(1);
    await answerPermission(id, allow);
  }

  /// Answer a pending permission request by id, whether or not its modal is the
  /// one on screen — the auto-allow path answers requests that never opened one.
  async function answerPermission(id, allow) {
    await fetch("/permission", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ id, allow }),
    });
  }

  // Model output is untrusted: it can carry HTML, and this page can approve
  // shell commands. The sanitiser lives in the markdown chunk, so until that
  // lands this returns escaped text — never model HTML.
  function md(text) {
    if (!mdReady || !render) {
      loadMd();
      return `<pre class="md-pending">${esc(text ?? "")}</pre>`;
    }
    return render(text);
  }

  async function loadCatalog() {
    try {
      const r = await fetch("/catalog");
      if (r.ok) catalog = await r.json();
    } catch {
      /* no catalog, no menus: the composer still types */
    }
  }

  // shortcut: a trigger only fires at the END of the draft, so an `@handle` or a
  // url mid-sentence never opens the menu. Caret tracking is the upgrade.
  function scanMenu() {
    const m = /(^|\s)([/@#])([\w.:-]*)$/.exec(draft);
    // `/` is only a command at the start of the draft; elsewhere it is prose.
    if (!m || (m[2] === "/" && m[1] !== "")) return (palette = null);
    if (palette && palette.trigger === m[2] && palette.query === m[3]) return;
    palette = { trigger: m[2], query: m[3], index: 0 };
  }

  function pick(o) {
    const query = palette.query;
    palette = null;
    if (o.kind === "cmd") {
      draft = "";
      if (o.name === "clear") {
        items = [];
        turns = [];
        seededFor = current; // an emptied page must not refill from the store
        clearStream();
        backToLive();
      } else {
        const named = catalog.items.filter((r) => r.kind !== "cmd").map((r) => r.name);
        push({ kind: "note", text: named.join(" · ") });
      }
      return;
    }
    const head = draft.slice(0, draft.length - (query.length + 1));
    draft = `${head}${SIGIL[o.kind]}${o.name} `;
    document.querySelector("textarea")?.focus();
  }

  // 60 ms is enough to fold keystrokes into one request and short enough to feel
  // like the results were already there. `searchSeq` drops the answers of
  // requests that a newer keystroke overtook.
  function onSearchInput() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(async () => {
      const term = q.trim();
      const mine = ++searchSeq;
      if (!term) {
        hits = [];
        return;
      }
      try {
        const r = await fetch(`/search?q=${encodeURIComponent(term)}`);
        if (!r.ok || mine !== searchSeq) return;
        const j = await r.json();
        if (mine === searchSeq) hits = j.hits || [];
      } catch {
        /* history search is a convenience: a failed read just shows nothing */
      }
    }, 60);
  }

  // Search is a field at the top of the rail, not a modal on top of the page:
  // Ctrl-K opens the rail and puts the cursor in it, so there is exactly one
  // place a query can go.
  function openSearch() {
    panelOpen = true;
    requestAnimationFrame(() => document.getElementById("railq")?.focus());
  }

  // Picking a match opens that conversation and clears the field, so the rail
  // returns to being a list.
  function pickHit(h) {
    q = "";
    hits = [];
    openConvo(h.convo);
  }

  // The matched words, not the first 80 characters of the message.
  function snippet(body, term) {
    const at = body.toLowerCase().indexOf(term.trim().toLowerCase());
    if (at < 0) return body.slice(0, 120);
    const from = Math.max(0, at - 50);
    const to = Math.min(body.length, at + term.trim().length + 90);
    return `${from > 0 ? "…" : ""}${body.slice(from, to)}${to < body.length ? "…" : ""}`;
  }

  // ── attachments ────────────────────────────────────────────────────────────
  // The server renames and caps everything; this side only decides what to send.
  async function upload(blob, name) {
    const r = await fetch("/attach", {
      method: "POST",
      headers: {
        "content-type": blob.type || "application/octet-stream",
        "x-name": encodeURIComponent(name),
      },
      body: blob,
    });
    if (!r.ok) {
      throw new Error(
        r.status === 415 ? "type not allowed" : r.status === 413 ? "too large (8 MB max)" : `upload failed (${r.status})`,
      );
    }
    return r.json();
  }

  // A file's name as the upload route stored it (`{stamp}-{stem}.{ext}`). The page
  // keeps its own copy so a sent message can ask the server for the bytes back.
  const baseOf = (p) => String(p).split(/[\\/]/).pop() || "";

  async function attach(files) {
    for (const f of files) {
      try {
        const j = await upload(f, f.name);
        // The blob URL is the thumbnail you see *before* sending. Once the
        // message is sent it points at the server's own copy instead, so nothing
        // has to stay alive in this tab for the picture to survive a reload.
        const url = kindOf(j.name) === "k-img" ? URL.createObjectURL(f) : "";
        attached = [...attached, { name: j.name, path: j.path, bytes: j.bytes, url }];
        draft += `${draft && !draft.endsWith("\n") ? "\n" : ""}[attached: ${j.name} · ${Math.max(1, Math.round(j.bytes / 1024))} KB → ${j.path}]\n`;
      } catch (e) {
        push({ kind: "err", text: `${f.name}: ${e.message}` });
      }
    }
    document.querySelector("textarea")?.focus();
  }

  // A long paste becomes a file the agent can read; a short one stays inline.
  async function onPaste(e) {
    const blob = [...(e.clipboardData?.items || [])]
      .find((i) => i.kind === "file")
      ?.getAsFile();
    if (blob) {
      e.preventDefault();
      return attach([blob]);
    }
    const text = e.clipboardData?.getData("text/plain") || "";
    const lines = text.split("\n").length;
    if (text.length <= 600 && lines <= 8) return;
    e.preventDefault();
    const n = ++attachSeq;
    try {
      const j = await upload(new Blob([text], { type: "text/markdown" }), `pasted-${n}.md`);
      draft += `[pasted text #${n} · ${lines} lines · ${Math.max(1, Math.round(j.bytes / 1024))} KB → ${j.path}]\n`;
    } catch (err) {
      push({ kind: "err", text: `paste: ${err.message}` });
    }
  }

  function onLogClick(e) {
    const btn = e.target.closest?.(".code-copy");
    if (!btn) return;
    const code = btn.closest(".code")?.querySelector("pre")?.textContent || "";
    navigator.clipboard?.writeText(code);
    btn.textContent = "copied";
    setTimeout(() => (btn.textContent = "copy"), 1200);
  }

  // One delegated listener paints the Material lift on every pressable thing —
  // buttons, the summary rows of a disclosure, and the source chips, which are
  // links and would otherwise be the only dead targets on the page.
  //
  // Everything is inline and the motion is `Element.animate`, the same way
  // deepscrape's ripple works — and for a reason found the hard way: a rule in
  // this component's `<style>` is scoped to `.svelte-hash`, so a span created at
  // run time never matched `.rip` and the animation never ran at all.
  //
  // Geometry is theirs too: the circle is sized to reach the furthest corner
  // from the press, so it covers the whole control, and it runs 500 ms linear
  // from scale(0) to scale(1.5) while the fill fades out. A dark fill on a
  // filled control, a light one on a ghost, decided from what is actually
  // painted rather than a class list.
  function ripple(e) {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const b = e.target.closest?.("button, summary, a.chip");
    if (!b) return;
    const r = b.getBoundingClientRect();
    const x = e.clientX - r.left;
    const y = e.clientY - r.top;
    const far = (p, size) => (p > size / 2 ? 0 : size);
    const radius = Math.hypot(far(x, b.offsetWidth) - x, far(y, b.offsetHeight) - y);
    const filled = getComputedStyle(b).backgroundColor !== "rgba(0, 0, 0, 0)";
    const s = document.createElement("span");
    s.style.cssText =
      `position:absolute;border-radius:50%;pointer-events:none;` +
      `left:${x - radius}px;top:${y - radius}px;width:${radius * 2}px;height:${radius * 2}px;` +
      `background:${filled ? "rgba(0,0,0,0.2)" : "rgba(255,255,255,0.18)"}`;
    b.appendChild(s);
    s.animate(
      [
        { transform: "scale(0)", opacity: 1 },
        { transform: "scale(1.5)", opacity: 0 },
      ],
      { duration: 500, easing: "linear" },
    );
    setTimeout(() => s.remove(), 500);
  }

  // The live catalogue for one provider, fetched the first time its tab is
  // opened and cached by the server for five minutes: switching tabs after that
  // costs nothing and never blocks the modal.
  async function loadModels(id, force = false) {
    if (!id || modelsBusy === id) return;
    if (!force && models[id]?.length) return;
    modelsBusy = id;
    try {
      const r = await fetch(`/models?provider=${encodeURIComponent(id)}`);
      const j = await r.json();
      models = { ...models, [id]: j.models || [] };
      modelsErr = { ...modelsErr, [id]: j.error ? String(j.error) : "" };
    } catch (e) {
      modelsErr = { ...modelsErr, [id]: e.message };
    } finally {
      modelsBusy = "";
    }
  }

  // A key pasted into the page. The server writes it to the encrypted vault under
  // the provider's name and answers with nothing but `saved`, so the value never
  // comes back to the browser or to the chat database.
  async function saveKey(id) {
    const key = keyIn.trim();
    if (!id || !key) return;
    try {
      const r = await fetch("/keys", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ provider: id, key }),
      });
      const j = await r.json();
      if (!j.saved) return push({ kind: "err", text: `key: ${j.error || "not saved"}` });
      keyIn = "";
      await loadProviders();
      // The catalogue may have failed for want of this key, so ask again.
      models = { ...models, [id]: [] };
      loadModels(id, true);
      push({
        kind: "note",
        text: `${id} key saved to the vault — an exported variable still wins over it`,
      });
    } catch (e) {
      push({ kind: "err", text: `key: ${e.message}` });
    }
  }

  // A model on the endpoint this session already talks to is a live switch that
  // keeps the context; a model on another endpoint replaces the session, and the
  // transcript's context is the thing that does not survive it. The server
  // decides which of the two applies — the page only says what happened.
  async function pickModel(provider, model) {
    if (switching) return false;
    switching = model || "…";
    try {
      const r = await fetch("/switch", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ provider, model, effort }),
      });
      const j = await r.json();
      if (!j.switched) {
        push({ kind: "err", text: `switch: ${j.error || "refused"}` });
        return false;
      }
      await loadProviders();
      await loadModels(j.provider || provider, true);
      push({
        kind: "note",
        text: j.restarted
          ? `${j.provider} · ${j.model} — a new session, so the model starts without this transcript's context`
          : `${j.model} · effort ${j.effort || "the model's default"}`,
      });
      return true;
    } catch (e) {
      push({ kind: "err", text: `switch: ${e.message}` });
      return false;
    } finally {
      switching = "";
    }
  }

  async function applyEffort() {
    if (await pickModel(activeProvider.id || "env", activeProvider.model)) effortOpen = false;
  }

  // What the model dialog's Save would do, resolved from the staged pick: its
  // catalogue row, the price the page already knows for it, and whether it lands
  // on the endpoint this session is on (a live switch) or another one (a new
  // session). The row itself cannot know that, because it is a fact about the
  // session, not about the model.
  function stages() {
    const provider = pickDraft?.provider;
    const row = providers.find((p) => p.id === provider) || {};
    const model = pickDraft?.model
      ? (models[provider] || []).find((m) => m.id === pickDraft.model)
      : null;
    return {
      provider,
      model,
      row,
      same: !!row.base_url && row.base_url === activeProvider.base_url,
      rate: model ? rate(model.pricing) : "",
    };
  }

  async function saveModel() {
    const d = stages();
    if (!d.model) return;
    if (await pickModel(d.provider, d.model.id)) modelOpen = false;
  }

  async function loadProviders() {
    try {
      const r = await fetch("/providers");
      if (r.ok) providers = (await r.json()).providers || [];
    } catch {
      /* the model modal simply lists nothing */
    }
    loadPrice();
  }

  // The provider's own published price for the model this session runs, asked
  // once. It is per-token in USD, so the page can show actual money for a turn;
  // a provider that publishes nothing leaves `price` null and cost stays n/a.
  async function loadPrice() {
    const p = activeProvider.id || "env";
    const m = activeProvider.model || "";
    if (!m) return;
    try {
      const r = await fetch(
        `/price?provider=${encodeURIComponent(p)}&model=${encodeURIComponent(m)}`,
      );
      if (!r.ok) return;
      const j = await r.json();
      const v = j.pricing;
      price = v && (v.prompt != null || v.completion != null) ? v : null;
    } catch {
      price = null;
    }
  }

  // Money for one call. Three rates, not one: a cached input token bills at a
  // fraction of a fresh one, which is the whole point of a long conversation
  // staying cheap — pricing every input token at the miss rate overstates it by
  // an order of magnitude on a warm cache. A vendor whose list price is its peak
  // rate halves it off-peak, and the page already knows which window it is in.
  // shortcut: the window is "now", so an older row is priced at today's rate.
  function spend(tokensIn, tokensOut, cachedIn = 0) {
    if (!price) return null;
    const pin = Number(price.prompt);
    const pout = Number(price.completion);
    if (!Number.isFinite(pin) || !Number.isFinite(pout)) return null;
    const hit = Number(price.input_cache_read);
    const cached = Number.isFinite(hit) && hit > 0 ? Math.min(cachedIn, tokensIn) : 0;
    const op = activeProvider.off_peak;
    const factor = op && !offPeakNow(op).peak ? Number(op.discount ?? 1) : 1;
    const raw = (tokensIn - cached) * pin + cached * (Number.isFinite(hit) ? hit : pin) + tokensOut * pout;
    return raw * factor;
  }

  function money(n) {
    if (n == null || !Number.isFinite(n)) return "n/a";
    if (n === 0) return "$0";
    return n < 0.01 ? `$${n.toFixed(5)}` : `$${n.toFixed(3)}`;
  }

  // Providers publish per-token prices; a per-million figure is the one a person
  // can read, and per-token rounds to $0.00000 at any length worth showing.
  function perM(n) {
    return Number.isFinite(n) ? `$${n < 0.01 ? n.toFixed(4) : n.toFixed(2)}` : "n/a";
  }

  // In/out per million, the way a price list reads. Empty when the catalogue has
  // no figure for this model, so a row never shows a price we do not have.
  function rate(p) {
    if (!p || p.prompt == null) return "";
    return `${perM(Number(p.prompt) * 1e6)}/${perM(Number(p.completion) * 1e6)} per 1M`;
  }

  async function addProvider() {
    const r = await fetch("/providers", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(fresh),
    });
    if (!r.ok) {
      return push({ kind: "err", text: "provider: check id, url, model and key env" });
    }
    adding = false;
    fresh = { id: "", base_url: "", model: "", key_env: "WEBRAIN_CHAT_API_KEY" };
    await loadProviders();
    push({ kind: "note", text: "provider saved — paste its key, then pick one of its models" });
  }

  // One row of the session list is a title and one button; rename and delete
  // moved into the popup that button opens, so a list of threads reads as a list
  // of threads instead of two hover targets per line.
  let menuFor = $state(null); // the conversation id whose menu is open
  // The rail scrolls, so a menu opened downward from a row near its bottom edge
  // is clipped by the scroll container. Flip it above the trigger instead.
  // shortcut: 96px is two rows plus padding; measuring the menu would need a
  // post-mount read. Raise it if the menu ever grows an item.
  let menuUp = $state(false);
  function openRowMenu(c, e) {
    if (menuFor === c.id) return (menuFor = null);
    const trigger = e.currentTarget.getBoundingClientRect();
    const rail = e.currentTarget.closest(".rail").getBoundingClientRect();
    menuUp = trigger.bottom + 96 > rail.bottom;
    menuFor = c.id;
  }
  $effect(() => {
    if (!menuFor) return;
    // `closest` rather than an element ref: the trigger lives inside an {#each},
    // so there is one per row and only one of them can be the open one.
    const away = (e) => {
      if (!e.target.closest?.(".morepick")) menuFor = null;
    };
    const esc = (e) => {
      if (e.key === "Escape") menuFor = null;
    };
    document.addEventListener("pointerdown", away);
    document.addEventListener("keydown", esc);
    return () => {
      document.removeEventListener("pointerdown", away);
      document.removeEventListener("keydown", esc);
    };
  });

  function startRename(c) {
    menuFor = null;
    editing = c.id;
    editTitle = c.title || "";
    requestAnimationFrame(() => document.querySelector(".convo.editing input")?.focus());
  }

  async function renameConvo(id) {
    const title = editTitle.trim();
    editing = null;
    if (!title) return;
    await fetch(`/sessions/${encodeURIComponent(id)}`, {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ title }),
    });
    loadSessions();
  }

  async function deleteConvo(id) {
    await fetch(`/sessions/${encodeURIComponent(id)}`, { method: "DELETE" });
    if (past) past = null;
    loadSessions();
  }

  // Everything the page remembers lives under one prefix, so "reset" is a
  // prefix sweep and a reload rather than twelve assignments that can drift.
  // The transcript is the server's: a reload loses the draft and nothing else.
  function resetSettings() {
    try {
      for (const k of Object.keys(localStorage)) {
        if (k.startsWith("webrain-chat-")) localStorage.removeItem(k);
      }
    } catch {
      /* private mode: there is nothing stored to clear */
    }
    location.reload();
  }

  // Every url a tool returned, for the sources strip. Matched with `https?://`
  // only, so a `javascript:` string in tool output can never become a link.
  // Tool output is scanned, not read: one regex over the escaped text gives keys,
  // strings, numbers and literals a colour, and a 200-line payload then has a
  // shape you can find your way around.
  const JTOK = /("(?:\\.|[^"\\])*")(\s*:)?|\b(?:true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g;
  function jsonOut(s) {
    const t = (s ?? "").trim();
    if (!t.startsWith("{") && !t.startsWith("[")) return esc(t);
    return esc(s).replace(JTOK, (m, str, colon) =>
      str
        ? `<span class="${colon ? "jk" : "js"}">${str}</span>${colon || ""}`
        : `<span class="${/^[-\d]/.test(m) ? "jn" : "jb"}">${m}</span>`,
    );
  }

  function linksOf(raw) {
    const seen = new Set();
    for (const m of JSON.stringify(raw ?? "").matchAll(/https?:\/\/[^\s"'\\<>)\]]+/g)) {
      seen.add(m[0].replace(/[.,;]+$/, ""));
      if (seen.size >= 12) break;
    }
    return [...seen];
  }

  // A modal that opens without moving focus leaves a keyboard user typing into
  // the page behind it. Focus the first control, keep Tab inside, and give focus
  // back to whatever opened it.
  function trap(node) {
    const opener = document.activeElement;
    node.querySelector("input, textarea, button")?.focus();
    const onKey = (e) => {
      if (e.key !== "Tab") return;
      const f = [...node.querySelectorAll("input, textarea, button")].filter((el) => !el.disabled);
      if (!f.length) return;
      const i = f.indexOf(document.activeElement);
      if (e.shiftKey && i <= 0) {
        e.preventDefault();
        f[f.length - 1].focus();
      } else if (!e.shiftKey && i === f.length - 1) {
        e.preventDefault();
        f[0].focus();
      }
    };
    node.addEventListener("keydown", onKey);
    return {
      destroy() {
        node.removeEventListener("keydown", onKey);
        if (opener instanceof HTMLElement) opener.focus();
      },
    };
  }

  async function answerAsk(text, freeform) {
    if (!asksIn.length) return;
    const id = asksIn[0].id;
    asksIn = asksIn.slice(1);
    freeAnswer = "";
    await fetch("/answer", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ id, answer: text || "", freeform }),
    });
  }

  function copy(text, id) {
    navigator.clipboard?.writeText(text);
    copiedId = id;
    setTimeout(() => (copiedId = -1), 1200);
  }

  // Read during init, before the persist effect below can overwrite the key with
  // whatever this process booted with — otherwise a reload always "restores" to
  // the boot model and the memory does nothing.
  const bootModel = readJson("model", null);

  // ── per-conversation setup ────────────────────────────────────────────────
  // Mode, effort and the tool picks belong to the conversation you are in, not to
  // the page: opening a past session and coming back must not silently re-arm
  // what you set somewhere else. The model is remembered here too, but it is the
  // runtime's to change and changing it replaces the live session — so history
  // offers the switch instead of taking it (see the bar under the banner).
  const setupId = (id) => `webrain-chat-setup-${id}`;

  function rememberSetup(id) {
    if (!id) return;
    try {
      localStorage.setItem(
        setupId(id),
        JSON.stringify({
          mode,
          effort,
          tools: toolsSel,
          off: builtinOff,
          provider: activeProvider.id || "",
          model: activeProvider.model || "",
        }),
      );
    } catch {
      /* private mode, or a full quota — the setup simply does not stick */
    }
  }

  function setupOf(id) {
    if (!id) return null;
    try {
      return JSON.parse(localStorage.getItem(setupId(id)) || "null");
    } catch {
      return null;
    }
  }

  function restoreSetup(id) {
    const s = setupOf(id);
    if (!s) return;
    if (s.mode) mode = s.mode;
    if (s.effort) effort = s.effort;
    if (Array.isArray(s.tools)) toolsSel = s.tools;
    if (Array.isArray(s.off)) builtinOff = s.off;
  }

  // What the conversation on screen was held with, and whether that is still what
  // this runtime is running. Both halves matter: a different model is a restart
  // of the live session, so the page asks rather than doing it.
  const viewSetup = $derived(setupOf(viewing));
  const viewModel = $derived(
    viewSetup?.provider && viewSetup?.model ? `${viewSetup.provider} · ${viewSetup.model}` : "",
  );
  const modelDiffers = $derived(
    !!viewModel &&
      (viewSetup.provider !== activeProvider.id || viewSetup.model !== activeProvider.model),
  );

  // Everything the composer lets you set — mode, effort, the preferred tools and
  // the model — is remembered and comes back on the next load. The transcript is
  // already the server's to keep; the runtime's own context is not restorable by
  // anything here, so nothing pretends it is.
  $effect(() => {
    try {
      localStorage.setItem("webrain-chat-mode", mode);
      localStorage.setItem("webrain-chat-effort", effort);
      localStorage.setItem("webrain-chat-tools", JSON.stringify(toolsSel));
      localStorage.setItem("webrain-chat-tools-builtin-off", JSON.stringify(builtinOff));
      localStorage.setItem("webrain-chat-rail-w", JSON.stringify(railW));
      localStorage.setItem("webrain-chat-inst-w", JSON.stringify(instW));
      localStorage.setItem("webrain-chat-instructions", instructions);
      localStorage.setItem("webrain-chat-language", language);
      localStorage.setItem("webrain-chat-suggest", JSON.stringify(suggest));
      // The half-written message is the one thing a reload would otherwise eat,
      // and it costs a few hundred bytes: losing a pasted prompt is worse than
      // the store. Attachments are not part of it — the file is on disk already,
      // so the chip would point at a path the next conversation may not have.
      localStorage.setItem("webrain-chat-draft", draft);
      if (activeProvider?.id && activeProvider?.model) {
        localStorage.setItem(
          "webrain-chat-model",
          JSON.stringify({ provider: activeProvider.id, model: activeProvider.model }),
        );
      }
    } catch {
      /* private mode or a full quota — the setting simply does not stick */
    }
    // The same values again, keyed by the conversation on screen: that is what
    // makes them per-session instead of per-page.
    rememberSetup(viewing || current);
  });

  // The model is the one setting the runtime does not keep, so restoring it is a
  // real switch — done once, after `/providers` says the row still exists, and
  // only when it differs from what this session booted with.
  let restoredModel = false;
  $effect(() => {
    if (restoredModel || !providers.length) return;
    restoredModel = true;
    const want = bootModel;
    if (!want?.provider || !want?.model) return;
    if (want.provider === activeProvider.id && want.model === activeProvider.model) return;
    if (!providers.some((p) => p.id === want.provider)) return;
    pickModel(want.provider, want.model);
  });

  $effect(() => {
    loadSessions();
    loadCatalog();
    loadProviders();
    const es = new EventSource("/events");
    es.onmessage = (e) => {
      try {
        handle(JSON.parse(e.data));
      } catch {
        /* a frame we cannot parse is dropped, not fatal */
      }
    };
    // EventSource reconnects on its own, so a dropped stream is news, not a
    // death sentence — the badge has to clear when it comes back or the page
    // reads "error" forever after one blip.
    es.onopen = () => {
      if (state === "error") state = busy ? "running" : "ready";
    };
    es.onerror = () => {
      if (!busy) state = "error";
    };
    const keys = (e) => {
      if ((e.key === "k" || e.key === "K") && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        return openSearch();
      }
      if (e.key === "Escape") {
        if (modelOpen) return (modelOpen = false);
        if (effortOpen) return (effortOpen = false);
        if (settingsOpen) return (settingsOpen = false);
        if (palette) return (palette = null);
        if (asks.length) return answer(false);
      }
      if (e.key === "/" && document.activeElement?.tagName !== "TEXTAREA") {
        e.preventDefault();
        document.querySelector("textarea")?.focus();
      }
    };
    window.addEventListener("keydown", keys);
    document.addEventListener("click", onLogClick);
    document.addEventListener("click", ripple);
    return () => {
      es.close();
      window.removeEventListener("keydown", keys);
      document.removeEventListener("click", onLogClick);
      document.removeEventListener("click", ripple);
      if (raf) cancelAnimationFrame(raf);
    };
  });

  const starters = [
    ["scrape a listing", "scrape every product title and price from "],
    ["compare pages", "open these two pages and tell me what changed: "],
    ["research a question", "what changed in the latest Rust release? cite the sources"],
  ];

  // ── derived evidence ───────────────────────────────────────────────────────
  // Nothing below invents content. Sources are the urls tools actually returned;
  // next steps are derived from what this turn actually did. A chip the page
  // could not point at evidence for would be a lie with a hover state.

  const sources = $derived.by(() => {
    const seen = new Map();
    for (const it of shown) {
      if (it.kind !== "tool" || !it.links) continue;
      for (const u of it.links) if (!seen.has(u)) seen.set(u, it.name);
    }
    return [...seen.entries()].map(([url, tool]) => ({ url, tool }));
  });

  const toolCount = $derived(shown.filter((i) => i.kind === "tool").length);
  // A saved conversation carries no tool rows to count, and a reloaded one has
  // lost them, so the ledger's running totals are the floor in both cases.
  const toolsShown = $derived(Math.max(toolCount, ledger[0]?.tools ?? 0));
  const sourcesShown = $derived(Math.max(sources.length, ledger[0]?.sources ?? 0));

  const nextSteps = $derived.by(() => {
    if (!shown.length || busy || past) return [];
    const out = [];
    const tools = shown.filter((i) => i.kind === "tool");
    const failed = tools.find((t) => t.status === "fail");
    if (failed) out.push(`why did ${failed.name} fail?`);
    if (sources.length >= 2) out.push("compare the top 3 sources");
    if (sources.length) out.push(`extract the tables from ${host(sources[0].url)}`);
    else if (shown.some((i) => i.kind === "assistant")) out.push("open the first page you named");
    if (tools.some((t) => /screenshot|capture|pdf/i.test(t.name))) out.push("what does the capture show?");
    return out.slice(0, 3);
  });

  function host(u) {
    try {
      return new URL(u).hostname.replace(/^www\./, "");
    } catch {
      return u;
    }
  }

  function ms(n) {
    return n < 1000 ? `${Math.round(n)}ms` : `${(n / 1000).toFixed(1)}s`;
  }

  function kb(n) {
    return n < 1024 ? `${n}B` : `${Math.max(1, Math.round(n / 1024))}KB`;
  }

  // What kind of thing is riding along: a `.pdf` and a `.md` reach the agent with
  // very different consequences, and one coloured edge is the cheap way to say
  // which before the prompt is sent.
  function kindOf(name) {
    const ext = (String(name).split(".").pop() || "").toLowerCase();
    if (/^(png|jpe?g|gif|webp|bmp|svg|ico|avif)$/.test(ext)) return "k-img";
    if (/^(webm|wav|mp3|ogg|m4a|mp4|flac)$/.test(ext)) return "k-aud";
    if (/^(csv|tsv|json|jsonl|xml|yml|yaml|toml|ini)$/.test(ext)) return "k-data";
    if (/^(rs|js|ts|jsx|tsx|py|css|scss|html|sh|ps1|sql|go|java|c|h|cpp)$/.test(ext)) return "k-code";
    return "k-doc";
  }

  // ── generative UI ──────────────────────────────────────────────────────────
  // A tool result is data, so the page picks a view by *shape*: rows become a
  // table, a flat object becomes a grid, links become chips, and anything else
  // stays preformatted — which is always correct. No tool is named here, so an
  // unfamiliar payload degrades instead of breaking.
  function shape(raw) {
    if (!raw) return { kind: "text" };
    let v;
    try {
      v = JSON.parse(raw);
    } catch {
      return { kind: "text" };
    }
    const rows = Array.isArray(v)
      ? v
      : Array.isArray(v?.rows)
        ? v.rows
        : Array.isArray(v?.items)
          ? v.items
          : Array.isArray(v?.results)
            ? v.results
            : null;
    if (rows?.length && rows.every((r) => r && typeof r === "object" && !Array.isArray(r))) {
      const cols = [...new Set(rows.flatMap((r) => Object.keys(r)))].slice(0, 6);
      if (cols.length && cols.length <= 6) return { kind: "table", cols, rows: rows.slice(0, 50) };
    }
    if (v && typeof v === "object" && !Array.isArray(v)) {
      const kv = Object.entries(v).filter(
        ([, x]) => x == null || ["string", "number", "boolean"].includes(typeof x),
      );
      if (kv.length >= 2 && kv.length <= 12) return { kind: "kv", kv };
    }
    return { kind: "text" };
  }

  function cell(v) {
    return v == null ? "—" : typeof v === "object" ? JSON.stringify(v) : String(v);
  }

  // ── branching ──────────────────────────────────────────────────────────────
  // A fork copies the thread up to this message into a new conversation. The
  // runtime keeps one linear context, so this is a clean place to carry on from,
  // not a rewind of what the model remembers — and the tooltip says exactly that.
  async function fork(item) {
    const upto = shown.findIndex((x) => x === item) + 1;
    if (upto < 1) return;
    try {
      const r = await fetch("/fork", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ convo: current, upto }),
      });
      if (!r.ok) throw new Error(`fork failed (${r.status})`);
      const j = await r.json();
      await loadSessions();
      await openConvo(j.convo);
    } catch (e) {
      push({ kind: "err", text: e.message });
    }
  }

  function clearAttachments() {
    // Same matcher as the transcript: a receipt the caret ran into is still a
    // receipt, and a prefix test on whole lines missed it.
    draft = draft
      .replace(ATTACHED_LINE, " ")
      .replace(/[ \t]{2,}/g, " ")
      .replace(/\n{3,}/g, "\n\n")
      .trim();
    // The blob URLs are the tab's only owner of those bytes: dropping the list
    // without revoking would keep every preview alive until the tab closed.
    for (const a of attached) if (a.url) URL.revokeObjectURL(a.url);
    attached = [];
  }

  // The `[attached: …]` line is what reached the model, so it is also the record:
  // replaying a conversation can put the pictures back without storing a second
  // copy of the transcript in the browser. The last two path segments are the
  // conversation and the stored file — the same two `/attach/` asks for.
  //
  // Not anchored to a line: a receipt typed on the end of a sentence (focus, then
  // keep writing) is still a receipt, and it was invisible to the anchored form.
  // `.` stops at a newline, so a match can never swallow the prose around it.
  const ATTACHED_LINE = /\[attached: (?<name>.+?) · (?<kb>\d+) KB → (?<path>.+?)\]/g;

  function filesOf(body) {
    const out = [];
    for (const m of String(body || "").matchAll(ATTACHED_LINE)) {
      const seg = m.groups.path.split(/[\\/]/).slice(-2).map(encodeURIComponent);
      if (seg.length !== 2) continue;
      out.push({
        name: m.groups.name,
        bytes: Number(m.groups.kb) * 1024,
        src: `/attach/${seg[0]}/${seg[1]}`,
      });
    }
    return out;
  }

  // The attachment receipt is redundant once the pictures are on screen.
  const withoutLines = (t) =>
    String(t || "")
      .replace(ATTACHED_LINE, " ")
      .replace(/[ \t]{2,}/g, " ")
      .replace(/\n{3,}/g, "\n\n")
      .trim();

  // ── approval ───────────────────────────────────────────────────────────────
  // "Always allow this kind" is answered by the page, not the runtime: the same
  // request arriving again is allowed without a modal. It is session-scoped and
  // never touches the permission handler's own state.
  let autoKinds = $state([]);

  function allowKind(kind) {
    if (kind && !autoKinds.includes(kind)) autoKinds = [...autoKinds, kind];
  }
</script>

<header class="topbar">
  <button
    class="icon"
    onclick={() => (panelOpen = !panelOpen)}
    aria-expanded={panelOpen}
    aria-label="toggle sessions"
    title="sessions"
  >
    <span class="gi" aria-hidden="true">{@html panelOpen ? ICON.back : ICON.forward}</span>
  </button>
  <span class="wordmark">webrain<i>chat</i></span>
  <span class="state {state}" role="status">{state === "running" ? "running" : state}</span>
  <span class="grow"></span>
  {#if measured}
    <span class="meter-inline" title="tokens in / out">
      <b>{stats.tok}</b><u>tok</u>
    </span>
    <span class="meter-inline" title="output tokens per second">
      <b>{stats.speed}</b><u>tok/s</u>
    </span>
  {/if}
  <div class="themepick" bind:this={themeEl}>
    <button
      class="icon"
      onclick={() => (themeOpen = !themeOpen)}
      aria-haspopup="menu"
      aria-expanded={themeOpen}
      aria-label="colour theme"
      title="colour theme"
    >
      <span class="gi" aria-hidden="true">{@html ICON.palette}</span>
    </button>
    {#if themeOpen}
      <div class="thememenu" role="menu" aria-label="colour theme">
        {#each THEMES as t (t.id)}
          <button
            role="menuitemradio"
            aria-checked={theme === t.id}
            class:on={theme === t.id}
            onclick={() => {
              theme = t.id;
              themeOpen = false;
            }}
          >
            <span class="swatch sw-{t.id}" aria-hidden="true"><i></i><i></i><i></i></span>
            <b>{t.label}</b>
            <small>{t.hint}</small>
          </button>
        {/each}
      </div>
    {/if}
  </div>
</header>

<div class="deck" class:open={panelOpen}>
  <!-- `click` is delegated from a document listener (see the effect): the
       markdown renderer's code-copy buttons are real buttons. -->
  <main
    bind:this={logEl}
    onscroll={() => {
      if (!logEl) return;
      pinned = logEl.scrollHeight - logEl.scrollTop - logEl.clientHeight < 48;
    }}
  >
    {#if past}
      <div class="banner">
        reading history
        <button onclick={backToLive}>back to live</button>
        <!-- A conversation remembers the model it ran on. Switching to it is a
             restart of the live session, so it is offered here and never taken
             on your behalf while you are only reading. -->
        {#if viewModel}
          <span class="dim">· ran on {viewModel}</span>
          {#if modelDiffers}
            <button
              onclick={() => pickModel(viewSetup.provider, viewSetup.model)}
              title="make this the live model — a different endpoint starts a new session"
            >
              use it
            </button>
          {/if}
        {/if}
      </div>
    {/if}

    {#if shown.length === 0 && !streamText}
      <div class="empty">
        <p class="eyebrow">a scraper you talk to</p>
        <h1>Ask for evidence,<br />not a summary.</h1>
        <p class="lede">
          This agent has the binary's own tools — browse, extract, screenshot, PDF, batch.
          Every claim it makes arrives with the tool result that backs it.
        </p>
        <div class="starters">
          {#each starters as [title, prompt] (title)}
            <button class="starter" onclick={() => (draft = prompt)}>
              <b>{title}</b><span>{prompt}</span>
            </button>
          {/each}
        </div>
      </div>
    {/if}

    {#each shown as item, i (i)}
      {#if item.kind === "tool"}
        {@const shape_ = shape(item.result)}
        <details class="tool {item.status}" open={item.status === "run"}>
          <summary>
            <span class="gi ev" aria-hidden="true">{@html toolIcon(item.name)}</span>
            <b>{item.name}</b>
            {#if item.ms != null}<span class="dur">{ms(item.ms)}</span>{/if}
            {#if item.status === "fail"}<span class="pill bad">failed</span>
            {:else if item.status === "ok" && item.ms != null && item.ms < 400}
              <span class="pill ok">fast</span>
            {/if}
            {#if item.note}<span class="bad">{item.note}</span>{/if}
          </summary>
          <!-- Input and result are the two halves of a report, so they are
               labelled and each carries its own copy: what the model asked for
               is evidence too, and it is the half people forget to quote. -->
          {#if item.args}
            <div class="io inp">
              <span class="lbl">input</span>
              <button class="linky" onclick={() => copy(item.args, -item.id)}>
                {copiedId === -item.id ? "copied" : "copy"}
              </button>
            </div>
            <pre class="args">{@html jsonOut(item.args)}</pre>
          {/if}
          {#if item.links?.length}
            <div class="src">
              {#each item.links as u (u)}
                <a class="chip" href={u} target="_blank" rel="noopener noreferrer"
                  >{u.replace(/^https?:\/\//, "").slice(0, 64)}</a
                >
              {/each}
            </div>
          {/if}
          {#if item.result}
            <div class="io outp">
              <span class="lbl">output</span>
              <span class="dim">{item.result.length.toLocaleString()} chars</span>
              <button class="linky" onclick={() => copy(item.result, -(item.id + 1e6))}>
                {copiedId === -(item.id + 1e6) ? "copied" : "copy"}
              </button>
            </div>
          {/if}
          {#if item.result}
            {#if shape_.kind === "table"}
              <div class="table-wrap">
                <table>
                  <thead><tr>{#each shape_.cols as c (c)}<th>{c}</th>{/each}</tr></thead>
                  <tbody>
                    {#each shape_.rows as r, ri (ri)}
                      <tr>{#each shape_.cols as c (c)}<td>{cell(r[c])}</td>{/each}</tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {:else if shape_.kind === "kv"}
              <dl class="kv">
                {#each shape_.kv as [k, v] (k)}<dt>{k}</dt><dd>{cell(v)}</dd>{/each}
              </dl>
            {:else}
              <pre class="out">{@html jsonOut(item.result)}</pre>
            {/if}
          {/if}
        </details>
      {:else if item.kind === "act"}
        <details class="act">
          <summary>
            <span class="gi ev" aria-hidden="true">{@html ICON.pulse}</span>
            runtime<span class="dim"> · {item.n} event{item.n === 1 ? "" : "s"}</span>
          </summary>
          <pre>{(item.lines || [item.json]).join("\n")}</pre>
        </details>
      {:else if item.kind === "steps"}
        <!-- The turn in one line: what ran, in what order, and that an answer
             came out of it. Each call still has its own card above; this is the
             route, which is what you need when the answer is wrong. -->
        <div class="steps">
          <span class="eyebrow">steps</span>
          <span class="route">
            {#each item.list as s, k (k)}
              {#if k}<span class="arrow" aria-hidden="true">→</span>{/if}
              <span class="stp {s.status}" title={s.status === "fail" ? "failed" : "returned"}>
                <i aria-hidden="true"></i>{s.name}
                {#if s.ms != null}<u>{ms(s.ms)}</u>{/if}
              </span>
            {/each}
            <span class="arrow" aria-hidden="true">→</span>
            <span class="stp done"><i aria-hidden="true"></i>answer</span>
          </span>
        </div>
      {:else if item.kind === "usage"}
        <div class="usage">{item.text}</div>
      {:else if item.kind === "note"}
        <div class="note">{item.text}</div>
      {:else}
        <div class="msg {item.kind}">
          <div class="who">
            {item.kind === "user" ? "you" : item.kind === "assistant" ? "webrain" : "error"}
            {#if item.kind === "user" && item.mode && item.mode !== "agent"}
              <span class="pill mode">{item.mode}</span>
            {/if}
            {#if item.kind === "user" && !past && !busy}
              <button class="linky" onclick={() => fork(item)} title="copy this thread up to here into a new conversation">
                fork
              </button>
            {/if}
          </div>
          {#if item.kind === "assistant"}
            <button class="copy" onclick={() => copy(item.text, item.id ?? -1)}>
              {copiedId === item.id ? "copied" : "copy"}
            </button>
            <div class="body md">{@html md(item.text)}</div>
          {:else}
            <div class="body">
              <!-- What you sent stays sent: the image is the server's copy, so it
                   renders after a reload and the link is a download. -->
              {#if item.files?.length}
                <div class="shots">
                  {#each item.files as f (f.src)}
                    <a class="shot" href="{f.src}?dl=1" download={f.name} title="download {f.name}">
                      {#if kindOf(f.name) === "k-img"}
                        <img src={f.src} alt={f.name} loading="lazy" />
                      {:else}
                        <span class="doc {kindOf(f.name)}" aria-hidden="true">
                          {kindOf(f.name) === "k-aud" ? "audio" : "file"} · {kb(f.bytes)}
                        </span>
                      {/if}
                      <span class="cap">{f.name}</span>
                    </a>
                  {/each}
                </div>
              {/if}
              {item.files?.length ? withoutLines(item.text) : item.text}
            </div>
          {/if}
        </div>
      {/if}
    {/each}

    {#if think}
      <details class="think">
        <summary>
          <span class="gi ev" aria-hidden="true">{@html ICON.spark}</span>
          thinking{#if thinkMs != null}<span class="dur">{ms(thinkMs)}</span>{/if}
        </summary>
        <!-- Reasoning is prose, so it gets the answer's renderer rather than a
             wall in a <pre>: a numbered plan arrives as a numbered list. -->
        <div class="body md">{@html md(think)}</div>
      </details>
    {/if}

    {#if streamText}
      <div class="msg assistant streaming" aria-live="polite">
        <div class="who">webrain</div>
        <div class="body md">{@html md(streamView)}<i class="caret"></i></div>
      </div>
    {:else if busy}
      <div class="msg assistant streaming">
        <div class="who">
          webrain
          <span class="gi spin" aria-hidden="true">{@html ICON.spark}</span>
          <span class="dim step">{liveStep}</span>
        </div>
        <div class="body"><i class="caret"></i></div>
      </div>
    {/if}

    <!-- Every url any tool returned, deduplicated across the whole turn. It is
         the same data as the chips on each tool row, collected where you can
         actually count it. -->
    {#if sources.length}
      <section class="evidence">
        <h3>sources <span class="dim">{sources.length}</span></h3>
        <ol>
          {#each sources as s (s.url)}
            <li>
              <a href={s.url} target="_blank" rel="noopener noreferrer">{host(s.url)}</a>
              <span class="dim">{s.tool}</span>
            </li>
          {/each}
        </ol>
      </section>
    {/if}

    <!-- Next steps are derived from what this turn actually did — a failed
         tool, the hosts that came back, a screenshot that was taken. The page
         never invents a suggestion it cannot point at evidence for. -->
    {#if nextSteps.length && !busy && !past && suggest}
      <div class="next">
        <span class="eyebrow">next</span>
        {#each nextSteps as s (s)}
          <button class="chip" onclick={() => { draft = s; document.querySelector("textarea")?.focus(); }}>{s}</button>
        {/each}
      </div>
    {/if}
  </main>

  {#if !pinned}
    <button
      class="jump"
      onclick={() => {
        pinned = true;
        if (logEl) logEl.scrollTop = logEl.scrollHeight;
      }}>
      <span class="gi" aria-hidden="true">{@html ICON.arrowDown}</span> latest</button
    >
  {/if}

  <aside class="rail" inert={!panelOpen}>
    <!-- Search is the first thing in the rail, because the rail is where history
         lives. Typing replaces the thread list with matches; clearing restores
         it. One field, one place. -->
    <div class="rail-search">
      <svg
        class="mag"
        viewBox="0 0 24 24"
        width="13"
        height="13"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
        aria-hidden="true"
      >
        <circle cx="10.5" cy="10.5" r="6.5" />
        <path d="m15.5 15.5 4 4" />
      </svg>
      <input
        id="railq"
        bind:value={q}
        oninput={onSearchInput}
        placeholder="search history"
        aria-label="search threads and messages"
        spellcheck="false"
        autocomplete="off"
      />
      {#if q}
        <button class="clearq" onclick={() => { q = ""; hits = []; }} aria-label="clear search">
          <span class="gi" aria-hidden="true">{@html ICON.x}</span>
        </button>
      {/if}
    </div>

    {#if q.trim()}
      <div class="side-h">matches <span class="count">{hits.length}</span></div>
      {#if hits.length === 0}
        <p class="none">nothing matches “{q.trim()}”</p>
      {/if}
      {#each hits as h, i (i)}
        <button class="rhit" onclick={() => pickHit(h)}>
          <span class="rtitle">{h.title || h.convo}</span>
          <span class="rsnip">{snippet(h.body, q)}</span>
        </button>
      {/each}
    {:else}
      <div class="side-h">threads</div>
      {#if convos.length === 0}
        <p class="none">no history yet</p>
      {/if}
      {#each convos as c (c.id)}
        {#if editing === c.id}
          <div class="convo editing">
            <input
              bind:value={editTitle}
              onkeydown={(e) => {
                if (e.key === "Enter") renameConvo(c.id);
                if (e.key === "Escape") editing = null;
              }}
              onblur={() => renameConvo(c.id)}
            />
          </div>
        {:else}
          <div class="row">
            <button
              class="convo"
              class:on={!past && c.id === current}
              class:reading={!!past && c.id !== current}
              onclick={() => openConvo(c.id)}
              title={c.id}
            >
              <span class="title">{c.id === current ? "live · " : ""}{c.title || "(no prompt)"}</span>
              <span class="meta">{c.ts} · {c.n} msg</span>
            </button>
            <div class="morepick">
              <button
                class="mini more"
                class:on={menuFor === c.id}
                title="rename or delete"
                aria-label="more"
                aria-haspopup="menu"
                aria-expanded={menuFor === c.id}
                onclick={(e) => openRowMenu(c, e)}
              >
                <span class="gi" aria-hidden="true">{@html ICON.more}</span>
              </button>
              {#if menuFor === c.id}
                <div class="modemenu moremenu" class:up={menuUp} role="menu" aria-label="this conversation">
                  <button role="menuitem" onclick={() => startRename(c)}>
                    <span class="gi" aria-hidden="true">{@html ICON.pencil}</span>
                    <b>rename</b>
                  </button>
                  <button
                    role="menuitem"
                    class="danger"
                    onclick={() => {
                      menuFor = null;
                      deleteConvo(c.id);
                    }}
                  >
                    <span class="gi" aria-hidden="true">{@html ICON.x}</span>
                    <b>delete</b>
                  </button>
                </div>
              {/if}
            </div>
          </div>
        {/if}
      {/each}
    {/if}

    <div class="side-foot">
      <button class="gear" onclick={() => { modelOpen = false; settingsOpen = true; }}>
        <span class="gi" aria-hidden="true">{@html ICON.gear}</span> settings
      </button>
      <button
        class="gear"
        onclick={() => {
          setTab = "about";
          modelOpen = false;
          settingsOpen = true;
        }}
      >
        <span class="gi" aria-hidden="true">{@html ICON.info}</span> about
      </button>
    </div>
  </aside>

  <!-- Instruments: the numbers a scraping harness is judged on, live, beside
       the transcript instead of crammed into a header. -->
  <aside class="inst" aria-label="session instruments">
    <div class="side-h">{past ? "last turn" : "this turn"}</div>
    {#if measured}
      <dl class="readout">
        <dt>model</dt><dd>{stats.model}</dd>
        <dt>tokens</dt><dd>{stats.tok}</dd>
        <dt>tok/s</dt><dd>{stats.speed}</dd>
        <dt>cost</dt><dd>{stats.cost}</dd>
        <dt>tools</dt><dd>{toolsShown}</dd>
        <dt>sources</dt><dd>{sourcesShown}</dd>
      </dl>
    {:else}
      <!-- Six em-dashes read as "these numbers are zero". They are not measured
           yet, which is a different thing, so say it once. -->
      <p class="none">nothing measured yet — the numbers land with the first answer</p>
    {/if}

    <!-- Context budget, straight from `session.usage_info`: the one number that
         says whether this conversation is about to be compacted. -->
    {#if ctx?.limit}
      <div class="side-h">context</div>
      <div
        class="ctx"
        role="meter"
        aria-valuenow={ctx.used}
        aria-valuemin="0"
        aria-valuemax={ctx.limit}
        aria-label="context used"
      >
        <i style="width: {Math.min(100, (ctx.used / ctx.limit) * 100).toFixed(1)}%"></i>
      </div>
      <p class="ctx-n">
        {((ctx.used / ctx.limit) * 100).toFixed(0)}% · {(ctx.used / 1000).toFixed(1)}k of
        {(ctx.limit / 1000).toFixed(0)}k
      </p>
      <!-- The model's advertised window and the budget the runtime actually sends
           are two different numbers: the runtime compacts at its own, and a 1M
           model behind a 128k session budget will never see 1M here. Say both. -->
      {#if ctxOf && ctxOf !== ctx.limit}
        <p
          class="ctx-n dim"
          title="This session's budget is the runtime's, not the model's advertised window — it compacts at the smaller number."
        >
          {ctxText(ctxOf)} model · {ctxText(ctx.limit)} session
        </p>
      {/if}
    {/if}

    <div class="side-h">turns</div>
    {#each ledger as t (t.n)}
      <div class="turn">
        <div class="n"><span>{t.n}</span>{t.model}</div>
        <dl>
          <dt>in</dt><dd>{t.input}</dd>
          <dt>out</dt><dd>{t.output}</dd>
          <dt>tok/s</dt><dd>{t.speed}</dd>
          <dt>api</dt><dd>{t.api}</dd>
          <dt>cost</dt>
          <dd title="{t.cached} of {t.input} input tokens were served from the prompt cache, which bills below the fresh rate">
            {t.cost === "n/a" && t.model === activeProvider.model
              ? money(spend(t.input, t.output, t.cached))
              : t.cost}
          </dd>
        </dl>
      </div>
    {/each}
    {#if ledger.length === 0}<p class="none">no turns yet</p>{/if}
  </aside>

  <!-- The two handles. Absolutely placed because what they resize is the grid
       columns themselves, and they follow the same two vars the grid does. -->
  <div
    class="grip l"
    role="separator"
    aria-orientation="vertical"
    aria-label="resize the sessions rail"
    tabindex="0"
    onpointerdown={(e) => gripDown("rail", e)}
    onkeydown={(e) => gripKey("rail", e)}
  ></div>
  <div
    class="grip r"
    role="separator"
    aria-orientation="vertical"
    aria-label="resize the session instruments"
    tabindex="0"
    onpointerdown={(e) => gripDown("inst", e)}
    onkeydown={(e) => gripKey("inst", e)}
  ></div>
</div>

<div id="composer" use:measureComposer>
  {#if palette && options.length}
    <div class="palette" role="listbox">
      <div class="head">
        {palette.trigger === "/" ? "everything" : palette.trigger === "@" ? "skills" : "tools"}
        <span class="dim"
          >{options.length}{menuRows.length < options.length ? " · " + menuRows.length + " shown" : ""}</span
        >
      </div>
      {#each menuRows as o, i (o.kind + o.name)}
        <button
          class="opt k-{o.kind}"
          class:on={i === palette.index}
          role="option"
          aria-selected={i === palette.index}
          onclick={() => pick(o)}
          onmouseenter={() => (palette.index = i)}
        >
          <b>{SIGIL[o.kind]}{o.name}</b>
          {#if o.hint}<span class="dim">{o.hint}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
  <!-- Attachments are files on disk the agent was told about; the chips are the
       receipt, and the prompt lines are what actually reaches the model. -->
  {#if attached.length}
    <div class="attached">
      {#each attached as a (a.path)}
        <span class="chip {kindOf(a.name)}" title={a.path}>
          {#if a.url}<img class="thumb" src={a.url} alt="" />{/if}
          {a.name} <span class="dim">{kb(a.bytes)}</span>
        </span>
      {/each}
      <button class="linky" onclick={clearAttachments}>remove all</button>
    </div>
  {/if}
  <footer>
    <input
      bind:this={fileInput}
      type="file"
      multiple
      hidden
      accept="image/png,image/jpeg,image/gif,image/webp,audio/webm,audio/wav,audio/mpeg,audio/ogg,audio/mp4,.txt,.md,.json,.csv,.log,.yml,.yaml,.toml,.rs,.js,.ts,.py,.css,.xml"
      onchange={(e) => {
        attach([...e.currentTarget.files]);
        e.currentTarget.value = "";
      }}
    />
    <textarea
      bind:value={draft}
      disabled={busy}
      placeholder="Ask, scrape, extract — / commands, @ skills, # tools, + attach"
      oninput={() => scanMenu()}
      onpaste={onPaste}
      onkeydown={(e) => {
        if (palette && menuRows.length) {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            // Over the rows on screen: arrowing onto a row past the fold would
            // select something the user cannot see.
            const n = menuRows.length;
            const d = e.key === "ArrowDown" ? 1 : -1;
            palette.index = (palette.index + d + n) % n;
            return;
          }
          if (e.key === "Enter" || e.key === "Tab") {
            e.preventDefault();
            return pick(menuRows[palette.index]);
          }
        }
        if (e.key === "Escape" && listening) {
          e.preventDefault();
          return stopRecording(true);
        }
        if (e.key === "Enter" && !e.shiftKey) {
          e.preventDefault();
          // While a turn runs, Enter steers it and Alt+Enter queues behind it —
          // the two choices the runtime has that a plain send does not.
          if (e.altKey) return send("queue");
          if (busy) return send("steer");
          send();
        }
      }}
    ></textarea>
    <!-- Dictation sits beside Send, where the thumb is: record, transcribe
         locally, and the text lands in the field above. -->
    <button
      class="ico rec"
      class:live={listening}
      class:busy={dictating}
      onclick={() => (listening ? stopRecording(false) : startRecording())}
      title={listening
        ? "stop and transcribe · Esc discards"
        : dictating
          ? "transcribing…"
          : "dictate — recorded locally, transcribed by Nemotron 3.5 ASR"}
      aria-label="dictate"
      aria-pressed={listening}
    >
      {#if listening}
        <span class="rec-ms" aria-hidden="true">{recSec}s</span>
        <span class="rec-stop" aria-hidden="true"></span>
      {:else}
        <svg
          viewBox="0 0 24 24"
          width="16"
          height="16"
          fill="none"
          stroke="currentColor"
          stroke-width="1.7"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <rect x="9" y="3" width="6" height="11" rx="3" />
          <path d="M5.5 11a6.5 6.5 0 0013 0" />
          <path d="M12 17.5V21" />
          <path d="M8.5 21h7" />
        </svg>
      {/if}
    </button>
    {#if busy}
      <!-- One small icon each, and they are the runtime's own two ways to
           deliver a message mid-turn: queue behind it, or make it yield. -->
      <button
        class="mini"
        onclick={() => send("queue")}
        disabled={!draft.trim()}
        title="queue this message to send after the current turn — Alt+Enter"
        aria-label="add to queue"
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor"
          stroke-width="1.7" stroke-linecap="round" aria-hidden="true">
          <path d="M4 7h16M4 12h16M4 17h10" />
        </svg>
      </button>
      <button
        class="mini"
        onclick={() => send("steer")}
        disabled={!draft.trim()}
        title="send now, letting the current turn yield — Enter"
        aria-label="steer with message"
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor"
          stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M4 18l6-6 4 3 6-8" />
          <path d="M20 7v5h-5" />
        </svg>
      </button>
      <button aria-label="stop" class="send stop" onclick={stopTurn} title="stop this turn">
        <svg
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 24 24"
          width="16"
          height="16"
          fill="currentColor"
          aria-hidden="true"
        >
          <path d="M6 6h12v12H6z" />
        </svg>
        
      </button>
    {:else}
      <button aria-label="stop" title="send this prompt" class="send" onclick={() => send()} disabled={!draft.trim()}>
        <svg
          xmlns="http://www.w3.org/2000/svg"
          viewBox="0 0 24 24"
          width="16"
          height="16"
          fill="currentColor"
          aria-hidden="true"
        >
          <path d="M3,20V4l19,8L3,20Zm2-3l11.85-5L5,7v3.5L11,12L5,13.5V17Zm0,0V12V7v3.5v3V17Z" />
        </svg>
      </button>
    {/if}
  </footer>

  <!-- The dock: everything that shapes the next turn, under the field it
       shapes, in the order the turn is built — attach, mode, effort, tools. -->
  <div class="dock">
    <button
      class="ico"
      onclick={() => fileInput?.click()}
      title="attach a file, or a finished voice note"
      aria-label="attach"
    >
      <span class="gi" aria-hidden="true">{@html ICON.plus}</span>
    </button>
    <div class="modepick" bind:this={modeEl}>
      <button
        class="ghost"
        onclick={() => (modeOpen = !modeOpen)}
        aria-haspopup="menu"
        aria-expanded={modeOpen}
        title="{mode} mode — what the turn is allowed to do"
      >
        <span class="gi" aria-hidden="true">{@html MODE_ICON[mode] ?? MODE_ICON.agent}</span>
        {mode}
      </button>
      {#if modeOpen}
        <div class="modemenu" role="menu" aria-label="agent mode">
          {#each MODES as m (m.id)}
            <button
              role="menuitemradio"
              aria-checked={mode === m.id}
              class:on={mode === m.id}
              onclick={() => {
                mode = m.id;
                modeOpen = false;
              }}
            >
              <span class="gi" aria-hidden="true">{@html MODE_ICON[m.id]}</span>
              <b>{m.id}</b>
              <small>{m.hint}</small>
            </button>
          {/each}
        </div>
      {/if}
    </div>
    <button
      class="ghost"
      onclick={() => {
        modelOpen = false;
        settingsOpen = false;
        effortOpen = true;
        loadModels(activeProvider.id);
      }}
      title="thinking effort — sent to the runtime as reasoning_effort, plus what this provider and model can do"
      aria-haspopup="dialog"
    >
      <span class="gi" aria-hidden="true">{@html EFFORT_ICON[effort] ?? EFFORT_ICON.medium}</span>
      {effort}
    </button>
    <button
      class="ghost"
      class:on={toolsSel.length > 0}
      onclick={openTools}
      title="choose which MCP tools this turn should reach for"
      aria-haspopup="dialog"
    >
      <span class="gi" aria-hidden="true">{@html ICON.tools}</span>
      {toolsSel.length ? `tools ${toolsSel.length}` : "tools"}
    </button>
    {#if lastUser && !busy}
      <button class="ghost" onclick={retry} title="send the last prompt again">
        <span class="gi" aria-hidden="true">{@html ICON.refresh}</span>
        retry
      </button>
    {/if}
    <button
      class="modelbtn"
      onclick={() => {
        effortOpen = false;
        settingsOpen = false;
        modelOpen = true;
        adding = false;
        keyIn = "";
        // Open on the row this session runs, not on whichever tab is first.
        const i = providers.findIndex((p) => p.active);
        provTab = i >= 0 ? i : 0;
        // Staged like the tools chooser: opening the dialog must not be able to
        // change the session, so the draft starts at what is already running.
        pickDraft = { provider: providers[provTab]?.id, model: activeProvider.model || "" };
        loadModels(providers[provTab]?.id);
      }}
    >
      {activeProvider.model || "model"}
      <span class="gi" aria-hidden="true">{@html ICON.down}</span>
    </button>
  </div>
</div>

<!-- The tools chooser. A preference the prompt states, not a switch the runtime
     obeys: the tool set is fixed when the session is created. Staged until Save,
     so closing the dialog is the same as changing your mind. -->
{#if toolsOpen}
  <div
    id="tools-modal"
    onclick={(e) => e.target === e.currentTarget && (toolsOpen = false)}
    role="presentation"
  >
    <div class="card tools">
      <div class="thead">
        <span class="ttitle">tools</span>
        <div class="tsearch" class:open={searchOpen}>
          {#if searchOpen}
            <input
              bind:value={toolsQuery}
              placeholder="search tools"
              aria-label="search tools"
              use:focusNow
              onkeydown={(e) => {
                if (e.key === "Escape") {
                  toolsQuery = "";
                  searchOpen = false;
                }
              }}
            />
            {#if toolsQuery}
              <button class="clearx" onclick={() => (toolsQuery = "")} aria-label="clear search">
                <span class="gi" aria-hidden="true">{@html ICON.x}</span>
              </button>
            {/if}
          {/if}
          <button
            class="ico"
            onclick={() => (searchOpen = !searchOpen)}
            title="search tools"
            aria-label="search tools"
            aria-expanded={searchOpen}
          >
            <span class="gi" aria-hidden="true">{@html ICON.search}</span>
          </button>
        </div>
      </div>
      <div class="tlist">
        {#if builtinShown.length}
          {@const onCount = builtinShown.filter(builtinOn).length}
          {@const closed = toolsClosed.includes("builtin")}
          <div class="tgroup">
            <div class="trow">
              <button
                class="tfold"
                onclick={() => toggleClosed("builtin")}
                aria-expanded={!closed}
                title="tools this build installs itself; unticking one asks the agent to prefer the others"
              >
                <span class="tcaret" aria-hidden="true"
                  >{@html closed ? ICON.right : ICON.down}</span
                >
                builtin
                <span class="dim">{onCount} of {builtinShown.length} on</span>
              </button>
              <button class="linky" onclick={() => allBuiltin(true)}>all</button>
              <button class="linky" onclick={() => allBuiltin(false)}>none</button>
            </div>
            {#if !closed}
              <div class="tchecks">
                {#each builtinShown as name (name)}
                  <label
                    class="tcheck"
                    title="a tool this build installs itself — unticking only tells the agent to prefer others"
                  >
                    <input
                      type="checkbox"
                      checked={builtinOn(name)}
                      onchange={() => toggleBuiltin(name)}
                    />
                    <span class="box" aria-hidden="true"></span>
                    <span>{name}</span>
                  </label>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
        {#each toolGroups as g (g.server)}
          {@const picked = g.tools.filter((t) => toolsDraft.includes(t.full)).length}
          {@const closed = toolsClosed.includes(g.server)}
          <div class="tgroup">
            <div class="trow">
              <button
                class="tfold"
                onclick={() => toggleClosed(g.server)}
                aria-expanded={!closed}
              >
                <span class="tcaret" aria-hidden="true"
                  >{@html closed ? ICON.right : ICON.down}</span
                >
                {g.server}
                <span class="dim">{picked} of {g.tools.length}</span>
              </button>
              <button class="linky" onclick={() => allOf(g.server, true)}>all</button>
              <button class="linky" onclick={() => allOf(g.server, false)}>none</button>
            </div>
            {#if !closed}
              <div class="tchecks">
                {#each g.tools as t (t.full)}
                  <label class="tcheck">
                    <input
                      type="checkbox"
                      checked={toolsDraft.includes(t.full)}
                      onchange={() => toggleDraft(t.full)}
                    />
                    <span class="box" aria-hidden="true"></span>
                    <span>{t.name}</span>
                  </label>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
        {#if !toolGroups.length && !builtinShown.length}
          <p class="none">no tool matches “{toolsQuery}”</p>
        {/if}
      </div>
      <div class="tfoot">
        <p class="note">
          Sent as a prompt instruction — the runtime's tool set is fixed when the session starts.
        </p>
        <div class="row">
          <span class="dim tcount"
            >{draftCount + builtinOnCount} of {toolRows.length + builtinAll.length} preferred</span
          >
          <button class="deny" onclick={() => (toolsOpen = false)}>Close</button>
          <button
            onclick={() => {
              toolsSel = toolsDraft;
              toolsOpen = false;
            }}
          >
            Save · {draftCount}
          </button>
        </div>
      </div>
    </div>
  </div>
{/if}

{#if modelOpen}
  <div
    id="modal"
    onclick={(e) => e.target === e.currentTarget && (modelOpen = false)}
    role="presentation"
  >
    <div class="card wide">
      <div class="tabswrap" class:left={tabsAt.left} class:right={tabsAt.right}>
        <button
          class="tabnav l"
          onclick={() => tabsScroll(-1)}
          aria-label="previous providers"
          tabindex={tabsAt.left ? 0 : -1}
          >{@html ICON.back}</button
        >
        <div
          class="tabs"
          use:measureTabs
          onscroll={tabsSync}
          onpointerdown={tabsDown}
          onclickcapture={(e) => {
            if (!tabSwallow) return;
            tabSwallow = false;
            e.stopPropagation();
            e.preventDefault();
          }}
        >
        {#each providers as p, i (p.id)}
          <button
            class="tab"
            class:on={i === provTab}
            title={p.description || p.id}
            onclick={() => {
              provTab = i;
              adding = false;
              keyIn = "";
              modelFilter = "";
              // Stage this row's own model, so Save always means "this provider,
              // the model it is configured to run" until you pick another.
              pickDraft = { provider: p.id, model: p.model || "" };
              loadModels(p.id);
            }}
          >
            <span class="mark" aria-hidden="true">{@html logoOf(p.id)}</span>
            {p.id}
            {#if p.active}
              <!-- The provider this session runs: a tick, not another key dot. -->
              <span class="cur" title="this session is running this provider" aria-label="running now"
                >{@html ICON.current}</span
              >
            {:else}
              <span
                class="kd"
                class:on={p.key_present || p.key_vault}
                title={p.key_present
                  ? "key in the environment"
                  : p.key_vault
                    ? "key in the vault"
                    : "no key yet"}
              ></span>
            {/if}
          </button>
        {/each}
        <button class="tab plus" onclick={() => (adding = true)} title="add a provider">+</button>
        </div>
        <button
          class="tabnav r"
          onclick={() => tabsScroll(1)}
          aria-label="more providers"
          tabindex={tabsAt.right ? 0 : -1}
          >{@html ICON.forward}</button
        >
      </div>
      {#if adding}
        <div class="form">
          <input bind:value={fresh.id} placeholder="id — e.g. openrouter" />
          <input bind:value={fresh.base_url} placeholder="base url — https://…/v1" />
          <input bind:value={fresh.model} placeholder="model id" />
          <input bind:value={fresh.key_env} placeholder="API key env var (optional)" />
          <div class="row">
            <button class="deny" onclick={() => (adding = false)}>Cancel</button>
            <button onclick={addProvider}>Save</button>
          </div>
        </div>
      {:else if providers[provTab]}
        {@const p = providers[provTab]}
        <dl class="facts">
          <dt>base url</dt>
          <dd>{p.base_url}</dd>
          <dt>model</dt>
          <dd>{p.model}{p.active ? " · running" : ""}</dd>
          <dt>max context</dt>
          <dd>{ctxText(ctxFor(p))}</dd>
          <dt>key</dt>
          <dd>
            {#if p.key_present}{p.key_env} · in the environment{:else if p.key_vault}vault ·
              encrypted, on this machine{:else}none yet{/if}
          </dd>
        </dl>
        <p class="note">{p.description || "no description for this provider"}</p>
        {#if p.off_peak}
          {@const op = p.off_peak}
          {@const now = offPeakNow(op)}
          <div class="offpeak">
            <div class="mhead">
              <span>off-peak window</span>
              <span class="dim">{now.utc} UTC</span>
            </div>
            <p class="op" class:peak={now.peak}>
              {now.peak ? "peak now — full price" : "off-peak now — half price"}
            </p>
            <dl class="facts">
              <dt>peak hours</dt>
              <dd>{op.peak.map(([a, b]) => `${pad2(a)}:00–${pad2(b)}:00`).join(" · ")} UTC</dd>
              <dt>peak days</dt>
              <dd>Monday to Friday</dd>
              <dt>discount</dt>
              <dd>{Math.round((1 - op.discount) * 100)}% off outside those hours</dd>
            </dl>
            <p class="note">
              {op.note}. <a href={op.docs} target="_blank" rel="noopener noreferrer">pricing</a>
            </p>
          </div>
        {/if}
        {#if p.id === "env"}
          <p class="note">
            This is the endpoint this session was started against. Pick a provider tab to store a
            key for it.
          </p>
        {:else}
          <div class="keyrow">
            <input
              type="password"
              bind:value={keyIn}
              placeholder={p.key_present || p.key_vault
                ? `paste a new ${p.id} key to replace the saved one`
                : `paste the ${p.id} API key — saved to the encrypted vault`}
              autocomplete="off"
              spellcheck="false"
              aria-label="{p.id} API key"
            />
            <button onclick={() => saveKey(p.id)} disabled={!keyIn.trim()}>
              {p.key_present || p.key_vault ? "replace" : "save key"}
            </button>
          </div>
          {#if p.key_present || p.key_vault}
            <p class="keysaved">
              <span class="gi" aria-hidden="true">{@html ICON.check}</span>
              key saved {p.key_present ? `in ${p.key_env}` : "in the vault, encrypted on this machine"}
            </p>
          {:else}
            <p class="nokey">
              <span class="gi" aria-hidden="true">{@html ICON.warn}</span>
              no key for {p.id} yet — the catalogue and this provider are unusable until there is one
            </p>
          {/if}
        {/if}
        <div class="mhead">
          <span>live models</span>
          <span class="dim">{models[p.id]?.length ?? 0}</span>
        </div>
        {#if modelsBusy === p.id}
          <p class="none">asking {p.id}…</p>
        {:else if modelsErr[p.id]}
          <p class="none">catalogue unavailable — {modelsErr[p.id]}</p>
        {:else if models[p.id]?.length}
          <input
            class="mfilter"
            bind:value={modelFilter}
            placeholder="filter {models[p.id].length} models"
            aria-label="filter models"
          />
          <div class="mlist">
            {#each models[p.id].filter(
              (m) => !modelFilter || m.id.toLowerCase().includes(modelFilter.toLowerCase()),
            ) as m (m.id)}
              <button
                class="m"
                class:on={pickDraft?.provider === p.id && pickDraft?.model === m.id}
                onclick={() => (pickDraft = { provider: p.id, model: m.id })}
                title="stage {m.id} — Save applies it"
              >
                <span>
                  {m.id}{#if m.reasoning === true}<span class="tag">reasoning</span>{/if}{#if p.active && p.model === m.id}<span class="tag on">running</span>{/if}
                </span>
                <span class="dim">
                  {switching === m.id
                    ? "switching…"
                    : [m.context ? ctxText(m.context) : "", rate(m.pricing)]
                        .filter(Boolean)
                        .join(" · ")}
                </span>
              </button>
            {/each}
          </div>
        {:else if !models[p.id]}
          <p class="none">no catalogue loaded</p>
        {:else}
          <p class="none">this provider returned no models</p>
        {/if}
        <p class="note">
          Prices are in / out per 1M tokens. Picking a model on this endpoint switches live and keeps
          the context; picking one on another provider's tab starts a new session. Where a vendor
          publishes no price of its own, the figure is OpenRouter's list price for the same model.
        </p>
        {@const d = stages()}
        <div class="tfoot">
          <p class="note">
            {#if d.model}
              <b>{d.model.id}</b> · {ctxText(d.model.context ?? d.row.context)} ·
              {d.rate || "price not published"} · {d.same
                ? "same endpoint, keeps this session's context"
                : "another provider, so Save starts a new session"}
            {/if}
          </p>
          <div class="row">
            <button class="deny" onclick={() => (modelOpen = false)}>Close</button>
            <button
              onclick={saveModel}
              disabled={!!switching || !pickDraft?.model || (d.same && d.model?.id === activeProvider.model)}
            >
              {switching ? "switching…" : `Save · ${d.model?.id ?? "pick a model"}`}
            </button>
          </div>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if effortOpen}
  <div
    id="modal"
    onclick={(e) => e.target === e.currentTarget && (effortOpen = false)}
    role="presentation"
  >
    <div class="card wide">
      <div class="mhead">
        <span>thinking effort</span>
        <span class="dim">{activeProvider.model || "—"}</span>
      </div>
      <div class="seg wide" role="group" aria-label="thinking effort">
        {#each EFFORTS as e (e)}
          <button class:on={effort === e} onclick={() => (effort = e)} title={EFFORT_HINT[e]}>
            <span class="gi" aria-hidden="true">{@html EFFORT_ICON[e]}</span>
            {e}
          </button>
        {/each}
      </div>
      <p class="note">{EFFORT_HINT[effort]}</p>
      <dl class="facts">
        <dt>provider</dt>
        <dd>{activeProvider.id || "—"}</dd>
        <dt>model</dt>
        <dd>{activeProvider.model || "—"}</dd>
        <dt>max context</dt>
        <dd>{ctxText(ctxOf)}</dd>
        <dt>price</dt>
        <dd>
          {price
            ? `${perM(Number(price.prompt) * 1e6)} in · ${perM(Number(price.completion) * 1e6)} out per 1M tokens${priceNote}`
            : "not published by this provider"}
        </dd>
      </dl>
      <p class="note">{activeProvider.description || "no description for this provider"}</p>
      {#if activeProvider.off_peak}
        {@const op = activeProvider.off_peak}
        {@const now = offPeakNow(op)}
        <div class="offpeak">
          <div class="mhead">
            <span>off-peak window</span>
            <span class="dim">{now.utc} UTC</span>
          </div>
          <p class="op" class:peak={now.peak}>
            {now.peak ? "peak now — full price" : "off-peak now — half price"}
          </p>
          <dl class="facts">
            <dt>peak hours</dt>
            <dd>{op.peak.map(([a, b]) => `${pad2(a)}:00–${pad2(b)}:00`).join(" · ")} UTC</dd>
            <dt>peak days</dt>
            <dd>Monday to Friday</dd>
            <dt>discount</dt>
            <dd>{Math.round((1 - op.discount) * 100)}% off outside those hours</dd>
          </dl>
          <p class="note">
            {op.note}. <a href={op.docs} target="_blank" rel="noopener noreferrer">pricing</a>
          </p>
        </div>
      {/if}
      <div class="row">
        <button class="deny" onclick={() => (effortOpen = false)}>Close</button>
        <button onclick={applyEffort} disabled={!!switching}>
          {switching ? "applying…" : `apply · ${effort}`}
        </button>
      </div>
    </div>
  </div>
{/if}

{#if settingsOpen}
  <div
    id="modal"
    onclick={(e) => e.target === e.currentTarget && (settingsOpen = false)}
    role="presentation"
  >
    <div class="card settings">
      <div class="stabs">
        {#each [["instructions", "Instructions"], ["chat", "Chat"], ["connection", "Connection"], ["storage", "Storage"], ["about", "About"]] as [k, label] (k)}
          <button class="tab" class:on={setTab === k} onclick={() => (setTab = k)}>{label}</button>
        {/each}
      </div>
      <div class="sbody">
        {#if setTab === "instructions"}
          <!-- The one setting the model has to be told rather than configured:
               the same lever as the effort and tool hints, sent as a preamble. -->
          <div class="form">
            <label class="sfld">
              <span>standing instructions</span>
              <textarea
                rows="6"
                maxlength="8192"
                bind:value={instructions}
                placeholder="e.g. answer in bullets; cite the url for every claim; never summarise a table you have not read"
              ></textarea>
              <small>Sent before every message. Kept in this browser — the server stores nothing.</small>
            </label>
            <label class="sfld">
              <span>reply language</span>
              <input maxlength="40" bind:value={language} placeholder="English" />
              <small>Becomes “reply in …”. Empty lets the model choose.</small>
            </label>
          </div>
        {:else if setTab === "chat"}
          <div class="form">
            <div class="sfld">
              <span>agent mode</span>
              <div class="seg">
                {#each MODES as m (m.id)}
                  <button class:on={mode === m.id} onclick={() => (mode = m.id)} title={m.hint}>{m.id}</button>
                {/each}
              </div>
              <small>{MODES.find((m) => m.id === mode)?.hint}</small>
            </div>
            <div class="sfld">
              <span>thinking effort</span>
              <div class="seg">
                {#each EFFORTS as e (e)}
                  <button class:on={effort === e} onclick={() => (effort = e)}>{e}</button>
                {/each}
              </div>
              <small>Sent as `reasoning_effort` when the provider takes one.</small>
            </div>
            <div class="sfld">
              <span>theme</span>
              <div class="seg">
                {#each THEMES as t (t.id)}
                  <button class:on={theme === t.id} onclick={() => (theme = t.id)} title={t.hint}>{t.label}</button>
                {/each}
              </div>
            </div>
            <label class="tcheck">
              <input type="checkbox" bind:checked={suggest} />
              <span class="box" aria-hidden="true"></span>
              <span>suggest next steps after a turn</span>
            </label>
          </div>
        {:else if setTab === "connection"}
          <dl class="facts">
            <dt>provider</dt><dd>{activeProvider.id || "—"}</dd>
            <dt>model</dt><dd>{activeProvider.model || "—"}</dd>
            <dt>base url</dt><dd>{activeProvider.base_url || "—"}</dd>
            <dt>agent mode</dt><dd>{mode}</dd>
            <dt>tools</dt><dd>{toolsSel.length + builtinOnCount} of {toolRows.length + builtinAll.length} preferred</dd>
          </dl>
        {:else if setTab === "storage"}
          <dl class="facts">
            <dt>conversations</dt><dd>{convos.length}</dd>
            <dt>database</dt><dd>WEBRAIN_CHAT_DB, else the webrain vault dir</dd>
            <dt>uploads</dt><dd>%TEMP%\webrain-chat\&lt;conversation&gt;\</dd>
            <dt>this page</dt><dd>settings live in localStorage; nothing here is sent to a server</dd>
          </dl>
        {:else}
          <dl class="facts">
            <dt>build</dt><dd>webrain chat --serve</dd>
            <dt>routes</dt>
            <dd>/events /turn /permission /answer /sessions /fork /catalog /search /attach /providers</dd>
            <dt>secrets</dt><dd>never stored — a provider row keeps the env var name</dd>
          </dl>
          <h3 class="sub-h">keys</h3>
          <dl class="facts">
            <dt>Enter</dt><dd>send · Shift+Enter a new line · Alt+Enter queue behind a running turn</dd>
            <dt>Esc</dt><dd>close a menu or dialog · discard a recording</dd>
            <dt>/ @ #</dt><dd>commands, skills, tools — the menus above the composer</dd>
            <dt>Ctrl K</dt><dd>search every stored message</dd>
          </dl>
          <button class="linky danger" onclick={resetSettings}>reset every setting</button>
        {/if}
      </div>
    </div>
  </div>
{/if}

{#if asksIn.length}
  <div id="modal">
    <div class="card" use:trap>
      <h2>
        webrain asks
        {#if asksIn.length > 1}<span class="queued">{asksIn.length - 1} more queued</span>{/if}
      </h2>
      <p class="q">{asksIn[0].question}</p>
      {#if asksIn[0].choices?.length}
        <div class="row choices">
          {#each asksIn[0].choices as c (c)}
            <button class="deny" onclick={() => answerAsk(c, false)}>{c}</button>
          {/each}
        </div>
      {/if}
      {#if asksIn[0].allow_freeform !== false}
        <div class="row">
          <input
            class="answer"
            bind:value={freeAnswer}
            placeholder="type an answer, Enter sends"
            onkeydown={(e) => {
              if (e.key === "Enter") answerAsk(freeAnswer, true);
            }}
          />
          <button onclick={() => answerAsk(freeAnswer, true)}>Send</button>
        </div>
      {/if}
    </div>
  </div>
{/if}

{#if asks.length}
  {@const ask = asks[0]}
  <div id="modal">
    <div class="card ask" use:trap role="dialog" aria-modal="true" aria-labelledby="asktitle">
      <div class="ask-head">
        <span class="seal" aria-hidden="true">{@html ICON.seal}</span>
        <div class="ask-id">
          <h2 id="asktitle">{ask.title || ask.tool}</h2>
          <p class="sub">
            <span class="mono">{ask.server || "mcp"}</span>
            <span class="pill" class:ro={ask.read_only}>
              {ask.read_only ? "read-only" : "changes state"}
            </span>
            {#if asks.length > 1}<span class="queued">{asks.length - 1} queued</span>{/if}
          </p>
        </div>
      </div>

      {#if ask.args && Object.keys(ask.args).length}
        <dl class="kv args">
          {#each Object.entries(ask.args) as [k, v] (k)}
            <dt>{k}</dt>
            <dd>{cell(v)}</dd>
          {/each}
        </dl>
      {:else}
        <p class="none">called with no arguments</p>
      {/if}

      <p class="fine">
        Approving runs this tool with exactly these arguments — the same authority
        as <code>--yes</code>.
      </p>

      <div class="row">
        <button class="deny" onclick={() => answer(false)}>Deny</button>
        <button onclick={() => answer(true)}>Approve once</button>
        <button
          class="ghost"
          onclick={() => {
            allowKind(ask.kind);
            answer(true);
          }}
          title="answer every {ask.kind} request in this tab without asking"
        >
          Always allow {ask.kind}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ── design system ─────────────────────────────────────────────────────
     One place defines a colour, a step, a radius. Everything below consumes
     these; no component invents a value. */
  /* ── Material 3 system ─────────────────────────────────────────────────
     A dark scheme seeded from webrain's mint, not M3's default violet: every
     neutral below is tinted with the primary hue, which is what makes an M3
     surface read as one material instead of grey boxes.
     Roles, not colours: components ask for `surface-container-high`, never
     `#1a231e`. Legacy names (--bg/--panel/--line/…) alias the roles so nothing
     below this block has to know the scheme changed. */
  :global(:root) {
    /* primary — the signal: live state, links, focus, the caret */
    --primary: #4edea3;
    --on-primary: #00382a;
    --primary-container: #00523f;
    --on-primary-container: #7bf8c6;
    /* surfaces — elevation in M3 is a tone, not a shadow */
    --surface: #0c1210;
    --surface-container-lowest: #080d0b;
    --surface-container-low: #101714;
    --surface-container: #141c18;
    --surface-container-high: #1a231e;
    --surface-container-highest: #212b25;
    --on-surface: #dfe7e2;
    --on-surface-variant: #9aa8a0;
    --outline: #6f7d75;
    --outline-variant: #232c27;
    /* secondary / tertiary */
    --secondary-container: #1f3a30;
    --on-secondary-container: #b8d9c9;
    --tertiary: #e8c268; /* warnings, queued work */
    --error: #ffb4a9;
    --on-error-container: #ffdad5;
    --error-container: #3d1512;
    /* state layers — M3 interaction feedback, applied as an overlay */
    --state-hover: #ffffff0f;
    --state-focus: #ffffff1a;
    --state-press: #ffffff24;

    /* legacy aliases: old rules pick up the new scheme for free */
    --bg: var(--surface);
    --panel: var(--surface-container);
    --line: var(--outline-variant);
    --rule: var(--outline-variant);
    --text: var(--on-surface);
    --dim: var(--on-surface-variant);
    --faint: #6f7d75;
    --accent: var(--primary);
    --warn: var(--tertiary);
    --bad: var(--error);

    /* shape scale */
    --shape-xs: 4px; --shape-sm: 8px; --shape-md: 12px; --shape-lg: 16px; --shape-full: 999px;
    --r1: var(--shape-sm); --r2: var(--shape-md);

    /* one chevron for every collapsed/expanded control */
    --chev: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%23000' stroke-width='3' stroke-linecap='round' stroke-linejoin='round'%3E%3Cpath d='M9 5l7 7-7 7'/%3E%3C/svg%3E");

    /* motion: two durations and one emphasised curve, nothing ad hoc */
    --motion-short: 150ms;
    --motion-medium: 250ms;
    --fast: var(--motion-short);
    --ease: cubic-bezier(0.2, 0, 0, 1);

    /* Spacing rhythm: one scale, ~25% more air than a dense terminal would take.
       Luxury here is space and target size, not decoration. */
    --s1: 5px; --s2: 10px; --s3: 14px; --s4: 20px; --s5: 26px; --s6: 32px;

    /* Three type roles, and no webfonts: the page makes zero network requests,
       which is worth more than a downloaded face. `--display` is a serif and is
       used twice — the wordmark and the empty state. Everything structural is
       `--mono`, because this is a log, not an article. Body text is the OS UI
       face, which is the one thing that must stay comfortable for paragraphs. */
    --mono: ui-monospace, "Cascadia Mono", "Segoe UI Mono", "SF Mono", Menlo, Consolas, monospace;
    --display: "Iowan Old Style", "Palatino Linotype", Palatino, "Book Antiqua", Georgia, serif;
    --scroll-thumb: #2a3630;
    color-scheme: dark;
  }

  /* ── palettes ──────────────────────────────────────────────────────────
     A theme overrides the role map and nothing else — no component below this
     point knows a theme exists. `mint` is the :root block above, so it costs no
     rule at all; the three below are the ones people already read code in. */
  :global(:root[data-theme="dark"]) {
    --primary: #4daafc; --on-primary: #08213a;
    --primary-container: #0e4a73; --on-primary-container: #b8ddfb;
    --surface: #1e1e1e;
    --surface-container-lowest: #161617;
    --surface-container-low: #252526;
    --surface-container: #2a2a2b;
    --surface-container-high: #333333;
    --surface-container-highest: #3c3c3c;
    --on-surface: #d4d4d4; --on-surface-variant: #9d9d9d;
    --outline: #6e6e6e; --outline-variant: #3c3c3c;
    --secondary-container: #1f3347; --on-secondary-container: #b8d4ea;
    --tertiary: #d7ba7d; --error: #f48771;
    --on-error-container: #ffd7cf; --error-container: #5a1d1d;
    --faint: #6e6e6e; --scroll-thumb: #424242;
  }
  :global(:root[data-theme="monokai"]) {
    --primary: #a6e22e; --on-primary: #1f2400;
    --primary-container: #3c4a10; --on-primary-container: #d7f59a;
    --surface: #272822;
    --surface-container-lowest: #1e1f1c;
    --surface-container-low: #2e2f29;
    --surface-container: #33342d;
    --surface-container-high: #3b3c34;
    --surface-container-highest: #44453c;
    --on-surface: #f8f8f2; --on-surface-variant: #b6b8a8;
    --outline: #7c7f6d; --outline-variant: #3b3c34;
    --secondary-container: #4a2340; --on-secondary-container: #f9a8c4;
    --tertiary: #fd971f; --error: #ff6188;
    --on-error-container: #ffd6e2; --error-container: #4d1024;
    --faint: #7c7f6d; --scroll-thumb: #4a4b41;
  }
  :global(:root[data-theme="light"]) {
    --primary: #0060b0; --on-primary: #ffffff;
    --primary-container: #cfe6fb; --on-primary-container: #062d4d;
    --surface: #ffffff;
    --surface-container-lowest: #f4f4f5;
    --surface-container-low: #f7f7f8;
    --surface-container: #f3f3f4;
    --surface-container-high: #ebebec;
    --surface-container-highest: #e2e2e4;
    --on-surface: #2b2b2b; --on-surface-variant: #5f5f63;
    --outline: #8e8e93; --outline-variant: #dcdcdf;
    --secondary-container: #dbe9f7; --on-secondary-container: #123a5c;
    --tertiary: #8a5300; --error: #b3261e;
    --on-error-container: #410e0b; --error-container: #fbe2e0;
    /* the M3 state layers are white alpha; on white they have to be black */
    --state-hover: #0000000f; --state-focus: #0000001a; --state-press: #00000024;
    --faint: #77777c; --scroll-thumb: #c9c9cd;
    color-scheme: light;
  }
  :global(*) { box-sizing: border-box; }
  :global(body) { margin: 0; background: var(--bg); color: var(--text); }
  :global(body, #app) { height: 100vh; }
  :global(#app) { display: grid; grid-template-rows: auto 1fr; overflow: hidden;
    position: relative; }
  :global(#app) {
    font: 15px/1.65 ui-sans-serif, "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif;
  }
  :global(:focus-visible) { outline: 2px solid var(--primary); outline-offset: 2px; }
  :global(::selection) { background: var(--primary-container); color: var(--on-surface); }

  /* Slim scrollbars: the default trough on a near-black page is the one piece of
     chrome that looked foreign. The thumb is a token so the light theme does not
     inherit a dark one. */
  :global(*) { scrollbar-width: thin; scrollbar-color: var(--scroll-thumb) transparent; }
  :global(::-webkit-scrollbar) { width: 10px; height: 10px; }
  :global(::-webkit-scrollbar-track) { background: transparent; }
  :global(::-webkit-scrollbar-thumb) { background: var(--scroll-thumb); border-radius: 8px; border: 2px solid var(--bg); }
  :global(::-webkit-scrollbar-thumb:hover) { background: var(--outline); }
  :global(::-webkit-scrollbar-corner) { background: transparent; }

  /* ── top app bar ───────────────────────────────────────────────────────
     One 44px strip: wordmark, a state pill, two live numbers. The rest of the
     instrumentation moved to the right rail, where it can be read without a
     header growing and re-flowing the transcript under it. */
  .topbar { display: flex; align-items: center; gap: var(--s2); height: 44px;
    padding: 0 var(--s4); background: var(--surface-container-low);
    border-bottom: 1px solid var(--outline-variant); }
  .wordmark { font-family: var(--display); font-size: 15px; color: var(--on-surface);
    margin-right: var(--s2); white-space: nowrap; }
  .wordmark i { font-style: normal; color: var(--primary); margin-left: 0.4em;
    font-family: var(--mono); font-size: 10px; text-transform: uppercase; letter-spacing: 0.24em; }
  .grow { flex: 1; }
  .state { font-family: var(--mono); font-size: 10px; text-transform: uppercase;
    letter-spacing: 0.16em; color: var(--on-surface-variant); border-radius: var(--shape-full);
    padding: 2px var(--s2); background: var(--surface-container-high); white-space: nowrap; }
  .state.running { color: var(--on-secondary-container); background: var(--secondary-container); }
  .state.error, .state.ended { color: var(--error); background: var(--error-container); }
  .meter-inline { display: inline-flex; align-items: baseline; gap: 3px; font-family: var(--mono);
    font-size: 11px; color: var(--on-surface); font-variant-numeric: tabular-nums; }
  .meter-inline u { text-decoration: none; color: var(--faint); font-size: 9px;
    text-transform: uppercase; letter-spacing: 0.12em; }
  .icon { background: none; border: 0; color: var(--on-surface-variant); border-radius: var(--shape-full);
    width: 2rem; height: 2rem; min-height: 0; padding: 0; font: inherit; font-size: 14px; line-height: 1;
    display: grid; place-items: center; cursor: pointer; flex: 0 0 auto;
    transition: background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  .icon:hover { background: var(--state-hover); color: var(--primary); }

  /* ── the deck: rail | transcript | instruments ──────────────────────────
     Three columns, one scroller. The rails are navigation and instrumentation
     — context for the thing you are actually doing, which is reading. */
  .deck { position: relative; display: grid;
    grid-template-columns: var(--rail-w, 16rem) minmax(0, 1fr) var(--inst-w, 15rem);
    min-height: 0; overflow: hidden; }
  .deck:not(.open) { grid-template-columns: minmax(0, 1fr) var(--inst-w, 15rem); }
  .deck:not(.open) .rail { display: none; }

  /* The rails are as wide as you drag them. One number per side, held in a CSS
     var so the grid, the composer's offset and the handles cannot disagree. */
  .grip { position: absolute; top: 0; bottom: 0; width: 9px; z-index: 6;
    cursor: col-resize; touch-action: none; }
  .grip::after { content: ""; position: absolute; top: 0; bottom: 0; left: 4px; width: 1px;
    background: transparent; transition: background var(--motion-short) var(--ease); }
  .grip:hover::after, .grip:focus-visible::after, .grip:active::after { background: var(--primary); }
  .grip:focus-visible { outline: none; }
  .grip.l { left: calc(var(--rail-w, 16rem) - 4px); }
  .grip.r { right: calc(var(--inst-w, 15rem) - 4px); }
  .deck:not(.open) .grip.l { display: none; }

  main { position: relative; overflow-y: auto; min-height: 0; scrollbar-gutter: stable;
    /* The field floats over the bottom of this column, so the transcript has to
       end above it — the height is measured, not guessed. */
    padding: var(--s2) clamp(1rem, 4vw, 3rem) calc(var(--composer-h, 8rem) + var(--s3)) 3.4rem;
    background: var(--surface); }

  /* The spine. One hairline the whole transcript hangs off: every row hangs a
     tick on it, a failed tool turns its tick red, and the ticks are the only
     thing on the page that is not text. This is the anchor — cover the wordmark
     and the spine still says "a log you are watching, not a chat you are in". */
  main::before { content: ""; position: absolute; top: 0; bottom: 0; left: 2.3rem;
    width: 1px; background: var(--outline-variant); }

  .rail { order: -1; display: flex; flex-direction: column; overflow-y: auto;
    background: var(--surface-container-low); padding: var(--s3) var(--s2);
    border-right: 1px solid var(--outline-variant); font-size: 12px; }
  .inst { overflow-y: auto; background: var(--surface-container-low);
    padding: var(--s3); border-left: 1px solid var(--outline-variant);
    font-family: var(--mono); font-size: 11px; }
  .side-h { color: var(--faint); font-family: var(--mono); font-size: 9px; font-weight: 600;
    letter-spacing: 0.18em; text-transform: uppercase; margin: var(--s4) 0 var(--s2); }
  .side-h:first-child { margin-top: 0; }
  .none { color: var(--faint); margin: 0 0 var(--s2); }

  /* A readout, not a card: label left, value right, tabular numerals. */
  .readout { display: grid; grid-template-columns: auto 1fr; gap: 3px var(--s2); margin: 0; }
  .readout :global(dt) { color: var(--faint); text-transform: uppercase; font-size: 9px; letter-spacing: 0.12em; }
  .readout :global(dd) { margin: 0; text-align: right; font-variant-numeric: tabular-nums;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* List items: shape-medium, hover = state layer, selected = secondary container. */
  .convo { display: grid; gap: 1px; width: 100%; text-align: left; font: inherit; cursor: pointer;
    background: none; color: var(--on-surface); border: 0; border-radius: var(--shape-md);
    padding: var(--s2) var(--s3); margin-bottom: 2px;
    transition: background var(--motion-short) var(--ease); }
  .convo:hover { background: var(--state-hover); }
  /* Selected is a tone and a tick, not a mint pill: a rail full of green pills
     reads as buttons, and this list is a list. */
  .convo.on { background: var(--surface-container-high); color: var(--on-surface);
    box-shadow: inset 2px 0 0 var(--primary); }
  .convo.reading { background: var(--surface-container); }
  .convo .title { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .convo .meta { color: var(--faint); font-size: 11px; font-family: var(--mono);
    white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .turn { background: var(--surface-container); border-radius: var(--shape-sm);
    padding: var(--s2) var(--s3); margin-bottom: var(--s2); }
  .turn .n { color: var(--faint); display: flex; gap: var(--s2); align-items: baseline; }
  .turn .n span { color: var(--primary); font-variant-numeric: tabular-nums; }
  .turn dl { margin: var(--s1) 0 0; display: grid; grid-template-columns: auto 1fr; gap: 1px var(--s2); }
  .turn :global(dt) { color: var(--faint); }
  .turn :global(dd) { margin: 0; text-align: right; font-variant-numeric: tabular-nums; }

  .banner { display: flex; align-items: center; gap: var(--s2); border-radius: var(--shape-sm);
    background: var(--surface-container-high); color: var(--tertiary); padding: var(--s2) var(--s3);
    margin-bottom: var(--s3); font-size: 12px; font-family: var(--mono); }
  .banner button { background: none; border: 0; color: var(--tertiary); border-radius: var(--shape-full);
    font: inherit; font-size: 11px; padding: 0 var(--s2); min-height: 1.6rem; cursor: pointer; }

  /* ── transcript rows ───────────────────────────────────────────────────
     A rule separates, the tick on the spine identifies, and the role is the
     only thing that changes colour. Nothing floats and nothing is boxed: on a
     page that streams, a card per message is a wall of boxes. */
  .msg, .tool, .act, .think, .usage, .note, .steps, .evidence, .next { position: relative; }
  .msg { border-top: 1px solid var(--rule); padding: var(--s4) 0; margin: 0;
    white-space: pre-wrap; word-break: break-word; animation: rise var(--motion-short) var(--ease); }
  .msg::after { content: ""; position: absolute; left: -1.32rem; top: calc(var(--s4) + 3px);
    width: 7px; height: 7px; border-radius: 50%; background: var(--outline);
    box-shadow: 0 0 0 3px var(--surface); }
  /* Your turn is a block, the answer is the page (VS Code's own changelog found
     readers could not tell the two apart). The negative margin keeps the text on
     the same left edge as the answer, so the spine still lines up. */
  .msg.user { background: var(--surface-container-low); border-radius: var(--shape-sm);
    padding: var(--s4) var(--s3); margin-inline: calc(-1 * var(--s3)); }
  .msg.user::after { left: calc(-1.32rem + var(--s3)); }
  .msg.assistant::after { background: var(--primary); }
  .msg.user::after { background: var(--on-surface-variant); }
  .msg.err { color: var(--error); }
  .msg.err::after { background: var(--error); }
  .msg .md { white-space: normal; }
  .who { display: flex; align-items: center; gap: var(--s2); color: var(--faint);
    font-family: var(--mono); font-size: 9px; text-transform: uppercase;
    letter-spacing: 0.2em; margin-bottom: var(--s2); }
  .msg.assistant .who { color: var(--primary); }
  .linky { background: none; border: 0; color: var(--faint); font: inherit; font-size: 10px;
    letter-spacing: 0.08em; padding: 0 var(--s1); border-radius: var(--shape-xs); cursor: pointer;
    min-height: 0; text-transform: uppercase; }
  .linky:hover { color: var(--primary); background: var(--state-hover); }
  .copy { float: right; background: none; border: 0; color: var(--faint); border-radius: var(--shape-full);
    padding: 0 var(--s2); font: inherit; font-size: 10px; text-transform: uppercase;
    letter-spacing: 0.12em; cursor: pointer; min-height: 0;
    transition: color var(--motion-short) var(--ease); }
  .copy:hover { color: var(--primary); background: var(--state-hover); }
  /* What you attached, in the message rather than in the composer: a thumbnail
     that is also the download. The link is wrapped as a block so the caption
     sits under the picture at the same width. */
  .shots { display: flex; flex-wrap: wrap; gap: var(--s2); margin: 0 0 var(--s2); }
  .shot { display: grid; gap: 2px; justify-items: start; max-width: 15rem;
    text-decoration: none; color: var(--faint); }
  .shot img { display: block; max-width: 15rem; max-height: 12rem; border-radius: var(--shape-sm);
    border: 1px solid var(--outline-variant); background: var(--surface-container-high); }
  .shot:hover img { border-color: var(--primary); }
  .shot .cap { font-family: var(--mono); font-size: 9px; letter-spacing: 0.04em;
    max-width: 15rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .shot .doc { display: grid; place-items: center; min-width: 7rem; min-height: 4rem;
    padding: var(--s2); border-radius: var(--shape-sm); border: 1px solid var(--outline-variant);
    background: var(--surface-container-high); font-family: var(--mono); font-size: 10px;
    text-transform: uppercase; letter-spacing: 0.1em; color: var(--on-surface-variant); }
  .shot:hover .doc { border-color: var(--primary); color: var(--primary); }

  /* markdown */
  .md :global(p) { margin: 0 0 var(--s2); }
  .md :global(p:last-child) { margin-bottom: 0; }
  .md :global(h1), .md :global(h2), .md :global(h3) { font-family: var(--display);
    font-size: 1.02em; margin: var(--s4) 0 var(--s2); letter-spacing: -0.01em; font-weight: 600; }
  .md :global(h1) { font-size: 1.16em; }
  .md :global(ul), .md :global(ol) { margin: 0 0 var(--s2); padding-left: 1.4em; }
  /* A scraped tool name or a url is one unbreakable token; without this a long
     one pushed the whole transcript into horizontal scroll. */
  .md :global(li), .md :global(p), .md :global(td) { overflow-wrap: anywhere; }
  .md :global(code) { overflow-wrap: anywhere; }
  .md :global(li) { margin: 2px 0; }
  /* A list inside a list is the one place markdown indents itself, so it needs no
     extra left padding — only its own spacing, or the nested rows float. */
  .md :global(li) > :global(ul), .md :global(li) > :global(ol) { margin: var(--s1) 0 0; }
  .md :global(li) > :global(p) { margin: 0 0 var(--s1); }
  .md :global(li::marker) { color: var(--faint); }
  .md :global(a) { color: var(--primary); text-underline-offset: 3px;
    text-decoration-color: var(--outline); }
  .md :global(a:hover) { text-decoration-color: var(--primary); }
  .md :global(code) { background: var(--surface-container-highest); border-radius: var(--shape-xs);
    padding: 1px 5px; font-family: var(--mono); font-size: 0.9em; }
  .md :global(pre) { background: var(--surface-container-highest); border-radius: var(--shape-sm);
    padding: var(--s3); overflow-x: auto; margin: 0 0 var(--s2); white-space: pre;
    font-family: var(--mono); font-size: 12.5px; }
  .md :global(pre code) { background: none; padding: 0; }
  .md :global(.code) { border-radius: var(--shape-sm); margin: 0 0 var(--s2); overflow: hidden;
    background: var(--surface-container-highest); }
  .md :global(.code .bar) { display: flex; align-items: center; justify-content: space-between;
    padding: 2px var(--s3); background: var(--surface-container); color: var(--on-surface-variant);
    font-family: var(--mono); font-size: 10px; text-transform: uppercase; letter-spacing: 0.14em; }
  .md :global(.code-copy) { background: none; border: 0; color: var(--on-surface-variant);
    border-radius: var(--shape-full); font: inherit; font-size: 10px; padding: 0 var(--s2);
    min-height: 0; cursor: pointer; }
  .md :global(.code-copy:hover) { color: var(--primary); }
  .md :global(.code pre) { margin: 0; border-radius: 0; background: none; }
  .md :global(blockquote) { margin: 0 0 var(--s2); padding-left: var(--s3);
    border-left: 2px solid var(--primary-container); color: var(--dim); }
  /* An image in a reply is content, not an icon: full width of the column up to
     its own size, rounded, and it opens on click (the delegated listener below
     turns a click on one into a new tab). */
  .md :global(img) { display: block; max-width: 100%; height: auto; margin: var(--s2) 0;
    border-radius: var(--shape-sm); border: 1px solid var(--outline-variant);
    background: var(--surface-container-high); cursor: zoom-in; }
  .md :global(img:hover) { border-color: var(--primary); }
  .md :global(a img) { border-color: transparent; }
  .md :global(figure) { margin: 0 0 var(--s2); }
  .md :global(figcaption) { color: var(--faint); font-family: var(--mono); font-size: 10px;
    margin-top: 2px; }
  /* A destination that stood alone on its line is a card: the host named, the
     path behind it, one click out. Inline links stay links — a card mid-sentence
     would break the paragraph it sits in. */
  .md :global(a.card) { display: inline-flex; align-items: baseline; gap: var(--s2);
    max-width: 100%; margin: var(--s1) 0; padding: var(--s1) var(--s3);
    background: var(--surface-container-low); border: 1px solid var(--outline-variant);
    border-radius: var(--shape-full); text-decoration: none; font-family: var(--mono);
    font-size: 11.5px; }
  .md :global(a.card:hover) { border-color: var(--primary); }
  .md :global(a.card b) { color: var(--primary); font-weight: 500; flex: none; }
  .md :global(a.card span) { color: var(--faint); overflow: hidden; text-overflow: ellipsis;
    white-space: nowrap; }
  /* A wide table scrolls inside its own box: the transcript itself must never
     gain a horizontal scrollbar over one column too many. A tall one is capped,
     so a 60-row dump cannot push the answer out of reach. */
  .md :global(.table-wrap) { max-height: 24rem; overflow: auto; margin: 0 0 var(--s2);
    border-radius: var(--shape-sm); }
  .md :global(.table-wrap) :global(table) { display: table; overflow: visible; margin: 0; }
  .md :global(table) { border-collapse: collapse; margin: 0 0 var(--s2); font-size: 0.95em;
    display: block; max-width: 100%; overflow-x: auto; }
  .md :global(th), .md :global(td) { border-bottom: 1px solid var(--rule); padding: 3px var(--s2);
    text-align: left; }
  .md :global(th) { color: var(--faint); font-family: var(--mono); font-size: 10px;
    text-transform: uppercase; letter-spacing: 0.1em; position: sticky; top: 0;
    background: var(--surface); white-space: nowrap; }
  .md :global(tbody tr:nth-child(even)) { background: var(--surface-container-low); }
  .md :global(hr) { border: 0; border-top: 1px solid var(--rule); margin: var(--s4) 0; }

  /* A floating action for "take me back to the live edge" — M3 elevation level 2. */
  .jump { position: absolute; left: 50%; transform: translateX(-50%);
    bottom: calc(var(--composer-h, 8rem) + var(--s2)); z-index: 10;
    background: var(--surface-container-highest); color: var(--on-surface); border: 0;
    border-radius: var(--shape-full); padding: var(--s2) var(--s4); font: inherit; font-size: 12px;
    cursor: pointer; box-shadow: 0 2px 6px 2px #0006, 0 1px 2px #0008;
    animation: rise var(--motion-short) var(--ease); }
  .jump:hover { color: var(--primary); }

  /* Material: every button presses with a ripple. It is the one interaction
     animation in the page, and it is the one that carries meaning. The span and
     its animation are drawn from `ripple()`; only the containing box is CSS. */
  button { position: relative; overflow: hidden; }

  /* ── the dock ──────────────────────────────────────────────────────────
     The row under the field is where a turn is shaped: attach, record, mode,
     effort, tools. Ordered by how early the choice applies — the model sits far
     right because it is the environment, not this turn. */
  .dock { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2);
    padding: var(--s2) clamp(1rem, 4vw, 3rem) var(--s3); }
  .dock .modelbtn { margin-left: auto; }
  .ico { flex: 0 0 auto; width: 2.2rem; min-height: 2.2rem; padding: 0; display: grid; place-items: center;
    background: var(--surface-container); color: var(--on-surface-variant); border: 0;
    border-radius: var(--shape-full); font: inherit; font-size: 15px; line-height: 1; cursor: pointer;
    transition: background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  .ico:hover { background: var(--surface-container-high); color: var(--primary); }
  .ico.rec { color: var(--error); }
  .ico.rec.live { width: auto; gap: var(--s1); padding: 0 var(--s2) 0 var(--s3);
    background: var(--error-container); color: var(--error); display: flex; }
  .rec-ms { font-family: var(--mono); font-size: 11px; font-variant-numeric: tabular-nums; }
  .rec-stop { width: 8px; height: 8px; border-radius: 2px; background: var(--error);
    animation: pulse 1.2s ease-in-out infinite; }
  /* One control for one choice. The dock has room for a button, not a segmented
     row, and the menu is the only place with space to say what each mode means. */
  .modepick { position: relative; }
  .modemenu { position: absolute; bottom: calc(100% + var(--s2)); left: 0; z-index: 20;
    display: grid; gap: 2px; padding: var(--s1); min-width: 15rem;
    background: var(--surface-container-high); border: 1px solid var(--outline-variant);
    border-radius: var(--shape-md); box-shadow: 0 12px 32px rgb(0 0 0 / 0.45);
    animation: rise var(--motion-short) var(--ease); }
  .modemenu button { display: grid; grid-template-columns: 1.15rem 1fr; gap: 0 var(--s2);
    align-items: center; background: none; border: 0; color: var(--on-surface-variant); font: inherit;
    text-align: left; padding: var(--s2); border-radius: var(--shape-xs); cursor: pointer;
    transition: background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  .modemenu button:hover { background: var(--state-hover); color: var(--on-surface); }
  .modemenu button.on { color: var(--primary); background: var(--surface-container-highest); }
  .modemenu .gi { grid-row: 1; align-self: center; }
  .modemenu .gi :global(svg) { width: 14px; height: 14px; }
  .modemenu b { font-weight: 500; font-size: 12px; letter-spacing: 0.01em; }
  .modemenu small { grid-column: 2; color: var(--faint); font-size: 11px; line-height: 1.3; }
  /* The rail's row menu is the same list opening the other way: downward from a
     26px trigger, right-aligned, and narrow enough to fit a 235px rail. */
  .moremenu { top: calc(100% + 4px); bottom: auto; left: auto; right: 0;
    min-width: 9.5rem; z-index: 30; animation: pop var(--motion-short) var(--ease); }
  .moremenu.up { top: auto; bottom: calc(100% + 4px); }
  .moremenu button { padding: var(--s1) var(--s2); }
  .moremenu button.danger:hover { color: var(--error); }

  /* The theme menu is the mode menu pinned to the top bar: same rows, opening
     downward, right-aligned to the button that opens it. */
  .themepick { position: relative; display: flex; }
  .thememenu { position: absolute; top: calc(100% + var(--s2)); right: 0; z-index: 30;
    display: grid; gap: 2px; padding: var(--s1); min-width: 15rem;
    background: var(--surface-container-high); border: 1px solid var(--outline-variant);
    border-radius: var(--shape-md); box-shadow: 0 12px 32px rgb(0 0 0 / 0.45);
    animation: pop var(--motion-short) var(--ease); }
  .thememenu button { display: grid; grid-template-columns: 1.15rem 1fr; gap: 0 var(--s2);
    align-items: center; background: none; border: 0; color: var(--on-surface-variant); font: inherit;
    text-align: left; padding: var(--s2); border-radius: var(--shape-xs); cursor: pointer;
    transition: background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  .thememenu button:hover { background: var(--state-hover); color: var(--on-surface); }
  .thememenu button.on { color: var(--primary); background: var(--surface-container-highest); }
  .thememenu b { font-weight: 500; font-size: 12px; letter-spacing: 0.01em; }
  .thememenu small { grid-column: 2; color: var(--faint); font-size: 11px; line-height: 1.3; }
  /* Three dots of the theme's own surface / primary / tertiary. */
  .swatch { grid-row: 1; display: grid; grid-template-columns: repeat(3, 5px); gap: 1px; }
  .swatch i { width: 5px; height: 1.15rem; border-radius: 1px; }
  .sw-mint i:nth-child(1) { background: #0c1210; } .sw-mint i:nth-child(2) { background: #4edea3; }
  .sw-mint i:nth-child(3) { background: #e8c268; }
  .sw-dark i:nth-child(1) { background: #1e1e1e; } .sw-dark i:nth-child(2) { background: #4daafc; }
  .sw-dark i:nth-child(3) { background: #d7ba7d; }
  .sw-monokai i:nth-child(1) { background: #272822; } .sw-monokai i:nth-child(2) { background: #a6e22e; }
  .sw-monokai i:nth-child(3) { background: #ff6188; }
  .sw-light i:nth-child(1) { background: #ffffff; border: 1px solid #dcdcdf; } .sw-light i:nth-child(2) { background: #0060b0; }
  .sw-light i:nth-child(3) { background: #8a5300; }

  .seg { display: inline-flex; gap: 2px; padding: 2px; background: var(--surface-container);
    border-radius: var(--shape-full); flex: 0 0 auto; }
  .seg button { background: none; border: 0; color: var(--on-surface-variant); font: inherit;
    font-size: 12px; padding: 0 var(--s3); min-height: 1.8rem; border-radius: var(--shape-full);
    cursor: pointer; transition: background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  .seg button:hover { background: var(--state-hover); }
  .seg button.on { background: var(--primary); color: var(--on-primary); }
  /* In a dialog the same control is a full-width row of equal choices. */
  .seg.wide { display: flex; width: 100%; margin-bottom: var(--s2); }
  .seg.wide button { flex: 1; display: inline-flex; align-items: center; justify-content: center;
    gap: var(--s1); font-family: var(--mono); }
  .modelbtn { background: var(--surface-container); border: 0; color: var(--on-surface-variant);
    border-radius: var(--shape-full); font: inherit; font-family: var(--mono); font-size: 11px;
    padding: 0 var(--s3); min-height: 2.2rem; min-width: 0; flex: 0 1 auto; overflow: hidden;
    text-overflow: ellipsis; white-space: nowrap; cursor: pointer;
    transition: background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  .modelbtn:hover { background: var(--surface-container-high); color: var(--primary); }
  .modelbtn .gi { margin-left: var(--s1); }

  /* Every control icon is an inline SVG in a `.gi` box, never a font character:
     glyph coverage differs per OS and some of them simply do not exist. */
  .gi { display: inline-grid; place-items: center; flex: 0 0 auto; line-height: 0; }
  .gi :global(svg) { width: 15px; height: 15px; display: block; }
  .mini .gi :global(svg), .clearx .gi :global(svg), .clearq .gi :global(svg),
  .jump .gi :global(svg), .tcaret :global(svg) { width: 12px; height: 12px; }

  /* The tools chooser. A list, so it is a dialog with a fold per server, a
     search that opens on demand, and one Save — not a strip of chips that
     rewrites the setting on every stray click. */
  .card.tools { width: min(38rem, 92vw); display: grid; gap: 0; padding: 0; }
  .thead { display: flex; align-items: center; gap: var(--s2); padding: var(--s3) var(--s3) var(--s2); }
  .ttitle { font-family: var(--mono); font-size: 9px; text-transform: uppercase;
    letter-spacing: 0.16em; color: var(--faint); }
  .tsearch { margin-left: auto; display: flex; align-items: center; gap: var(--s1); }
  /* The field grows out of the icon instead of appearing: width, fill and the
     caret's own fade on one curve, so it reads as one motion. The colours are
     set here because a bare <input> inherits the browser's white fill. */
  .tsearch input { width: 0; min-width: 0; opacity: 0; padding: 0; border: 1px solid transparent;
    border-radius: var(--shape-full); background: var(--surface-container-highest);
    color: var(--on-surface); font: inherit; font-size: 13px; caret-color: var(--primary);
    transition: width 260ms var(--ease), opacity 160ms var(--ease), padding 260ms var(--ease),
      border-color 200ms var(--ease); }
  .tsearch input::placeholder { color: var(--faint); }
  /* Open, it takes the whole row: the field is the only thing you are reading
     at that moment, and 15rem of it in a 38rem dialog is a postage stamp. */
  .tsearch.open { flex: 1; }
  .tsearch.open input { width: 100%; opacity: 1; padding: 0 var(--s3);
    min-height: 2.2rem; border-color: var(--outline-variant); }
  .tsearch.open input:focus { outline: none; border-color: var(--primary); }
  .tsearch .ico { transition: color var(--motion-short) var(--ease),
    background var(--motion-short) var(--ease), transform 260ms var(--ease); }
  .tsearch.open .ico { transform: scale(0.92); color: var(--primary); }
  .clearx { background: none; border: 0; color: var(--faint); cursor: pointer; font: inherit;
    font-size: 11px; padding: 0 var(--s1); }
  .clearx:hover { color: var(--on-surface); }
  .tlist { max-height: min(58vh, 30rem); overflow-y: auto; padding: 0 var(--s2) var(--s2); }
  .tgroup { border-top: 1px solid var(--outline-variant); }
  .trow { display: flex; align-items: center; gap: var(--s2); padding: var(--s1) var(--s1) var(--s1) 0; }
  .tfold { background: none; border: 0; color: var(--on-surface); font: inherit; font-size: 12px;
    display: flex; align-items: center; gap: var(--s2); flex: 1; cursor: pointer; text-align: left;
    padding: var(--s1) 0; }
  .tcaret { color: var(--primary); width: 1rem; display: grid; place-items: center; flex: 0 0 auto; }
  .tchecks { display: grid; grid-template-columns: repeat(auto-fill, minmax(11rem, 1fr));
    gap: var(--s1) var(--s2); padding: 0 0 var(--s2) calc(1rem + var(--s2)); }
  .tcheck { position: relative; display: flex; align-items: center; gap: var(--s2);
    font-family: var(--mono); font-size: 11px; color: var(--on-surface-variant); cursor: pointer;
    min-height: 2rem; animation: rise 160ms var(--ease); }
  .tcheck:hover { color: var(--on-surface); }
  /* The real input stays in the tab order and the a11y tree; the box beside it is
     the picture. A drawn box beats `appearance: none` on the input itself because
     the tick can then animate on its own curve. */
  .tcheck input { position: absolute; width: 1px; height: 1px; opacity: 0; margin: 0;
    pointer-events: none; }
  .tcheck .box { flex: 0 0 auto; width: 1.05rem; height: 1.05rem; border-radius: 6px;
    border: 1.5px solid var(--outline); background: var(--surface-container);
    display: grid; place-items: center;
    transition: background 170ms var(--ease), border-color 170ms var(--ease),
      box-shadow 200ms var(--ease); }
  .tcheck .box::after { content: ""; width: 0.52rem; height: 0.28rem; margin-top: -2px;
    border-left: 2px solid var(--on-primary); border-bottom: 2px solid var(--on-primary);
    transform: rotate(-45deg) scale(0.3); opacity: 0;
    transition: transform 180ms var(--ease), opacity 140ms var(--ease); }
  .tcheck:hover .box { border-color: var(--primary); }
  .tcheck input:checked + .box { background: var(--primary); border-color: var(--primary); }
  .tcheck input:checked + .box::after { transform: rotate(-45deg) scale(1); opacity: 1; }
  .tcheck input:focus-visible + .box { box-shadow: 0 0 0 3px var(--primary-container); }
  .tfoot { display: grid; gap: var(--s2); padding: var(--s2) var(--s3) var(--s3);
    border-top: 1px solid var(--outline-variant); }
  .tfoot .note { margin: 0; }
  .tfoot .row { justify-content: flex-end; gap: var(--s2); align-items: center; }
  /* The count moved here from the header: it describes what Save will send, so
     it belongs beside Save, not floating over the list it counts. */
  .tcount { margin-right: auto; font-family: var(--mono); font-size: 11px;
    font-variant-numeric: tabular-nums; }

  .chips { display: flex; flex-wrap: wrap; gap: var(--s1) var(--s2); }
  .chip { background: var(--surface-container-high); color: var(--on-surface-variant);
    border: 1px solid transparent; border-radius: var(--shape-full); font: inherit;
    font-family: var(--mono); font-size: 11px; padding: 0 var(--s3); min-height: 1.9rem;
    cursor: pointer; display: inline-flex; align-items: center; gap: var(--s1);
    transition: background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  .chip:hover { background: var(--surface-container-highest); color: var(--on-surface); }
  .chip.on { background: var(--secondary-container); color: var(--on-secondary-container);
    border-color: var(--primary); }

  .row { display: flex; align-items: center; gap: 2px; }
  .row :global(.convo) { flex: 1; }
  /* Scoped, because `.mini` is also the composer's mid-turn icon: that one is a
     38px filled circle that is always visible. One name, two lives, and the
     later rule won — the row buttons took the circle's width and two invisible
     circles ate a third of every thread title. */
  .row .mini { flex: 0 0 auto; width: 1.6rem; min-height: 1.6rem; padding: 0;
    display: grid; place-items: center; background: none; border: 0;
    border-radius: var(--shape-full); color: var(--faint); font: inherit; cursor: pointer;
    opacity: 0; transition: opacity var(--motion-short) var(--ease),
      background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  .row:hover .mini, .row .mini:focus-visible { opacity: 1; }
  /* An open menu whose trigger vanished on mouse-out is a menu you cannot close
     by clicking the thing that opened it. */
  .row .mini.on { opacity: 1; background: var(--surface-container-high); color: var(--primary); }
  .row .mini:hover { background: var(--surface-container-high); color: var(--primary); }
  /* No hover to reveal them, so there is nothing to reveal: keep them shown. */
  @media (hover: none) { .row .mini { opacity: 1; } }
  .morepick { position: relative; display: flex; flex: 0 0 auto; }
  .convo.editing { background: var(--surface-container-high); }
  .convo.editing input { width: 100%; background: var(--surface-container-highest); color: var(--on-surface);
    border: 0; border-radius: var(--shape-xs); font: inherit; font-size: 12px; padding: var(--s1) var(--s2); }
  /* Pinned to the foot of the rail: the two entries you look for at the bottom of
     a menu, not scrolled away above the history. */
  .side-foot { position: sticky; bottom: calc(-1 * var(--s3)); z-index: 1; margin-top: auto;
    display: grid; gap: 2px; padding-top: var(--s3); border-top: 1px solid var(--outline-variant);
    background: var(--surface-container-low); }
  .gear { display: flex; align-items: center; gap: var(--s2); width: 100%; text-align: left;
    background: none; border: 0; color: var(--on-surface-variant); border-radius: var(--shape-full);
    font: inherit; font-size: 12px; padding: var(--s2) var(--s3); min-height: 2rem; cursor: pointer;
    transition: background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  .gear:hover { color: var(--primary); background: var(--state-hover); }
  .gear span { width: 1em; text-align: center; opacity: 0.75; }

  /* ── tool rows ─────────────────────────────────────────────────────────
     A tool call is a tick off the spine, not a card: the state is the colour of
     its marker, the duration is the number beside the name, and the payload is
     behind a disclosure because the payload is the evidence, not the message. */
  .tool, .act, .think { position: relative; margin: 0 0 2px; padding: 0; border-top: 0; }
  /* The marker is the event's own glyph, coloured by what the call did, so a
     scan down the spine reads as a sequence of actions — not a row of identical
     dots that could only say "something happened". */
  .ev { color: var(--tertiary); }
  .tool.ok .ev { color: var(--primary); }
  .tool.fail .ev { color: var(--error); }
  .tool.run .ev { animation: pulse 1.2s ease-in-out infinite; }
  .think .ev, .act .ev { color: var(--outline); }
  .ev :global(svg) { width: 13px; height: 13px; }
  /* The current thinking state: a slow turn, not a blink, because it has to look
     alive for as long as the model takes. */
  .spin { color: var(--primary); animation: turn 1.6s linear infinite; }
  .spin :global(svg) { width: 13px; height: 13px; }
  @keyframes turn { to { transform: rotate(360deg); } }
  .tool summary, .act summary, .think summary { cursor: pointer; list-style: none; display: flex;
    align-items: center; gap: var(--s2); padding: 3px 0; color: var(--on-surface-variant);
    font-family: var(--mono); font-size: 12px; }
  .tool summary::-webkit-details-marker, .act summary::-webkit-details-marker,
  .think summary::-webkit-details-marker { display: none; }
  .tool summary b { font-weight: 500; color: var(--on-surface); }
  .tool.ok summary b { color: var(--on-surface); }
  .tool.fail summary b { color: var(--error); }
  .dur { color: var(--faint); font-size: 10px; font-variant-numeric: tabular-nums; }
  /* A chevron, not "▸": the glyph is absent from some mono fonts and renders as tofu. */
  .tool summary::before, .act summary::before, .think summary::before { content: ""; flex: none;
    width: 7px; height: 7px; background: var(--faint);
    -webkit-mask: var(--chev) center / contain no-repeat; mask: var(--chev) center / contain no-repeat;
    transition: transform 0.12s ease; }
  .tool[open] summary::before, .act[open] summary::before,
  .think[open] summary::before { transform: rotate(90deg); }
  .tool pre, .act pre { margin: var(--s1) 0 var(--s2); padding: var(--s2) var(--s3);
    background: var(--surface-container-low); border-radius: var(--shape-sm); color: var(--dim);
    font-family: var(--mono); font-size: 11.5px; max-height: 18rem; overflow: auto; white-space: pre-wrap; }
  .tool .out { max-height: 24rem; }
  /* JSON token colours. Muted on purpose: this is evidence you scan, and the
     keys are the only part that needs to stand out. One rule for both halves of
     the report — the input is JSON as often as the result is. */
  .tool pre :global(.jk) { color: var(--primary); }
  .tool pre :global(.js) { color: var(--on-surface-variant); }
  .tool pre :global(.jn) { color: var(--tertiary); }
  .tool pre :global(.jb) { color: var(--error); }
  /* An open card is a panel, and its left edge is the outcome: mint for a call
     that returned, rose for one that did not. Nothing moved when closed, so the
     spine still reads as one column. */
  .tool[open] { border-left: 2px solid var(--outline-variant); padding-left: var(--s3);
    margin-bottom: var(--s2); }
  .tool[open].ok { border-left-color: var(--primary); }
  .tool[open].fail { border-left-color: var(--error); }
  /* The two halves of the report, labelled and each with its own copy. Colour is
     the difference: what was asked arrives in the tertiary hue, what came back in
     the primary one — so a long card can be read by its edges alone. */
  .io { display: flex; align-items: center; gap: var(--s2); margin: var(--s2) 0 2px; }
  .io .lbl { font-family: var(--mono); font-size: 9px; font-weight: 600; letter-spacing: 0.16em;
    text-transform: uppercase; color: var(--faint); }
  .io.inp .lbl { color: var(--tertiary); }
  .io.outp .lbl { color: var(--primary); }
  .io .linky { margin-left: auto; }
  .tool pre.args { border-left: 2px solid var(--tertiary); }
  .tool pre.out { border-left: 2px solid var(--primary); }
  /* A failed call's output is the thing that went wrong, so its edge says so. */
  .tool.fail pre.out { border-left-color: var(--error); }
  .pill { font-family: var(--mono); font-size: 9px; letter-spacing: 0.12em; text-transform: uppercase;
    border: 1px solid currentColor; border-radius: var(--shape-full); padding: 1px 6px;
    color: var(--faint); }
  .pill.ok { color: var(--primary); }
  .pill.bad { color: var(--error); }
  .pill.mode { color: var(--tertiary); }
  .act { color: var(--faint); font-size: 11px; font-family: var(--mono); }
  .think { color: var(--faint); font-size: 12px; }
  /* Dimmed and clamped: a long thought is context, not the answer, so it must
     never be able to push the answer off the screen. */
  .think .body { margin: var(--s1) 0 var(--s2); padding: var(--s2) var(--s3);
    max-height: 18rem; overflow: auto; border-left: 2px solid var(--outline-variant);
    border-radius: 0 var(--shape-sm) var(--shape-sm) 0; color: var(--dim); font-size: 12px; }

  /* Sources: chips, because a url is a thing you click, not a line you read. */
  .src { display: flex; flex-wrap: wrap; gap: var(--s1); margin: 0 0 var(--s2); }
  .src .chip { text-decoration: none; max-width: 22rem; overflow: hidden; text-overflow: ellipsis;
    white-space: nowrap; display: inline-block; line-height: 1.9rem; }
  /* The ripple is appended inside the target, so a link or a disclosure row must
     clip it the way a button already does. */
  a.chip, summary { position: relative; overflow: hidden; }

  /* ── generative UI ─────────────────────────────────────────────────────
     A result's shape picks its view. None of these knows a tool by name, so an
     unfamiliar payload falls through to <pre>, which is always correct. */
  .table-wrap { max-height: 24rem; overflow: auto; border-radius: var(--shape-sm);
    background: var(--surface-container-low); margin: 0 0 var(--s2); }
  .table-wrap table { border-collapse: collapse; font-family: var(--mono); font-size: 11.5px; width: 100%; }
  .table-wrap th { position: sticky; top: 0; background: var(--surface-container); color: var(--faint);
    text-transform: uppercase; font-size: 9px; letter-spacing: 0.12em; text-align: left;
    padding: var(--s1) var(--s3); border-bottom: 1px solid var(--outline-variant); white-space: nowrap; }
  .table-wrap td { padding: var(--s1) var(--s3); border-bottom: 1px solid var(--outline-variant);
    max-width: 22rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .table-wrap tr:last-child td { border-bottom: 0; }
  .kv { display: grid; grid-template-columns: minmax(6rem, auto) 1fr; gap: 2px var(--s3);
    margin: 0 0 var(--s2); padding: var(--s2) var(--s3); border-radius: var(--shape-sm);
    background: var(--surface-container-low); font-family: var(--mono); font-size: 11.5px; }
  .kv :global(dt) { color: var(--faint); }
  .kv :global(dd) { margin: 0; overflow: hidden; text-overflow: ellipsis; }

  /* ── evidence and next steps ───────────────────────────────────────────
     Both are derived from the turn, never generated: the sources are the urls
     tools returned, the suggestions are what those results make possible. */
  .evidence { margin-top: var(--s4); border-top: 1px solid var(--rule); padding-top: var(--s3); }
  .evidence h3 { display: flex; gap: var(--s2); align-items: baseline; margin: 0 0 var(--s2);
    font-family: var(--mono); font-size: 9px; font-weight: 600; text-transform: uppercase;
    letter-spacing: 0.18em; color: var(--faint); }
  .evidence ol { margin: 0; padding: 0; list-style: none; display: grid; gap: 2px;
    font-family: var(--mono); font-size: 11.5px; }
  .evidence li { display: flex; gap: var(--s2); align-items: baseline; }
  .evidence li a { color: var(--primary); text-decoration: none; }
  .evidence li a:hover { text-decoration: underline; text-underline-offset: 3px; }
  .evidence li .dim { font-size: 10px; }
  .next { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); margin-top: var(--s3); }
  .eyebrow { font-family: var(--mono); font-size: 9px; text-transform: uppercase;
    letter-spacing: 0.18em; color: var(--faint); }
  .next .chip { color: var(--on-surface); }

  .dim { color: var(--faint); }
  .bad { color: var(--error); font-size: 11px; }
  .usage { border-top: 1px dashed var(--rule); padding-top: var(--s1); color: var(--dim);
    font-size: 11px; font-family: var(--mono); margin-bottom: var(--s2); }
  /* The turn's route. Wraps rather than scrolls: a 12-tool turn is a paragraph of
     steps, and hiding half of it would defeat the point of showing it. */
  .steps { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2);
    margin: var(--s2) 0; font-family: var(--mono); font-size: 10.5px; }
  .steps .route { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; min-width: 0; }
  .steps .arrow { color: var(--faint); }
  .steps .stp { display: inline-flex; align-items: center; gap: 5px; max-width: 16rem;
    padding: 2px 8px; border-radius: var(--shape-full); background: var(--surface-container-low);
    color: var(--on-surface-variant); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .steps .stp i { flex: none; width: 6px; height: 6px; border-radius: 50%; background: var(--faint); }
  .steps .stp.ok i { background: var(--primary); }
  .steps .stp.fail { color: var(--error); }
  .steps .stp.fail i { background: var(--error); }
  .steps .stp.done { color: var(--on-surface); }
  .steps .stp.done i { background: var(--tertiary); }
  .steps .stp u { text-decoration: none; color: var(--faint); font-size: 9.5px;
    font-variant-numeric: tabular-nums; }
  .note { color: var(--faint); font-size: 12px; border-left: 2px solid var(--outline-variant);
    padding: 0 var(--s3); margin-bottom: var(--s2); word-break: break-word; }

  /* ── empty state ───────────────────────────────────────────────────────
     The one place with a display face. A page that will be full of monospace
     evidence can afford three words of serif at the start. */
  .empty { display: grid; gap: var(--s3); max-width: min(44rem, 100%);
    margin: clamp(1.5rem, 9vh, 4rem) 0 0; }
  .empty h1 { font-family: var(--display); font-size: clamp(1.6rem, 3.4vw, 2.3rem); line-height: 1.15;
    font-weight: 400; margin: 0; letter-spacing: -0.02em; }
  .empty .lede { color: var(--on-surface-variant); margin: 0; max-width: 30rem; }
  .starters { display: grid; gap: var(--s2); margin-top: var(--s2); }
  .starter { text-align: left; background: var(--surface-container-low); color: var(--on-surface);
    border: 0; border-left: 2px solid var(--outline-variant); border-radius: 0 var(--shape-sm) var(--shape-sm) 0;
    padding: var(--s2) var(--s3); font: inherit; font-size: 13px; cursor: pointer;
    transition: border-color var(--motion-short) var(--ease), background var(--motion-short) var(--ease); }
  .starter:hover { border-left-color: var(--primary); background: var(--surface-container); }
  .starter b { display: block; color: var(--primary); font-family: var(--mono); font-size: 10px;
    text-transform: uppercase; letter-spacing: 0.14em; margin-bottom: 3px; }
  .starter span { color: var(--on-surface-variant); }

  /* ── composer ──────────────────────────────────────────────────────────
     One fused surface, M3 search-bar shaped: the field and its dock are the
     same object, so the dock cannot drift away from what it shapes. The bar
     spans the page, but its content belongs to the transcript column — when the
     rail opens the field has to shrink with the transcript, not stay full width
     and slide out of alignment. */
  #composer { position: absolute; left: var(--rail-w, 16rem); right: var(--inst-w, 15rem);
    bottom: 0; z-index: 3; background: var(--surface-container-low);
    border-top: 1px solid var(--outline-variant); }
  /* The field belongs to the column between the rails, so it follows the same two
     numbers the grid does — no padding hack, and nothing to drift. */
  :global(#app:not(:has(.deck.open)) #composer) { left: 0; }
  @media (max-width: 1100px) { #composer { right: 0; } }
  @media (max-width: 860px) { #composer { left: 0; } }
  .attached { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s1);    padding: var(--s2) clamp(1rem, 4vw, 3rem) 0; }
  .attached .chip { cursor: default; border-left: 3px solid var(--hue, var(--outline));
    display: inline-flex; align-items: center; gap: var(--s1); }
  /* The preview you see before sending. It is a blob URL — no round trip, and it
     dies with the list. */
  .attached .thumb { width: 1.5rem; height: 1.5rem; object-fit: cover; border-radius: 3px;
    flex: 0 0 auto; }
  /* One hue per kind of thing attached; the text stays neutral so contrast holds. */
  .k-img { --hue: var(--primary); }
  .k-aud { --hue: var(--tertiary); }
  .k-data { --hue: var(--on-secondary-container); }
  .k-code { --hue: var(--outline); }
  .k-doc { --hue: var(--on-surface-variant); }
  footer { display: flex; gap: var(--s2); align-items: center;
    padding: var(--s3) clamp(1rem, 4vw, 3rem) 0; }
  textarea { flex: 1; min-width: 0; resize: none; min-height: 2.6rem; max-height: 200px; background: none;
    color: var(--on-surface); border: 0; border-radius: 0; padding: var(--s2) 0; font: inherit;
    font-size: 14px; caret-color: var(--primary); }
  textarea:focus { outline: none; }
  textarea::placeholder { color: var(--faint); }
  .send { flex: 0 0 auto; display: inline-flex; align-items: center; gap: var(--s1);
    background: var(--primary); color: var(--on-primary); border: 0;
    border-radius: var(--shape-full); padding: 0 var(--s4); font: inherit; font-size: 13px;
    font-weight: 600; min-height: 2.6rem; cursor: pointer;
    transition: opacity var(--motion-short) var(--ease), box-shadow var(--motion-short) var(--ease); }
  .send svg { flex: 0 0 auto; }
  .send:hover:not(:disabled) { box-shadow: 0 1px 3px #0006; }
  .send:disabled { opacity: 0.4; cursor: default; }
  /* Stop is the same seat as Send, in the error colour: one thing occupies that
     slot, and which one tells you whether a turn is running. */
  .send.stop { background: var(--error); color: var(--error-container); }
  .send.stop:hover { box-shadow: 0 1px 3px #0006; }
  /* The mid-turn deliveries are icons, not labels: they appear only while a turn
     runs, so they have one job and no room to explain it twice. */
  #composer .mini { flex: 0 0 auto; width: 2.4rem; min-height: 2.4rem; padding: 0; display: grid;
    place-items: center; background: var(--surface-container); color: var(--on-surface-variant);
    border: 0; border-radius: var(--shape-full); cursor: pointer;
    transition: background var(--motion-short) var(--ease), color var(--motion-short) var(--ease); }
  #composer .mini:hover:not(:disabled) { background: var(--surface-container-high); color: var(--primary); }
  #composer .mini:disabled { opacity: 0.35; cursor: default; }
  .ghost { display: inline-flex; align-items: center; justify-content: center; gap: var(--s1);
    background: none; border: 1px solid var(--outline-variant); color: var(--on-surface-variant);
    border-radius: var(--shape-full); padding: 0 var(--s3); font: inherit; font-size: 12px;
    min-height: 2.2rem; white-space: nowrap; cursor: pointer;
    transition: color var(--motion-short) var(--ease), border-color var(--motion-short) var(--ease),
      background var(--motion-short) var(--ease); }
  .ghost:hover { color: var(--primary); border-color: var(--primary); }
  .ghost.on { background: var(--secondary-container); color: var(--on-secondary-container);
    border-color: transparent; }

  /* The command palette floats above the composer, inside it. */
  .palette { position: absolute; bottom: calc(100% + var(--s2)); left: clamp(1rem, 4vw, 3rem);
    right: clamp(1rem, 4vw, 3rem); max-height: 17rem; overflow-y: auto; z-index: 5;
    background: var(--surface-container-high); border-radius: var(--shape-md);
    box-shadow: 0 8px 24px #000a; animation: pop var(--motion-short) var(--ease); }
  .palette .head { position: sticky; top: 0; display: flex; justify-content: space-between;
    padding: var(--s1) var(--s3); background: var(--surface-container-high);
    border-bottom: 1px solid var(--outline-variant); color: var(--faint);
    font-family: var(--mono); font-size: 9px; text-transform: uppercase; letter-spacing: 0.16em; }
  .opt { display: flex; gap: var(--s2); align-items: baseline; width: 100%; text-align: left;
    background: none; border: 0; color: var(--on-surface); font: inherit; font-size: 13px;
    padding: var(--s1) var(--s3); cursor: pointer; min-height: 0; }
  .opt.on { background: var(--secondary-container); }
  .opt b { color: var(--primary); font-family: var(--mono); font-weight: 500; white-space: nowrap; }
  /* The sigil carries the kind, so the three lists are told apart at a glance:
     / command is the signal colour, @ skill is the warning colour, # tool is mint. */
  .opt.k-skill b { color: var(--tertiary); }
  .opt.k-tool b { color: var(--on-secondary-container); }
  .opt .dim { font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }

  /* ── dialogs ───────────────────────────────────────────────────────────
     A dialog interrupts, so it is the only surface allowed an elevation
     shadow. Everything else is tone. */
  /* One rule for every dialog. `#tools-modal` was missing from it, which is why
     its card sat at the left edge with no scrim — and why its own
     click-outside-to-close never fired: the container was not full-bleed.
     Flex + auto margins rather than `place-items: center`: a centred grid item
     taller than the window is clipped at both ends with nothing to scroll, so
     the action row at the bottom of a long dialog was unreachable. This centres
     while the card fits and scrolls the backdrop when it does not. */
  #modal, #tools-modal { position: fixed; inset: 0; background: #000a;
    display: flex; overflow-y: auto; padding: var(--s4);
    backdrop-filter: blur(3px); animation: fade var(--motion-short) var(--ease); z-index: 40; }
  .card { margin: auto; }
  .card { background: var(--surface-container-high); border-radius: var(--shape-lg);
    padding: var(--s4) var(--s5, 20px); max-width: 44rem; width: 100%;
    box-shadow: 0 12px 32px #000b; animation: pop var(--motion-medium) var(--ease); }
  .card h2 { margin: 0 0 var(--s2); font-family: var(--display); font-size: 16px; font-weight: 600;
    color: var(--on-surface); }
  .card h2 .queued { float: right; font-family: var(--mono); font-size: 10px; font-weight: 400;
    color: var(--faint); }
  .card .q { margin: 0 0 var(--s3); white-space: pre-wrap; color: var(--on-surface-variant); }
  .card .answer { flex: 1; background: var(--surface-container-highest); color: var(--on-surface);
    border: 0; border-bottom: 1px solid var(--outline); border-radius: var(--shape-xs) var(--shape-xs) 0 0;
    padding: var(--s2) var(--s3); font: inherit; }
  .card .answer:focus { border-bottom-color: var(--primary); outline: none; }
  .card .row.choices { justify-content: flex-start; flex-wrap: wrap; }
  .card .row.choices button { background: var(--surface-container-highest); color: var(--on-surface);
    font-weight: 400; }
  .card pre { white-space: pre-wrap; word-break: break-word; color: var(--dim);
    font-family: var(--mono); font-size: 11.5px; max-height: 40vh; overflow: auto;
    background: var(--surface-container-lowest); border-radius: var(--shape-sm); padding: var(--s2) var(--s3); }

  /* ── approval ──────────────────────────────────────────────────────────
     The tool, what it will do, and the exact arguments it will do it with —
     built from the runtime's structured request instead of a Debug string. */
  .card.ask { max-width: 34rem; }
  .ask-head { display: flex; gap: var(--s3); align-items: flex-start; margin-bottom: var(--s3); }
  .seal { width: 2.2rem; height: 2.2rem; flex: 0 0 auto; display: grid; place-items: center;
    border-radius: var(--shape-full); background: var(--secondary-container);
    color: var(--on-secondary-container); }
  .seal :global(svg) { width: 17px; height: 17px; display: block; }
  .ask-id { min-width: 0; }
  .card .ask-id h2 { margin: 0 0 2px; font-family: var(--mono); font-size: 15px; font-weight: 600;
    letter-spacing: -0.01em; }
  .sub { display: flex; flex-wrap: wrap; align-items: center; gap: var(--s2); margin: 0;
    color: var(--faint); font-size: 11px; }
  .sub .mono { font-family: var(--mono); }
  .pill { border-radius: var(--shape-full); padding: 1px var(--s2); font-size: 9px;
    text-transform: uppercase; letter-spacing: 0.12em;
    background: var(--error-container); color: var(--error); }
  .pill.ro { background: var(--surface-container-highest); color: var(--on-surface-variant); }
  .card .args { margin: 0 0 var(--s3); padding: var(--s2) var(--s3); max-height: 12rem; overflow: auto;
    background: var(--surface-container-lowest); border-radius: var(--shape-sm); }
  .card .args :global(dd) { font-family: var(--mono); }
  .fine { margin: 0; color: var(--faint); font-size: 11px; line-height: 1.5; }
  .fine code { font-family: var(--mono); background: var(--surface-container-highest);
    border-radius: var(--shape-xs); padding: 0 4px; }

  /* Context meter — the only gauge in the rail, and the only number that says a
     conversation is about to be compacted. */
  .ctx { height: 3px; border-radius: var(--shape-full); overflow: hidden;
    background: var(--surface-container-highest); }
  .ctx i { display: block; height: 100%; border-radius: inherit; background: var(--primary);
    transition: width var(--motion-medium) var(--ease); }
  .ctx-n { margin: var(--s1) 0 0; color: var(--faint); font-size: 10px;
    font-variant-numeric: tabular-nums; }

  .card.wide, .card.settings { max-width: 56rem; }
  /* M3 primary tabs: the active one carries an indicator, not a border.
     The row scrolls sideways with no scrollbar: drag it, or use the two edge
     buttons — each only exists while there is something that way. */
  .tabswrap { position: relative; margin-bottom: var(--s4); }
  .tabs { display: flex; gap: var(--s1); overflow-x: auto; scrollbar-width: none;
    border-bottom: 1px solid var(--outline-variant); cursor: grab; touch-action: pan-y; }
  .tabs::-webkit-scrollbar { display: none; }
  .tabs:active { cursor: grabbing; }
  /* A shadow of the dialog's own fill is the hint that the row continues. */
  .tabswrap::before, .tabswrap::after { content: ""; position: absolute; top: 0; bottom: 1px;
    width: 2.4rem; pointer-events: none; opacity: 0; z-index: 1;
    transition: opacity var(--motion-short) var(--ease); }
  .tabswrap::before { left: 0;
    background: linear-gradient(to right, var(--surface-container-high), transparent); }
  .tabswrap::after { right: 0;
    background: linear-gradient(to left, var(--surface-container-high), transparent); }
  .tabswrap.left::before, .tabswrap.right::after { opacity: 1; }
  .tabnav { position: absolute; top: 50%; transform: translateY(-50%) scale(0.85); z-index: 2;
    width: 1.9rem; height: 1.9rem; display: grid; place-items: center; padding: 0;
    background: var(--surface-container-highest); color: var(--on-surface-variant);
    border: 1px solid var(--outline-variant); border-radius: var(--shape-full); cursor: pointer;
    box-shadow: 0 2px 8px rgb(0 0 0 / 0.5); opacity: 0; pointer-events: none;
    transition: opacity var(--motion-short) var(--ease), transform var(--motion-short) var(--ease),
      color var(--motion-short) var(--ease), border-color var(--motion-short) var(--ease); }
  .tabnav :global(svg) { width: 13px; height: 13px; }
  .tabnav.l { left: -0.6rem; }
  .tabnav.r { right: -0.6rem; }
  .tabswrap.left .tabnav.l, .tabswrap.right .tabnav.r {
    opacity: 1; pointer-events: auto; transform: translateY(-50%) scale(1); }
  .tabnav:hover { color: var(--primary); border-color: var(--primary); }
  .tab { flex: 0 0 auto; background: none; border: 0; color: var(--on-surface-variant);
    border-radius: var(--shape-xs) var(--shape-xs) 0 0; font: inherit; font-size: 12px;
    padding: var(--s2) var(--s3); min-height: 2.4rem; cursor: pointer;
    border-bottom: 2px solid transparent;
    transition: color var(--motion-short) var(--ease), border-color var(--motion-short) var(--ease); }
  .tab:hover { color: var(--on-surface); background: var(--state-hover); }
  .tab.on { color: var(--primary); border-bottom-color: var(--primary); }
  /* Tabs carry a mark, so the row is a set of icons with names, not a list of
     words. The dot is the provider this session is actually using. */
  .tab { display: inline-flex; align-items: center; gap: var(--s1); }
  .tab .mark { display: grid; place-items: center; flex: 0 0 auto; opacity: 0.9; }
  .tab .mark :global(svg) { width: 15px; height: 15px; display: block; }
  /* The tick replaces the key dot on the row that is actually running: "has a
     key" and "is the one in use" are different facts and one slot cannot say
     both, so the running row gives up the dot. */
  .tab .cur { display: grid; place-items: center; flex: 0 0 auto; color: var(--primary); }
  .tab .cur :global(svg) { width: 14px; height: 14px; display: block; }
  .tab.on .cur { color: var(--primary); }
  /* Whether a key is already stored is the first thing you need before picking a
     provider, so every tab answers it: filled when there is one, hollow when not. */
  .kd { width: 5px; height: 5px; border-radius: 50%; border: 1px solid var(--outline);
    flex: 0 0 auto; }
  .kd.on { background: var(--tertiary); border-color: var(--tertiary); }
  .tab.plus { width: 2.4rem; padding: 0; font-size: 16px; margin-left: auto; justify-content: center; }
  /* The key field is the one thing in this dialog that writes to the vault, so it
     reads as an action row, not as another fact. */
  .keyrow { display: flex; gap: var(--s2); margin-top: var(--s3); }
  .keyrow input { flex: 1; min-width: 0; background: var(--surface-container-highest);
    color: var(--on-surface); border: 0; border-bottom: 1px solid var(--outline);
    border-radius: var(--shape-xs) var(--shape-xs) 0 0; padding: var(--s2) var(--s3);
    font: inherit; font-family: var(--mono); font-size: 12px; }
  .keyrow input:focus { border-bottom-color: var(--primary); outline: none; }
  .keyrow button { background: var(--primary); color: var(--on-primary); border: 0;
    border-radius: var(--shape-full); padding: 0 var(--s4); font: inherit; font-size: 12px;
    min-height: 2.2rem; cursor: pointer; }
  .keyrow button:disabled { background: var(--surface-container-highest);
    color: var(--on-surface-variant); cursor: default; }
  /* Whether a key exists is the question this dialog exists to answer, so it is
     a line of text with a glyph, not a 6px dot you have to hover to read. */
  .keysaved, .nokey { display: flex; align-items: center; gap: var(--s2); margin: var(--s2) 0 0;
    font-family: var(--mono); font-size: 11px; color: var(--primary); }
  .nokey { color: var(--tertiary); }
  /* The off-peak block: a fact about billing, so it sits on its own surface. */
  .offpeak { margin-top: var(--s4); padding: var(--s3); border-radius: var(--shape-md);
    background: var(--surface-container-highest); }
  .offpeak .op { margin: 0 0 var(--s2); font-family: var(--mono); font-size: 12px;
    letter-spacing: 0.04em; color: var(--primary); }
  .offpeak .op.peak { color: var(--tertiary); }
  .form { display: grid; gap: var(--s3); }
  .form input { background: var(--surface-container-highest); color: var(--on-surface); border: 0;
    border-bottom: 1px solid var(--outline); border-radius: var(--shape-xs) var(--shape-xs) 0 0;
    padding: var(--s3); font: inherit; transition: border-color var(--motion-short) var(--ease); }
  .form input:focus { border-bottom-color: var(--primary); outline: none; }
  /* A labelled field inside the settings form: label, control, one line of why. */
  .sfld { display: grid; gap: var(--s1); justify-items: start; }
  .sfld > span { font-family: var(--mono); font-size: 9px; text-transform: uppercase;
    letter-spacing: 0.16em; color: var(--faint); }
  .sfld small { color: var(--faint); font-size: 11px; line-height: 1.4; }
  .sfld textarea, .sfld input { width: 100%; background: var(--surface-container-highest);
    color: var(--on-surface); border: 0; border-bottom: 1px solid var(--outline);
    border-radius: var(--shape-xs) var(--shape-xs) 0 0; padding: var(--s2) var(--s3);
    font: inherit; font-size: 13px; min-height: 2.4rem; resize: vertical; }
  .sfld textarea { min-height: 7rem; line-height: 1.55; }
  .sfld textarea:focus, .sfld input:focus { border-bottom-color: var(--primary); outline: none; }
  .sfld textarea::placeholder, .sfld input::placeholder { color: var(--faint); }
  .sub-h { margin: var(--s4) 0 var(--s2); font-family: var(--mono); font-size: 9px;
    text-transform: uppercase; letter-spacing: 0.16em; color: var(--faint); }
  .linky.danger { color: var(--error); }
  .facts { display: grid; grid-template-columns: 8rem 1fr; gap: var(--s2) var(--s3); margin: 0;
    font-family: var(--mono); font-size: 12px; }
  .facts :global(dt) { color: var(--on-surface-variant); }
  .facts :global(dd) { margin: 0; color: var(--on-surface); word-break: break-all; }
  .card.settings { display: grid; grid-template-columns: 9rem 1fr; gap: var(--s4); }
  /* Settings navigates vertically, so its tabs are a rail, not an underline strip. */
  .stabs { display: flex; flex-direction: column; gap: 2px;
    border-right: 1px solid var(--outline-variant); padding-right: var(--s3); }
  .stabs .tab { text-align: left; border-radius: var(--shape-full); width: 100%;
    border-bottom: 0; min-height: 2.2rem; padding: var(--s2) var(--s3); }
  .stabs .tab.on { background: var(--secondary-container); color: var(--on-secondary-container); }
  .sbody { min-height: 9rem; }

  /* Provider tab body: the live catalogue, filterable, one row per model. It is
     the provider's own list — not a copy of the default model in the config. */
  .mhead { display: flex; justify-content: space-between; margin: var(--s4) 0 var(--s2);
    font-family: var(--mono); font-size: 9px; text-transform: uppercase; letter-spacing: 0.16em;
    color: var(--faint); }
  .mfilter { width: 100%; background: var(--surface-container-highest); color: var(--on-surface);
    border: 0; border-bottom: 1px solid var(--outline); border-radius: var(--shape-xs) var(--shape-xs) 0 0;
    padding: var(--s2) var(--s3); font: inherit; font-family: var(--mono); font-size: 12px;
    margin-bottom: var(--s2); }
  .mfilter:focus { border-bottom-color: var(--primary); outline: none; }
  .mlist { max-height: 40vh; overflow-y: auto; display: grid; gap: 1px; }
  .m { display: flex; justify-content: space-between; gap: var(--s2); width: 100%; text-align: left;
    background: none; border: 0; color: var(--on-surface); border-radius: var(--shape-xs);
    padding: 3px var(--s2); font: inherit; font-family: var(--mono); font-size: 11.5px;
    cursor: pointer; min-height: 0; transition: background var(--motion-short) var(--ease); }
  .m:hover { background: var(--state-hover); color: var(--primary); }
  /* Staged: the row Save would apply. Its own state, because it is neither the
     running model nor a committed change. */
  .m.on { background: var(--secondary-container); color: var(--on-secondary-container); }
  .m .tag { color: var(--tertiary); font-size: 9px; text-transform: uppercase; letter-spacing: 0.1em;
    margin-left: var(--s1); }
  /* The row this session runs — the answer to "did my pick take?" without
     reopening anything. */
  .m .tag.on { color: var(--on-surface); background: var(--tertiary); padding: 0 4px;
    border-radius: var(--shape-full); letter-spacing: 0.06em; }
  /* A message rendered before the markdown chunk lands. */
  .md :global(pre.md-pending) { background: none; padding: 0; font-family: inherit;
    font-size: inherit; white-space: pre-wrap; }

  /* ── rail search ───────────────────────────────────────────────────────
     A filled field, pinned at the top of the rail. It replaces the list in
     place: matches are the list while there is a query, threads when there is
     not, so there is never a second surface competing for the same answer. */
  .rail-search { position: relative; display: flex; align-items: center; margin-bottom: var(--s3); }
  .rail-search .mag { position: absolute; left: var(--s2); color: var(--faint); font-size: 13px;
    pointer-events: none; }
  .rail-search input { width: 100%; background: var(--surface-container);
    border: 1px solid transparent; border-radius: var(--shape-full);
    color: var(--on-surface); padding: 0 var(--s5) 0 1.6rem; min-height: 2.1rem;
    font: inherit; font-size: 12px; font-family: var(--mono);
    transition: background var(--motion-short) var(--ease), border-color var(--motion-short) var(--ease); }
  .rail-search input::placeholder { color: var(--faint); }
  .rail-search input:focus { outline: none; background: var(--surface-container-high);
    border-color: var(--primary); }
  .clearq { position: absolute; right: 2px; width: 1.6rem; height: 1.6rem; padding: 0;
    display: grid; place-items: center; background: none; border: 0; color: var(--faint);
    border-radius: var(--shape-full); font: inherit; font-size: 10px; cursor: pointer; min-height: 0; }
  .clearq:hover { color: var(--primary); background: var(--state-hover); }
  .count { color: var(--faint); font-variant-numeric: tabular-nums; }
  .rhit { display: grid; gap: 1px; width: 100%; text-align: left; background: none;
    border: 0; border-left: 2px solid var(--outline-variant); border-radius: 0 var(--shape-xs) var(--shape-xs) 0;
    color: var(--on-surface); padding: var(--s1) var(--s2); font: inherit; font-size: 12px;
    cursor: pointer; min-height: 0; margin-bottom: 2px;
    transition: background var(--motion-short) var(--ease), border-color var(--motion-short) var(--ease); }
  .rhit:hover, .rhit:focus-visible { background: var(--state-hover); border-left-color: var(--primary); }
  .rhit .rtitle { color: var(--primary); font-weight: 600; overflow: hidden;
    text-overflow: ellipsis; white-space: nowrap; }
  .rhit .rsnip { color: var(--on-surface-variant); font-size: 11px; overflow: hidden;
    text-overflow: ellipsis; white-space: nowrap; }
  /* Scoped to the dialog: `.row` is also the rail's list row, and an unscoped
     `button` rule here painted every thread with the primary fill. */
  .card .row { display: flex; gap: var(--s2); justify-content: flex-end; margin-top: var(--s3); }
  .card .row button { background: var(--primary); color: var(--on-primary); border: 0;
    border-radius: var(--shape-full); padding: 0 var(--s4); font: inherit; font-weight: 600;
    cursor: pointer; min-height: 2.4rem; }
  .card .row .deny { background: var(--surface-container-highest); color: var(--on-surface); }

  /* Two motion families and a ripple. Nothing else moves. */
  @keyframes rise { from { opacity: 0; transform: translateY(4px); } }
  @keyframes fade { from { opacity: 0; } }
  @keyframes pop { from { opacity: 0; transform: scale(0.985); } }
  @keyframes blink { 50% { opacity: 0; } }
  @keyframes pulse { 50% { opacity: 0.35; } }
  .caret { display: inline-block; width: 0.5em; height: 1em; margin-left: 2px; vertical-align: -2px;
    background: var(--primary); animation: blink 1.1s steps(2, start) infinite; }

  /* Tablet: the instruments go, the transcript keeps the width. The rail column
     is reserved only while the rail is actually shown — `.deck:not(.open)` hides
     it, so a fixed 16rem there left the transcript in the sidebar's column and
     the 1fr empty: collapsing the rail squashed the chat instead of widening it. */
  @media (max-width: 1100px) {
    .deck.open { grid-template-columns: var(--rail-w, 16rem) minmax(0, 1fr); }
    .deck:not(.open) { grid-template-columns: minmax(0, 1fr); }
    .inst { display: none; }
    .grip.r { display: none; }
  }

  /* Phone: the rail becomes an overlay drawer that stops above the composer —
     one that covers the field you are about to type into is a drawer you have to
     close twice. */
  @media (max-width: 860px) {
    .deck, .deck:not(.open) { grid-template-columns: minmax(0, 1fr); }
    .rail, .deck:not(.open) .rail {
      display: flex; position: fixed; top: 44px; bottom: var(--composer-h, 8rem); left: 0; z-index: 30;
      width: min(84vw, 17rem);
      border-right: 1px solid var(--outline-variant); transform: translateX(-100%);
      transition: transform var(--motion-short) var(--ease); }
    .deck.open .rail { transform: none; box-shadow: 0 0 60px #000c; }
    /* A drawer over the transcript has nothing to be dragged against. */
    .grip { display: none; }
    .side-foot { bottom: 0; }
    main { padding: var(--s2) var(--s3) calc(var(--composer-h, 8rem) + var(--s2)) 2.6rem; }
    main::before { left: 1.55rem; }
    .msg::after { left: -1.14rem; }
    .tool::after, .act::after, .think::after { left: -0.99rem; }
    .topbar { padding: 0 var(--s2); }
    .meter-inline { display: none; }
    .dock { padding-inline: var(--s3); gap: var(--s1) var(--s2); }
    footer { padding: var(--s2) var(--s3) 0; }
    .attached { padding-inline: var(--s3); }
    .palette { left: var(--s3); right: var(--s3); }
    .card { padding: var(--s4); }
    .card.settings { grid-template-columns: 1fr; }
    .stabs { flex-direction: row; overflow-x: auto; border-right: 0; border-bottom: 1px solid var(--outline-variant); }
    .stabs .tab { width: auto; }
    /* One column of boxes: three across a phone is a wall of 11px labels. */
    .card.tools { width: min(100%, 38rem); }
    .tchecks { grid-template-columns: 1fr; padding-left: var(--s2); }
    .tfoot .row { flex-wrap: wrap; }
    .tcount { width: 100%; margin: 0 0 var(--s1); }
    .ghost { padding: 0 var(--s2); }
    .dock { justify-content: flex-start; }
  }

  /* Small phone: the dock stacks and the dialog sheds its padding. */
  @media (max-width: 560px) {
    .dock { gap: var(--s1); }
    .dock .modelbtn { margin-left: 0; flex: 1 1 100%; }
    .modemenu { min-width: 0; width: min(88vw, 17rem); }
    .card.tools { border-radius: var(--shape-md); }
    .card { padding: var(--s4); border-radius: var(--shape-md); }
    .tlist { max-height: min(52vh, 24rem); }
    #modal, #tools-modal { padding: var(--s2); }
  }

  /* Motion is decoration here: none of it carries meaning. */
  @media (prefers-reduced-motion: reduce) {
    .msg, .card, .starter, #modal, #tools-modal, .palette { animation: none; }
    .caret, .rec-stop, .tool.run::after, .tool.run summary::before { animation: none; }
    .caret { display: none; }
    .rail { transition: none; }
  }
  /* ── scale pass ────────────────────────────────────────────────────────
     Type and hit targets, raised together. A UI feels expensive when nothing is
     small enough to squint at; these override the sizes above and nothing here
     changes a layout rule, only its measure. */
  textarea { font-size: 15px; min-height: 3rem; }
  .md { font-size: 15px; }
  .md :global(pre) { font-size: 13.5px; }
  .md :global(h1) { font-size: 1.24em; }
  .md :global(code) { font-size: 0.92em; padding: 1px 6px; }

  .ico { width: 2.5rem; min-height: 2.5rem; font-size: 17px; }
  .seg { padding: 3px; }
  .seg button { min-height: 2.1rem; font-size: 13px; padding: 0 var(--s4); }
  .ghost, .modelbtn { min-height: 2.5rem; font-size: 13px; }
  .send { min-height: 2.9rem; font-size: 14px; padding: 0 var(--s5); gap: var(--s2); }
  .send svg { width: 18px; height: 18px; }
  /* Send and dictate sit in the same row, so they get the same box: one 46px
     circle each, and the glyph inside them the same size too. */
  footer .send, footer .ico { width: 2.9rem; min-height: 2.9rem; padding: 0; justify-content: center; }
  footer .send svg, footer .ico svg { width: 18px; height: 18px; }
  /* The round controls around the composer and in the header carry the same
     18px glyph as Send and Dictate — at 15px they read as a second, smaller
     icon set sitting next to the first. */
  .topbar .icon .gi :global(svg), .dock .ico .gi :global(svg) { width: 18px; height: 18px; }
  .chip { min-height: 2.1rem; font-size: 12px; }
  .rail-search input { min-height: 2.5rem; font-size: 13px; }

  .convo { padding: var(--s2) var(--s3); }
  .convo .title { font-size: 13px; }
  .convo .meta { font-size: 12px; }
  .side-h { font-size: 10px; }
  .who { font-size: 10px; }
  .eyebrow { font-size: 10px; }
  .gear { font-size: 13px; min-height: 2.4rem; }
  .readout { font-size: 12px; }
  .readout :global(dt) { font-size: 10px; }
  .tool summary, .act summary, .think summary { font-size: 13px; padding: var(--s1) 0; }
  .opt { font-size: 14px; }
  .empty h1 { font-size: clamp(1.9rem, 3.8vw, 2.7rem); }
  .empty .lede { font-size: 15px; max-width: 32rem; }
  .starter { font-size: 14px; padding: var(--s3) var(--s4); }
  .starter b { font-size: 11px; }
  .card { padding: var(--s5); }
  .card .ask-id h2 { font-size: 17px; }
  .fine, .sub { font-size: 12px; }
</style>
