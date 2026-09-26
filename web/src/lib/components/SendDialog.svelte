<script lang="ts">
  import Dialog from './Dialog.svelte';
  import Icon from './Icon.svelte';
  import { api, errorText } from '../api/client';
  import type { Book, Device } from '../api/types';
  import { devicesState, defaultDevice } from '../stores/devices.svelte';
  import { getPref, setPref } from '../stores/prefs.svelte';
  import { watchJob } from '../stores/jobs.svelte';
  import { fillFileNameTemplate } from '../utils/fileNameTemplate';
  import { t, tn } from '../i18n';
  import { showToast } from '../stores/toast.svelte';
  import { clear as clearSelection } from '../stores/selection.svelte';
  import { navigate } from '../router.svelte';
  import HandoffPanel from './HandoffPanel.svelte';
  import { openInBooks } from '../stores/devices.svelte';
  import { isIOS } from '../utils/platform';

  let { lib, bookIds, open, onClose, device: initialDevice, seriesIds = [] }: {
    lib: number; bookIds: number[]; open: boolean; onClose: () => void;
    /** preselected device (e.g. the details pane's "Send to Kindle") */
    device?: number;
    /** "Send whole series": these series' books in reading order (the server resolves them) */
    seriesIds?: number[];
  } = $props();

  const ios = isIOS();
  let phone = $state(false);
  let seriesNames = $state<string[]>([]);

  let books = $state<Book[]>([]);
  let deviceId = $state<number | null>(null);
  let target = $state('');
  let fileName = $state('%a - %s %n - %b');
  let generateCover = $state(true);
  let joinSeries = $state(false);
  let sending = $state(false);

  $effect(() => {
    if (!open || (bookIds.length === 0 && seriesIds.length === 0)) return;
    books = [];
    if (seriesIds.length) {
      // the series' first books: for the file name preview and the names in the subtitle
      Promise.all(seriesIds.slice(0, 5).map((s) => api.books(lib, { series: s, limit: 30 })))
        .then((res) => {
          books = res.flatMap((r) => r.books);
          seriesNames = [...new Set(books.map((b) => b.series?.name).filter((n): n is string => !!n))];
        })
        .catch(() => {});
    } else {
      Promise.all(bookIds.slice(0, 30).map((id) => api.book(lib, id))).then((list) => { books = list; }).catch(() => {});
    }
    if (devicesState.items.length && deviceId === null) deviceId = initialDevice ?? defaultDevice()?.id ?? devicesState.items[0].id;
  });

  $effect(() => {
    const dev = devicesState.items.find((d) => d.id === deviceId);
    if (dev) {
      // the address typed on the last send to this device, else the device's own
      target = getPref<Record<string, string>>('sendTargets', {})[String(dev.id)] ?? dev.target ?? '';
      fileName = dev.fileName;
      generateCover = dev.options.createCover !== 'never';
      joinSeries = dev.options.joinSeries;
    }
  });

  const device = $derived(devicesState.items.find((d) => d.id === deviceId) ?? null);
  const needsAddress = $derived(device?.kind === 'email' && !target.trim());
  const isSeries = $derived(seriesIds.length > 0);
  const titleLine = $derived(
    isSeries
      ? t('send.seriesSubtitle', { names: seriesNames.map((n) => `«${n}»`).join(', ') })
      : books.slice(0, 3).map((b) => `«${b.title}»`).join(', ') + (bookIds.length > 3 ? ` ${t('send.andMore', { count: bookIds.length - 3 })}` : ''),
  );
  const single = $derived(!isSeries && bookIds.length === 1);
  // on iPhone/iPad, the Apple Books device opens the book right here
  const iosBooks = $derived(ios && single && device?.kind === 'download' && device?.preset === 'apple-books');
  const preview = $derived.by(() => {
    if (!books[0] || !device) return '';
    const name = fillFileNameTemplate(fileName, books[0], { transliterate: device.options.transliterate });
    const ext = device.format === 'original' ? books[0].ext : device.format;
    return `${name}.${ext}`;
  });

  function destLabel(d: Device): string {
    if (d.kind === 'email') return t('send.dest.email');
    if (d.kind === 'folder') return t('send.dest.folder');
    return t('send.dest.download');
  }
  function actionLabel(d: Device | null): string {
    if (!d) return '';
    if (iosBooks) return t('books.openInBooks');
    if (isSeries) return t('send.actionSeries');
    const kind = d.kind === 'email' ? 'send' : d.kind === 'folder' ? 'export' : 'download';
    return tn(`send.action.${kind}`, bookIds.length);
  }

  async function submit() {
    if (!device) return;
    sending = true;
    if (iosBooks) {
      try {
        await openInBooks(lib, bookIds[0], device);
        onClose();
      } catch (err) {
        showToast(errorText(err), 'error');
      } finally {
        sending = false;
      }
      return;
    }
    try {
      const job = await api.send({
        library: lib,
        books: isSeries ? [] : bookIds,
        series: isSeries ? seriesIds : undefined,
        device: device.id,
        target: target || undefined,
        fileName,
        options: {
          createCover: generateCover ? 'missing' : 'never',
          joinSeries,
        },
      });
      watchJob(job);
      if (device.kind === 'email' && target && target !== (device.target ?? '')) {
        setPref('sendTargets', { ...getPref<Record<string, string>>('sendTargets', {}), [String(device.id)]: target });
      }
      showToast(t('send.started', { device: device.name }));
      clearSelection(lib);
      onClose();
    } catch (err) {
      showToast(errorText(err), 'error');
    } finally {
      sending = false;
    }
  }
</script>

<Dialog {open} titleId="send-title" title={isSeries ? t('send.titleSeries') : tn('send.title', bookIds.length)} {onClose} width={760}>
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
        <span class="how">{d.kind === 'email' ? t('send.how.email') : d.kind === 'folder' ? t('send.how.folder') : t('send.how.download')}</span>
        <span class="fmt">{d.format.toUpperCase()}</span>
      </button>
    {/each}
  </fieldset>

  {#if device}
    <div class="options">
      {#if device.kind !== 'download'}
        <label>{destLabel(device)}
          <input type="text" bind:value={target} placeholder={device.kind === 'email' ? 'name@kindle.com' : ''} />
        </label>
      {/if}
      <label class:wide={device.kind === 'download'}>{t('send.fileName')}
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
    {#if device.kind === 'email' && (isSeries || bookIds.length > 1)}
      <div class="note"><Icon name="send" size={16} />{t('send.batchNote', { count: 25 })}</div>
    {/if}
    {#if phone && single}
      <div class="phone"><HandoffPanel {lib} bookId={bookIds[0]} {device} /></div>
    {/if}
  {/if}

  <div class="footer">
    {#if single && !ios}
      <button type="button" class="secondary" data-testid="send-dialog-phone" aria-pressed={phone} onclick={() => (phone = !phone)}><Icon name="phone" size={16} />{t('phone.action')}</button>
    {/if}
    <span class="bg-note" class:warn={needsAddress}>{needsAddress ? t('send.needAddress') : t('send.background')}</span>
    <button type="button" class="secondary" onclick={onClose}>{t('send.cancel')}</button>
    <!-- svelte-ignore a11y_autofocus -->
    <button type="button" class="primary" autofocus disabled={!device || sending || needsAddress} onclick={submit}>{actionLabel(device)}</button>
  </div>
</Dialog>

<style>
  .subtitle { margin: 4px 24px 0; font-size: 14px; color: var(--muted); }
  fieldset.devices { border: none; margin: 18px 24px 0; padding: 0; display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 10px; }
  @media (max-width: 700px) {
    fieldset.devices { grid-template-columns: repeat(2, minmax(0, 1fr)); }
    .options { grid-template-columns: 1fr !important; }
    .options label.wide, .preview { grid-column: auto !important; }
    .bg-note:not(.warn) { display: none; }
  }
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
  .options label.wide { grid-column: span 2; }
  .preview { grid-column: span 2; font-size: 12px; color: var(--muted); }
  .preview span { color: var(--ink); }
  .options label.checkbox { flex-direction: row; align-items: center; gap: 8px; font-size: 14px; }
  .checkbox input { width: 16px; height: 16px; accent-color: var(--accent); }
  .note { padding: 12px 24px 0; display: flex; align-items: center; gap: 6px; font-size: 13px; color: var(--muted); }
  .link-btn { all: unset; color: var(--accent); cursor: pointer; }
  .footer {
    margin-top: 20px; padding: 14px 24px; border-top: 1px solid var(--line); display: flex; align-items: center; gap: 10px;
    position: sticky; bottom: 0; background: var(--surface);
  }
  .bg-note { font-size: 13px; color: var(--muted); flex-grow: 1; }
  .phone { margin: 14px 24px 0; }
  button.secondary[aria-pressed='true'] { border-color: var(--accent); color: var(--accent); }
  button.secondary { gap: 6px; }
  .footer button { white-space: nowrap; flex-shrink: 0; }
  .bg-note.warn { color: var(--amber); }
  button.secondary { display: flex; align-items: center; height: 40px; padding: 0 16px; border-radius: 8px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 14px; }
  button.primary { display: flex; align-items: center; gap: 8px; height: 40px; padding: 0 18px; border: none; border-radius: 8px; background: var(--accent); color: #fff; font-size: 14px; font-weight: 500; }
  button.primary:disabled { opacity: .5; cursor: default; }
</style>
