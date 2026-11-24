<script>
  // The tools chooser. A list, so it is a dialog with a fold per server, a search
  // that opens on demand, and one Save — not a strip of chips that rewrites the
  // setting on every stray click.
  //
  // Everything staging is local (query, folds, the draft): this dialog is the only
  // reader of those, and keeping them here is what lets `App` forget it ever drew
  // a checklist. What it owns are the two persisted sets — which builtins are
  // preferred and which tools this page may approve without asking — and those it
  // reports up.
  let {
    open = false,
    tools = [], // catalogue rows of kind "tool": [{ name: "webrain_navigate" }, …]
    builtin = [], // every builtin this build installs
    off = [], // builtins switched off (persisted)
    allow = [], // "server:tool" keys approved without asking (persisted)
    sel = [], // the tools already preferred
    dead = new Set(), // builtins the runtime withheld from this session
    runtimeOff = "", // the runtime's own sentence about what it disabled
    icons = {},
    onclose = () => {},
    onsave = () => {},
    onoff = () => {},
    onallow = () => {},
  } = $props();

  let query = $state("");
  let searchOpen = $state(false);
  let closed = $state([]); // fold state, per server name
  let draft = $state([]);

  // Opening is a fresh look at the current preference, not the last visit's
  // half-finished edit: a draft that survived a Close would be a setting nobody
  // saved.
  $effect(() => {
    if (open) {
      draft = [...sel];
      query = "";
      searchOpen = false;
      closed = [];
    }
  });

  const builtinOn = (name) => !off.includes(name);

  // Tools by the server that owns them, so the checklist folds. `webrain_navigate`
  // is server `webrain`, tool `navigate`; a second server gets its own section
  // without this code changing.
  const groups = $derived.by(() => {
    const q = query.trim().toLowerCase();
    const out = new Map();
    for (const { name: full } of tools) {
      const bare = full.replace(/^webrain[-_]/, "");
      const cut = bare.indexOf("-");
      const server = cut > 0 ? bare.slice(0, cut) : "webrain";
      const name = cut > 0 ? bare.slice(cut + 1) : bare;
      if (q && !full.toLowerCase().includes(q) && !server.toLowerCase().includes(q)) continue;
      if (!out.has(server)) out.set(server, []);
      out.get(server).push({ full, name });
    }
    return [...out].map(([server, rows]) => ({ server, tools: rows }));
  });

  // Same search box filters both lists.
  const builtinShown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return builtin.filter((n) => !q || n.toLowerCase().includes(q));
  });
  const draftCount = $derived(draft.length);
  // Only what this session can actually call: a withheld builtin is listed (so
  // its absence is explained) but is not a switch and is not counted.
  const live = $derived(builtin.filter((n) => !dead.has(n)));
  const builtinOnCount = $derived(live.filter(builtinOn).length);

  const toggleDraft = (full) =>
    (draft = draft.includes(full) ? draft.filter((t) => t !== full) : [...draft, full]);
  const toggleBuiltin = (name) =>
    onoff(builtinOn(name) ? [...off, name] : off.filter((n) => n !== name));
  const allBuiltin = (on) =>
    onoff(on
      ? off.filter((n) => !builtinShown.includes(n))
      : [...new Set([...off, ...builtinShown.filter((n) => !dead.has(n))])]);
  const allOf = (server, on) => {
    const names = groups.find((g) => g.server === server)?.tools.map((t) => t.full) || [];
    draft = on ? [...new Set([...draft, ...names])] : draft.filter((t) => !names.includes(t));
  };
  const toggleClosed = (server) =>
    (closed = closed.includes(server) ? closed.filter((s) => s !== server) : [...closed, server]);

  // The field opens from a click, so it should already have the caret.
  const focusNow = (node) => node.focus();
</script>

{#if open}
  <div id="tools-modal" onclick={(e) => e.target === e.currentTarget && onclose()} role="presentation">
    <div class="card tools">
      <div class="thead">
        <span class="ttitle">tools</span>
        <div class="tsearch" class:open={searchOpen}>
          {#if searchOpen}
            <input
              bind:value={query}
              placeholder="search tools"
              aria-label="search tools"
              use:focusNow
              onkeydown={(e) => {
                if (e.key === "Escape") {
                  query = "";
                  searchOpen = false;
                }
              }}
            />
            {#if query}
              <button class="clearx" onclick={() => (query = "")} aria-label="clear search">
                <span class="gi" aria-hidden="true">{@html icons.x}</span>
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
            <span class="gi" aria-hidden="true">{@html icons.search}</span>
          </button>
        </div>
      </div>
      <div class="tlist">
        {#if builtinShown.length}
          {@const onCount = builtinShown.filter((n) => !dead.has(n) && builtinOn(n)).length}
          {@const liveShown = builtinShown.filter((n) => !dead.has(n)).length}
          {@const folded = closed.includes("builtin")}
          <div class="tgroup">
            <div class="trow">
              <button
                class="tfold"
                onclick={() => toggleClosed("builtin")}
                aria-expanded={!folded}
                title="tools this build installs itself; unticking one asks the agent to prefer the others"
              >
                <span class="tcaret" aria-hidden="true">{@html folded ? icons.right : icons.down}</span>
                builtin
                <span class="dim">{onCount} of {liveShown} on</span>
                {#if liveShown < builtinShown.length}
                  <span class="dim">{builtinShown.length - liveShown} withheld</span>
                {/if}
              </button>
              <button class="linky" onclick={() => allBuiltin(true)}>all</button>
              <button class="linky" onclick={() => allBuiltin(false)}>none</button>
            </div>
            {#if !folded}
              <div class="tchecks">
                {#each builtinShown as name (name)}
                  {@const served = !dead.has(name)}
                  <label
                    class="tcheck"
                    class:dead={!served}
                    title={served
                      ? "a tool this build installs itself — unticking only tells the agent to prefer others"
                      : "the runtime disabled this one for this session — no switch here can turn it on"}
                  >
                    <input
                      type="checkbox"
                      checked={served && builtinOn(name)}
                      disabled={!served}
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
        {#each groups as g (g.server)}
          {@const picked = g.tools.filter((t) => draft.includes(t.full)).length}
          {@const folded = closed.includes(g.server)}
          <div class="tgroup">
            <div class="trow">
              <button class="tfold" onclick={() => toggleClosed(g.server)} aria-expanded={!folded}>
                <span class="tcaret" aria-hidden="true">{@html folded ? icons.right : icons.down}</span>
                {g.server}
                <span class="dim">{picked} of {g.tools.length}</span>
              </button>
              <button class="linky" onclick={() => allOf(g.server, true)}>all</button>
              <button class="linky" onclick={() => allOf(g.server, false)}>none</button>
            </div>
            {#if !folded}
              <div class="tchecks">
                {#each g.tools as t (t.full)}
                  {@const key = `${g.server}:${t.name}`}
                  <!-- One cell per tool: what it is, and whether this page may
                       approve it without asking. The second half is the policy
                       the "Always allow" button on a card writes into. -->
                  <div class="tcell">
                    <label class="tcheck">
                      <input
                        type="checkbox"
                        checked={draft.includes(t.full)}
                        onchange={() => toggleDraft(t.full)}
                      />
                      <span class="box" aria-hidden="true"></span>
                      <span>{t.name}</span>
                    </label>
                    <button
                      class="tallow"
                      class:on={allow.includes(key)}
                      onclick={() => onallow(key)}
                      aria-pressed={allow.includes(key)}
                      title={allow.includes(key)
                        ? "approved without asking — click to be asked again"
                        : "approve this tool without asking"}
                    >
                      {allow.includes(key) ? "always" : "ask"}
                    </button>
                  </div>
                {/each}
              </div>
            {/if}
          </div>
        {/each}
        {#if !groups.length && !builtinShown.length}
          <p class="none">no tool matches “{query}”</p>
        {/if}
      </div>
      <div class="tfoot">
        <p class="note">
          Sent as a prompt instruction — the runtime's tool set is fixed when the session starts.
        </p>
        {#if runtimeOff}
          <!-- The runtime disabled these before the page ever drew a checkbox.
               Nothing here can switch them on, which is exactly why they are
               quoted instead of listed as options. -->
          <p class="note off">runtime disabled · {runtimeOff}</p>
        {/if}
        <div class="row">
          <span class="dim tcount"
            >{draftCount + builtinOnCount} of {tools.length + live.length} preferred</span
          >
          <button class="deny" onclick={onclose}>Close</button>
          <button onclick={() => onsave(draft)}>Save · {draftCount}</button>
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  /* The shell the dialog used to inherit from the app's stylesheet. Scoped CSS
     does not cross a component boundary, so the backdrop, the card, the round
     icon button and the footer buttons are redeclared here — that duplication is
     the honest price of the split. */
  #tools-modal { position: fixed; inset: 0; background: #000a; display: flex; overflow-y: auto;
    padding: var(--s4); backdrop-filter: blur(3px);
    animation: fade var(--motion-short) var(--ease); z-index: 40; }
  .card { margin: auto; background: var(--surface-container-high); border-radius: var(--shape-lg);
    padding: var(--s4) var(--s5, 20px); max-width: 44rem; width: 100%;
    box-shadow: 0 12px 32px #000b; animation: pop var(--motion-medium) var(--ease); }
  button { position: relative; overflow: hidden; }

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
  .tcell { display: flex; align-items: center; gap: var(--s1); min-width: 0; }
  .tcell .tcheck { flex: 1; min-width: 0; }
  .tallow { flex: none; border: 0; background: none; cursor: pointer; padding: 0 var(--s1);
    font-family: var(--mono); font-size: 9px; text-transform: uppercase; letter-spacing: 0.1em;
    color: var(--faint); border-radius: var(--shape-xs); }
  .tallow:hover { color: var(--on-surface); background: var(--state-hover); }
  .tallow.on { color: var(--primary); }
  .tcheck input { flex: 0 0 auto; }
  /* A built-in name can be longer than a 11rem cell; without a shrinkable,
     ellipsed name the text ran over the next column's checkbox. */
  .tcheck { min-width: 0; }
  .tcheck > span:last-child { min-width: 0; overflow: hidden; text-overflow: ellipsis;
    white-space: nowrap; }
  .tcheck.dead { opacity: 0.5; }
  .tcheck.dead input { cursor: not-allowed; }
  .tfoot { display: grid; gap: var(--s2); padding: var(--s2) var(--s3) var(--s3);
    border-top: 1px solid var(--outline-variant); }
  .tfoot .note { margin: 0; }
  /* The runtime's own sentence about what it withheld: quoted in mono, boxed,
     and scrollable, because it can name thirty tools. */
  .tfoot .note.off { font-family: var(--mono); font-size: 10px; line-height: 1.5;
    color: var(--on-surface-variant); max-height: 5.5rem; overflow-y: auto;
    padding: 5px 8px; border-radius: 8px; background: var(--surface-container-highest); }
  .tfoot .row { display: flex; justify-content: flex-end; gap: var(--s2); align-items: center;
    margin-top: 0; }
  .tfoot button { background: var(--primary); color: var(--on-primary); border: 0;
    border-radius: var(--shape-full); padding: 0 var(--s4); font: inherit; font-weight: 600;
    cursor: pointer; min-height: 2.4rem; }
  .tfoot .deny { background: var(--surface-container-highest); color: var(--on-surface); }
  /* The count moved here from the header: it describes what Save will send, so
     it belongs beside Save, not floating over the list it counts. */
  .tcount { margin-right: auto; font-family: var(--mono); font-size: 11px;
    font-variant-numeric: tabular-nums; }
  .tcaret :global(svg) { width: 12px; height: 12px; }
  @keyframes fade { from { opacity: 0; } }
  @keyframes pop { from { opacity: 0; transform: scale(0.985); } }
  /* The phone rules that used to live in the app's sheet — a scoped selector
     cannot reach another component's DOM. */
  @media (max-width: 860px) {
    .card.tools { width: min(100%, 38rem); }
    .tchecks { grid-template-columns: 1fr; padding-left: var(--s2); }
    .tfoot .row { flex-wrap: wrap; }
    .tcount { width: 100%; margin: 0 0 var(--s1); }
  }
  @media (max-width: 560px) {
    .card.tools { border-radius: var(--shape-md); }
    .tlist { max-height: min(52vh, 24rem); }
  }
</style>
