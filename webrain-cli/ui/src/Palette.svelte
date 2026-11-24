<script>
  // The composer's menu, as its own component.
  //
  // It is the piece the page redraws on every keystroke, so it gets one small
  // reactive scope instead of the app's 7.9k-line one, and its styles leave the
  // single <style> bucket the whole page shares. Pure by design: the selection
  // lives in the parent (the field's keydown reads it), so this only renders rows
  // and reports what was clicked or hovered.
  let {
    palette = null,
    selected = 0,
    // The count, not the array: the header needs "27 · 14 shown", and handing
    // over the full list only so it can be measured would tie this component to
    // a shape it never reads — the day the menu pages, `rows` stops being a
    // prefix of `options` and only this number has to keep meaning something.
    total = 0,
    rows = [],
    sigil = {},
    onpick = () => {},
    onselect = () => {},
  } = $props();

  let el = $state(null);
  // Arrow-key selection can land past the fold, and the row it moved to only
  // exists after the render flush — hence an effect, not a call in the handler.
  $effect(() => {
    selected;
    el?.querySelector(".opt.on")?.scrollIntoView({ block: "nearest" });
  });
</script>

{#if palette && total}
  <div class="palette" role="listbox" bind:this={el}>
    <div class="head">
      {palette.trigger === "/" ? "everything" : palette.trigger === "@" ? "skills" : "tools"}
      <span class="count"
        >{total}{rows.length < total ? " · " + rows.length + " shown" : ""}</span
      >
    </div>
    {#each rows as o, i (o.kind + o.name)}
      <button
        class="opt k-{o.kind}"
        class:on={i === selected}
        role="option"
        aria-selected={i === selected}
        onclick={() => onpick(o)}
        onmouseenter={() => onselect(i)}
      >
        <i class="sig" aria-hidden="true">{sigil[o.kind]}</i>
        <b>{o.name}</b>
        {#if o.hint}<span class="dim">{o.hint}</span>{/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  /* The palette floats above the composer, inside it. Rows are pills rather than
     bands: the menu is scanned in a glance, so the gap between choices is what
     makes one findable, and the sigil carries the kind as a coloured badge
     instead of a tint on the whole word. */
  .palette { position: absolute; bottom: calc(100% + var(--s2)); left: clamp(1rem, 4vw, 3rem);
    right: clamp(1rem, 4vw, 3rem); max-height: 17rem; overflow-y: auto; z-index: 5;
    padding: 4px 0 6px; background: var(--surface-container-high);
    border: 1px solid var(--outline-variant); border-radius: 16px;
    box-shadow: 0 18px 42px -14px #000d, 0 2px 6px #0004;
    animation: pop var(--motion-short) var(--ease); scrollbar-width: thin; }
  .palette .head { position: sticky; top: 0; z-index: 1; display: flex; align-items: center;
    justify-content: space-between; padding: var(--s1) var(--s3) var(--s2);
    background: linear-gradient(var(--surface-container-high) 78%, transparent);
    color: var(--faint); font-family: var(--mono); font-size: 9px;
    text-transform: uppercase; letter-spacing: 0.16em; }
  .palette .count { padding: 1px 7px; border-radius: 999px; letter-spacing: 0.04em;
    background: var(--surface-container); color: var(--on-surface); text-transform: none; }
  .opt { display: grid; grid-template-columns: 20px auto 1fr; align-items: center;
    gap: var(--s2); width: calc(100% - 10px); margin: 1px 5px; text-align: left;
    background: none; border: 0; border-radius: 10px; color: var(--on-surface);
    font: inherit; font-size: 14px; padding: 5px 9px; cursor: pointer; min-height: 0;
    transition: background var(--motion-short) var(--ease); }
  .opt:hover { background: var(--state-hover); }
  .opt.on { background: color-mix(in srgb, var(--primary) 16%, transparent); }
  /* One accent per kind, used by both the badge and the selected row, from the
     same three tokens the field's tint uses — `/` `#` `@` are one colour each,
     menu and composer alike. */
  .opt.k-skill.on { background: color-mix(in srgb, var(--m-skill) 16%, transparent); }
  .opt.k-tool.on, .opt.k-builtin.on { background: color-mix(in srgb, var(--m-tool) 16%, transparent); }
  .opt .sig { display: grid; place-items: center; width: 20px; height: 20px;
    border-radius: 7px; font-family: var(--mono); font-size: 11px; font-style: normal;
    background: color-mix(in srgb, var(--m-cmd) 18%, transparent); color: var(--m-cmd); }
  .opt.k-skill .sig { background: color-mix(in srgb, var(--m-skill) 18%, transparent); color: var(--m-skill); }
  .opt.k-tool .sig, .opt.k-builtin .sig {
    background: color-mix(in srgb, var(--m-tool) 18%, transparent); color: var(--m-tool); }
  .opt b { color: var(--on-surface); font-family: var(--mono); font-weight: 500; white-space: nowrap; }
  .opt.on b { color: var(--m-cmd); }
  .opt.k-skill.on b { color: var(--m-skill); }
  .opt.k-tool.on b, .opt.k-builtin.on b { color: var(--m-tool); }
  .opt .dim { font-size: 11px; color: var(--faint); overflow: hidden;
    text-overflow: ellipsis; white-space: nowrap; }
  /* Local copy: Svelte prefixes a keyframe to the component that defines it, so
     the app's own `pop` cannot be reached from here. */
  @keyframes pop { from { opacity: 0; transform: scale(0.985); } }
  /* The two media rules that used to shape the palette from the app's sheet:
     a scoped selector cannot reach another component's DOM, so they live here
     with the element they describe. */
  @media (max-width: 860px) {
    .palette { left: var(--s3); right: var(--s3); }
  }
  @media (prefers-reduced-motion: reduce) {
    .palette { animation: none; }
  }
</style>
