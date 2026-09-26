<script lang="ts">
  // "Send to my phone": a 15-minute link to one book as a QR code plus a short URL.
  import QrCode from './QrCode.svelte';
  import Icon from './Icon.svelte';
  import { api, errorText } from '../api/client';
  import type { Device, HandoffLink } from '../api/types';
  import { t } from '../i18n';
  import { isLocalHost } from '../utils/platform';
  import { showToast } from '../stores/toast.svelte';

  let { lib, bookId, device = null }: { lib: number; bookId: number; device?: Device | null } = $props();

  let link = $state<HandoffLink | null>(null);
  let error = $state<string | null>(null);
  let now = $state(Date.now());
  let tick = $state(0);

  $effect(() => {
    tick;
    link = null;
    error = null;
    let cancelled = false;
    api.handoff({ library: lib, book: bookId, device: device?.id })
      .then((l) => { if (!cancelled) link = l; })
      .catch((e) => { if (!cancelled) error = errorText(e); });
    return () => { cancelled = true; };
  });

  $effect(() => {
    const timer = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  const left = $derived(link ? Math.max(0, Math.round((Date.parse(link.expiresAt) - now) / 1000)) : 0);
  const leftText = $derived(`${Math.floor(left / 60)}:${String(left % 60).padStart(2, '0')}`);
  const expired = $derived(!!link && left === 0);
  const shortUrl = $derived(link ? link.absoluteUrl.replace(/^https?:\/\//, '') : '');

  async function copy() {
    if (!link) return;
    try {
      await navigator.clipboard.writeText(link.absoluteUrl);
      showToast(t('phone.copied'));
    } catch { /* clipboard unavailable (http): the URL is selectable */ }
  }
</script>

<div class="handoff" data-testid="handoff-panel">
  {#if error}
    <p class="error">{error}</p>
  {:else if !link}
    <p class="muted">{t('phone.creating')}</p>
  {:else}
    <div class="qr-wrap" class:dim={expired}>
      <QrCode text={link.absoluteUrl} size={196} />
    </div>
    <div class="info">
      <p class="what">{t('phone.what', { title: link.title, format: link.format.toUpperCase(), device: device?.name ?? 'Apple Books' })}</p>
      <p class="scan">{t('phone.scan')}</p>
      <div class="url-row">
        <code class="url" data-testid="handoff-url" title={link.absoluteUrl}>{shortUrl}</code>
        <button type="button" class="icon-btn" aria-label={t('phone.copy')} title={t('phone.copy')} onclick={copy}><Icon name="link" size={16} /></button>
      </div>
      {#if expired}
        <p class="warn">{t('phone.expired')}</p>
        <button type="button" class="secondary" onclick={() => tick++}><Icon name="refresh" size={16} />{t('phone.newLink')}</button>
      {:else}
        <p class="muted" data-testid="handoff-expires">{t('phone.expiresIn', { time: leftText, count: link.maxUses })}</p>
      {/if}
      <p class="muted small">{t('phone.security')}</p>
      {#if isLocalHost()}<p class="warn small">{t('phone.localhost')}</p>{/if}
    </div>
  {/if}
</div>

<style>
  .handoff { display: flex; gap: 20px; align-items: flex-start; padding: 16px; border-radius: 10px; background: var(--page); }
  .qr-wrap { flex-shrink: 0; border-radius: 10px; box-shadow: 0 1px 0 var(--line); background: #fff; }
  .qr-wrap.dim { opacity: .25; }
  .info { display: flex; flex-direction: column; gap: 8px; min-width: 0; }
  .info p { margin: 0; }
  .what { font-weight: 600; color: var(--ink); overflow-wrap: anywhere; }
  .scan { color: var(--muted-2); font-size: 14px; }
  .url-row { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .url { font-size: 13px; padding: 6px 8px; border-radius: 6px; background: var(--surface); border: 1px solid var(--border); color: var(--ink); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; user-select: all; min-width: 0; }
  .icon-btn { width: 32px; height: 32px; flex-shrink: 0; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); color: var(--ink); display: flex; align-items: center; justify-content: center; }
  .muted { color: var(--muted); font-size: 13px; }
  .small { font-size: 12px; }
  .warn { color: var(--amber); font-size: 13px; }
  .error { color: var(--danger); margin: 0; }
  .secondary { align-self: flex-start; display: flex; align-items: center; gap: 6px; height: 34px; padding: 0 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface); color: var(--ink); }
  @media (max-width: 560px) {
    .handoff { flex-direction: column; align-items: center; text-align: center; }
    .url-row { justify-content: center; }
  }
</style>
