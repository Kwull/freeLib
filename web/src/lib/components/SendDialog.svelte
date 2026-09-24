<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import { api } from '../api/client';
  import type { Book, Device } from '../api/types';
  import { devicesState } from '../stores/devices.svelte';
  import { fillFileNameTemplate } from '../utils/fileNameTemplate';
  import { t } from '../i18n';
  import { showToast } from '../stores/toast.svelte';
  import { clear as clearSelection } from '../stores/selection.svelte';
  import { navigate } from '../router.svelte';

  let { lib, bookIds, open, onClose }: { lib: number; bookIds: number[]; open: boolean; onClose: () => void } = $props();

  let books = $state<Book[]>([]);
  let deviceId = $state<number | null>(null);
  let target = $state('');
  let fileName = $state('%a - %s %n - %b');
  let generateCover = $state(true);
  let joinSeries = $state(false);
  let sending = $state(false);

  $effect(() => {
    if (!open || bookIds.length === 0) return;
    books = [];
    Promise.all(bookIds.slice(0, 30).map((id) => api.book(lib, id))).then((list) => { books = list; });
    if (devicesState.items.length && deviceId === null) deviceId = devicesState.items[0].id;
  });

  $effect(() => {
    const dev = devicesState.items.find((d) => d.id === deviceId);
    if (dev) {
      target = dev.target ?? '';
      fileName = dev.fileName;
      generateCover = dev.options.createCover !== 'never';
      joinSeries = dev.options.joinSeries;
    }
  });

  const device = $derived(devicesState.items.find((d) => d.id === deviceId) ?? null);
  const titleLine = $derived(books.map((b) => b.title).join(', '));
  const preview = $derived.by(() => {
    if (!books[0] || !device) return '';
    const name = fillFileNameTemplate(fileName, books[0], { transliterate: device.options.transliterate });
    const ext = device.format === 'original' ? books[0].ext : device.format;
    return `${name}.${ext}`;
  });

  function destLabel(d: Device): string {
    if (d.kind === 'email') return 'Email address';
    if (d.kind === 'folder') return 'Folder';
    return 'Save to';
  }
  function actionLabel(d: Device | null): string {
    if (!d) return '';
    const kind = d.kind === 'email' ? 'send' : d.kind === 'folder' ? 'export' : 'download';
    const label = t(`send.action.${kind}`, { count: bookIds.length });
    return bookIds.length === 1 ? label.replace(/\bbooks\b/, 'book').replace(/\bкниг\b/, 'книгу') : label;
  }

  async function submit() {
    if (!device) return;
    sending = true;
    try {
      await api.send({
        library: lib,
        books: bookIds,
        device: device.id,
        target: target || undefined,
        fileName,
        options: {
          createCover: generateCover ? 'missing' : 'never',
          joinSeries,
        },
      });
      showToast(t('send.background'));
      clearSelection(lib);
      onClose();
    } catch (err) {
      showToast(String(err), 'error');
    } finally {
      sending = false;
    }
  }
</script>

<Dialog {open} titleId="send-title" title={bookIds.length === 1 ? t('send.titleOne') : t('send.title', { count: bookIds.length })} {onClose} width={760}>
  <p class="subtitle">{titleLine}</p>

  <fieldset class="devices">
    <legend>{t('send.device')}</legend>
    {#each devicesState.items as d (d.id)}
      <button
        type="button"
        aria-pressed={d.id === deviceId}
        class="device-card"
        class:on={d.id === deviceId}
        onclick={() => (deviceId = d.id)}
      >
        <span class="row">
          <span class="name">{d.name}</span>
          {#if d.id === deviceId}<Icon name="check" size={18} />{/if}
        </span>
        <span class="how">{d.kind === 'email' ? 'Send by email' : d.kind === 'folder' ? 'Copy to server folder' : 'Download'}</span>
        <span class="fmt">{d.format.toUpperCase()}</span>
      </button>
    {/each}
  </fieldset>

  {#if device}
    <div class="options">
      <label>{destLabel(device)}
        <input type="text" bind:value={target} />
      </label>
      <label>{t('send.fileName')}
        <input type="text" bind:value={fileName} />
      </label>
      <div class="preview">{t('send.preview')}: <span>{preview}</span></div>
      <label class="checkbox"><input type="checkbox" bind:checked={generateCover} />{t('send.generateCover')}</label>
      <label class="checkbox"><input type="checkbox" bind:checked={joinSeries} />{t('send.joinSeries')}</label>
    </div>

    <div class="note">
      <Icon name="settings" size={16} />
      {t('send.formattingNote')} ·
      <button type="button" class="link-btn" onclick={() => navigate('/settings/devices')}>{t('send.editProfile')}</button>
    </div>
  {/if}

  <div class="footer">
    <span class="bg-note">{t('send.background')}</span>
    <button type="button" class="secondary" onclick={onClose}>{t('send.cancel')}</button>
    <button type="button" class="primary" disabled={!device || sending} onclick={submit}>{actionLabel(device)}</button>
  </div>
</Dialog>

<style>
  .subtitle { margin: 4px 24px 0; font-size: 14px; color: var(--muted); }
  fieldset.devices { border: none; margin: 18px 24px 0; padding: 0; display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; }
  legend { padding: 0 0 8px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; }
  .device-card {
    display: flex; flex-direction: column; align-items: flex-start; gap: 4px; min-height: 84px; padding: 12px 14px;
    border-radius: 10px; border: 1px solid var(--border); background: var(--surface); text-align: left; cursor: pointer;
  }
  .device-card.on { border: 2px solid var(--accent); background: var(--accent-soft); padding: 11px 13px; }
  .row { display: flex; width: 100%; justify-content: space-between; align-items: center; font-size: 15px; font-weight: 600; color: var(--ink); }
  .how { font-size: 13px; color: var(--muted); }
  .fmt { margin-top: auto; font-size: 11px; font-weight: 600; color: var(--muted-2); padding: 2px 6px; border-radius: 4px; background: var(--surface-hover); }
  .options { margin: 18px 24px 0; padding: 16px; border-radius: 10px; background: var(--page); display: grid; grid-template-columns: repeat(2, minmax(0,1fr)); gap: 14px 20px; }
  .options label { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--muted-2); }
  .options input[type='text'] { height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; font: inherit; font-size: 14px; color: var(--ink); background: var(--surface); }
  .preview { grid-column: span 2; font-size: 12px; color: var(--muted); }
  .preview span { color: var(--ink); }
  .checkbox { flex-direction: row; align-items: center; gap: 8px; font-size: 14px; }
  .checkbox input { width: 16px; height: 16px; accent-color: var(--accent); }
  .note { padding: 12px 24px 0; display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--muted); }
  .link-btn { all: unset; color: var(--accent); cursor: pointer; }
  .footer { margin-top: 20px; padding: 14px 24px; border-top: 1px solid var(--line); display: flex; align-items: center; gap: 10px; }
  .bg-note { font-size: 13px; color: var(--muted); flex-grow: 1; }
  button.secondary { display: flex; align-items: center; height: 40px; padding: 0 16px; border-radius: 8px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 14px; }
  button.primary { display: flex; align-items: center; gap: 8px; height: 40px; padding: 0 18px; border: none; border-radius: 8px; background: var(--accent); color: #fff; font-size: 14px; font-weight: 500; }
  button.primary:disabled { opacity: .5; cursor: default; }
</style>
