<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import { recentPopupTrigger } from '../utils/dismiss';

  let {
    open, titleId, title, onClose, width = 640, children,
  }: { open: boolean; titleId: string; title: string; onClose: () => void; width?: number; children: Snippet } = $props();

  let dialogEl: HTMLDivElement | undefined = $state();

  const FOCUSABLE = 'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';
  function focusables(): HTMLElement[] {
    if (!dialogEl) return [];
    return [...dialogEl.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.getClientRects().length > 0);
  }

  // Focus moves into the dialog when it opens, stays inside while it is open (Tab wraps, focus
  // that escapes is pulled back) and returns to the opener when it closes — also when the
  // dialog is closed by unmounting its owner ({#if …}<Dialog open={true}>). An opener that is
  // gone (a menu item of a closed menu) hands over to that menu's trigger.
  $effect(() => {
    if (!open) return;
    let opener = document.activeElement as HTMLElement | null;
    if (!opener || opener === document.body || !opener.isConnected) opener = recentPopupTrigger();
    queueMicrotask(() => {
      if (!dialogEl) return;
      const first = dialogEl.querySelector<HTMLElement>('[autofocus]:not([disabled])') ?? focusables()[0] ?? dialogEl;
      first.focus();
    });
    const keepInside = (e: FocusEvent) => {
      const to = e.target as Node | null;
      if (!dialogEl || !to || dialogEl.contains(to)) return;
      // another top-layer popup (a menu) opened from inside the dialog is fine
      if ((to as HTMLElement).closest?.('[data-fl-popover]')) return;
      (focusables()[0] ?? dialogEl).focus();
    };
    document.addEventListener('focusin', keepInside);
    return () => {
      document.removeEventListener('focusin', keepInside);
      const back = opener && opener.isConnected ? opener : recentPopupTrigger();
      // after the dialog's DOM is gone
      queueMicrotask(() => {
        const target = back && back.isConnected ? back : null;
        if (target && (document.activeElement === document.body || !document.activeElement || !document.activeElement.isConnected || dialogEl?.contains(document.activeElement))) {
          target.focus();
        }
      });
    };
  });

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'Escape') { e.preventDefault(); onClose(); return; }
    if (e.key === 'Tab' && dialogEl) {
      const list = focusables();
      if (list.length === 0) { e.preventDefault(); return; }
      const first = list[0], last = list[list.length - 1];
      if (e.shiftKey && (document.activeElement === first || document.activeElement === dialogEl)) { e.preventDefault(); last.focus(); }
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
      <div class="body">{@render children()}</div>
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
  .body { overflow-y: auto; min-height: 0; flex: 1 1 auto; display: flex; flex-direction: column; }
  @media (max-width: 700px) {
    .scrim { padding: 10px; align-items: flex-end; }
    .dialog { max-height: calc(100vh - 20px); width: 100% !important; }
  }
  .head { padding: 22px 24px 14px; display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; flex-shrink: 0; }
  h2 { margin: 0; font-family: var(--font-display); font-size: 22px; font-weight: 600; }
  .close { width: 36px; height: 36px; border: none; border-radius: 8px; background: transparent; color: var(--muted-2); display: flex; align-items: center; justify-content: center; }
  .close:hover { background: var(--surface-hover); }
</style>
