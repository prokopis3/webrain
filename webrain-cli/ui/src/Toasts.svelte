<script>
  // The snackbar: a line from a conversation that is not on screen. Presentational
  // — it renders what it is given and reports a dismissal — so the toast queue
  // stays where the frames arrive, in App.
  let { toasts = [], ondismiss = () => {} } = $props();
</script>

{#if toasts.length}
  <!-- A snackbar, not a modal: it reports what another thread did and never asks
       for an answer. Above the composer so it cannot cover the field. -->
  <div class="toasts" role="status" aria-live="polite">
    {#each toasts as t (t.id)}
      <div class="toast" class:bad={t.kind === "err"}>
        <span>{t.text}</span>
        <button class="linky" onclick={() => ondismiss(t.id)} aria-label="dismiss">×</button>
      </div>
    {/each}
  </div>
{/if}

<style>
  /* Snackbar: a line from another thread. Bottom-left, above the composer, so it
     never covers the field it is reporting about. */
  .toasts { position: fixed; left: var(--s4); bottom: 7.5rem; z-index: 70;
    display: flex; flex-direction: column; gap: var(--s2);
    max-width: min(26rem, calc(100vw - 2 * var(--s4))); }
  .toast { display: flex; align-items: center; gap: var(--s3);
    padding: var(--s2) var(--s2) var(--s2) var(--s4);
    border-radius: var(--shape-md); font-size: 13px;
    background: var(--surface-container-highest); color: var(--on-surface);
    box-shadow: 0 6px 18px rgb(0 0 0 / 0.35); }
  .toast.bad { background: var(--error-container); color: var(--on-error-container); }
  .toast span { flex: 1; }
</style>
