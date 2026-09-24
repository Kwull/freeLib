<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let {
    open, titleId, title, onClose, width = 640, children,
  }: { open: boolean; titleId: string; title: string; onClose: () => void; width?: number; children: Snippet } = $props();

  let dialogEl: HTMLDivElement | undefined = $state();
  let lastFocused: HTMLElement | null = null;

  $effect(() => {
    if (open) {
      lastFocused = document.activeElement as HTMLElement;
      queueMicrotask(() => dialogEl?.querySelector<HTMLElement>('[autofocus], button, input, a[href]')?.focus());
    } else {
      lastFocused?.focus();
    }
  });

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); onClose(); return; }
    if (e.key === 'Tab' && dialogEl) {
      const focusables = dialogEl.querySelectorAll<HTMLElement>('button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])');
      if (focusables.length === 0) return;
      const first = focusables[0], last = focusables[focusables.length - 1];
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
    }
  }
</script>

{#if open}
  <div class="scrim" onclick={onClose} role="presentation">
    <div
      bind:this={dialogEl}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      tabindex="-1"
      class="dialog"
      style="width: min({width}px, 92vw)"
      onclick={(e) => e.stopPropagation()}
      onkeydown={onKeydown}
    >
      <div class="head">
        <h2 id={titleId}>{title}</h2>
        <button type="button" aria-label="Close" class="close" onclick={onClose}><Icon name="close" size={18} /></button>
      </div>
      {@render children()}
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed; inset: 0; background: var(--scrim); display: flex; align-items: center; justify-content: center;
    z-index: 100; padding: 24px;
  }
  .dialog {
    background: var(--surface); border-radius: 12px; display: flex; flex-direction: column; overflow: hidden;
    max-height: 90vh;
  }
  .head { padding: 22px 24px 14px; display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; flex-shrink: 0; }
  h2 { margin: 0; font-family: var(--font-display); font-size: 22px; font-weight: 600; }
  .close { width: 36px; height: 36px; border: none; border-radius: 8px; background: transparent; color: var(--muted-2); display: flex; align-items: center; justify-content: center; }
  .close:hover { background: var(--surface-hover); }
</style>
