<script lang="ts">
  import { api } from '../api/client';
  import { librariesState, loadLibraries, upsertLibrary, removeLibrary } from '../stores/libraries.svelte';
  import Icon from '../components/Icon.svelte';
  import Dialog from '../components/Dialog.svelte';
  import QrCode from '../components/QrCode.svelte';
  import { t, tn } from '../i18n';
  import { formatDate } from '../utils/format';
  import { i18nState } from '../i18n';
  import { sessionState } from '../stores/session.svelte';
  import type { Library } from '../api/types';

  let addOpen = $state(false);
  let editing = $state<Library | null>(null);
  let deleting = $state<Library | null>(null);
  let qrFor = $state<Library | null>(null);

  const isAdmin = $derived(sessionState.openMode || sessionState.user?.role === 'admin');

  // Add dialog state
  let name = $state('');
  let path = $state('');
  let inpx = $state('');
  let firstAuthorOnly = $state(false);
  let skipDeleted = $state(false);
  let isDefault = $state(false);
  let browsing = $state(false);
  let browsePath = $state('');
  let browseEntries = $state<{ name: string; dir: boolean; size: number }[]>([]);

  async function browse(p: string) {
    const res = await api.fs(p);
    browsePath = res.path;
    browseEntries = res.entries;
  }

  function openBrowser() {
    browsing = true;
    browse('');
  }
  function pickEntry(e: { name: string; dir: boolean }) {
    const full = browsePath ? `${browsePath}/${e.name}` : e.name;
    if (e.dir) { browse(full); return; }
    if (e.name.endsWith('.inpx')) { inpx = full; path = browsePath; }
    browsing = false;
  }

  async function submitAdd() {
    const lib = await api.createLibrary({ name, path, inpx: inpx || undefined, firstAuthorOnly, skipDeleted, isDefault });
    upsertLibrary(lib);
    addOpen = false;
    name = ''; path = ''; inpx = ''; firstAuthorOnly = false; skipDeleted = false; isDefault = false;
  }

  async function saveEdit() {
    if (!editing) return;
    const lib = await api.updateLibrary(editing.id, editing);
    upsertLibrary(lib);
    editing = null;
  }

  async function confirmDelete() {
    if (!deleting) return;
    await api.deleteLibrary(deleting.id);
    removeLibrary(deleting.id);
    deleting = null;
  }

  async function reimport(lib: Library, mode: 'full' | 'new') {
    await api.importLibrary(lib.id, mode);
  }

  $effect(() => { loadLibraries(); });
</script>

<main class="libraries-page">
  <div class="head">
    <h1>{t('libraries.title')}</h1>
    {#if isAdmin}
      <button type="button" class="primary" onclick={() => (addOpen = true)}><Icon name="plus" size={16} />{t('libraries.add')}</button>
    {/if}
  </div>

  <div class="grid">
    {#each librariesState.items as lib (lib.id)}
      <div class="card">
        <div class="card-head">
          <span class="dot" style="background:{lib.status.state === 'error' ? 'var(--danger)' : lib.status.state === 'importing' ? 'var(--amber)' : '#2E7D4F'}"></span>
          <h2>{lib.name}</h2>
          {#if lib.isDefault}<span class="badge">default</span>{/if}
        </div>
        <div class="stats">
          <span>{lib.bookCount.toLocaleString()} {tn('libraries.books', lib.bookCount)}</span>
          <span>{lib.authorCount.toLocaleString()} {tn('libraries.authors', lib.authorCount)}</span>
          <span>{lib.seriesCount.toLocaleString()} {tn('libraries.series', lib.seriesCount)}</span>
        </div>
        <div class="muted">
          {lib.importedAt ? `${t('libraries.imported')} ${formatDate(lib.importedAt.slice(0, 10), i18nState.lang)}` : t('libraries.never')}
        </div>
        {#if lib.status.state === 'importing'}
          <div class="bar"><div class="fill" style="width:{Math.round((lib.status.progress ?? 0) * 100)}%"></div></div>
          <div class="muted small">{lib.status.message ?? t('libraries.status.importing')}</div>
        {/if}
        <div class="opds-row">
          <button type="button" class="link-btn" onclick={() => (qrFor = lib)}><Icon name="opds" size={14} />{t('libraries.opds')}</button>
          <a href={lib.opdsUrl} target="_blank" rel="noreferrer"><Icon name="external" size={14} /></a>
        </div>
        {#if isAdmin}
          <div class="actions">
            <div class="reimport">
              <button type="button" onclick={() => reimport(lib, 'new')}>{t('libraries.reimport')}: {t('libraries.reimport.new')}</button>
              <button type="button" onclick={() => reimport(lib, 'full')}>{t('libraries.reimport.full')}</button>
            </div>
            <button type="button" onclick={() => (editing = { ...lib })}>{t('libraries.edit')}</button>
            <button type="button" class="danger" onclick={() => (deleting = lib)}>{t('libraries.delete')}</button>
          </div>
        {/if}
      </div>
    {/each}
  </div>
</main>

<Dialog open={addOpen} titleId="add-lib-title" title={t('libraries.addDialog.title')} onClose={() => (addOpen = false)} width={480}>
  <form class="form" onsubmit={(e) => { e.preventDefault(); submitAdd(); }}>
    <label>{t('libraries.addDialog.name')}<input type="text" bind:value={name} required /></label>
    <label>{t('libraries.addDialog.folder')}<input type="text" bind:value={path} readonly /></label>
    <label>{t('libraries.addDialog.inpx')}
      <div class="row">
        <input type="text" bind:value={inpx} readonly />
        <button type="button" onclick={openBrowser}>{t('libraries.addDialog.browse')}</button>
      </div>
    </label>
    <label class="checkbox"><input type="checkbox" bind:checked={firstAuthorOnly} />{t('libraries.addDialog.firstAuthorOnly')}</label>
    <label class="checkbox"><input type="checkbox" bind:checked={skipDeleted} />{t('libraries.addDialog.skipDeleted')}</label>
    <label class="checkbox"><input type="checkbox" bind:checked={isDefault} />{t('libraries.addDialog.isDefault')}</label>
    <div class="footer">
      <button type="button" onclick={() => (addOpen = false)}>{t('common.cancel')}</button>
      <button type="submit" class="primary">{t('common.save')}</button>
    </div>
  </form>
</Dialog>

<Dialog open={browsing} titleId="fs-title" title={t('libraries.addDialog.browse')} onClose={() => (browsing = false)} width={420}>
  <div class="fs">
    <div class="fs-path">/{browsePath}</div>
    {#if browsePath}
      <button type="button" class="fs-row" onclick={() => browse(browsePath.split('/').slice(0, -1).join('/'))}>..</button>
    {/if}
    {#each browseEntries as e (e.name)}
      <button type="button" class="fs-row" onclick={() => pickEntry(e)}>
        <Icon name={e.dir ? 'folder' : 'file'} size={16} />{e.name}
      </button>
    {/each}
  </div>
</Dialog>

{#if editing}
  <Dialog open={true} titleId="edit-lib-title" title={t('libraries.edit')} onClose={() => (editing = null)} width={420}>
    <form class="form" onsubmit={(e) => { e.preventDefault(); saveEdit(); }}>
      <label>{t('libraries.addDialog.name')}<input type="text" bind:value={editing.name} /></label>
      <label class="checkbox"><input type="checkbox" bind:checked={editing.firstAuthorOnly} />{t('libraries.addDialog.firstAuthorOnly')}</label>
      <label class="checkbox"><input type="checkbox" bind:checked={editing.skipDeleted} />{t('libraries.addDialog.skipDeleted')}</label>
      <label class="checkbox"><input type="checkbox" bind:checked={editing.isDefault} />{t('libraries.addDialog.isDefault')}</label>
      <div class="footer">
        <button type="button" onclick={() => (editing = null)}>{t('common.cancel')}</button>
        <button type="submit" class="primary">{t('common.save')}</button>
      </div>
    </form>
  </Dialog>
{/if}

{#if deleting}
  <Dialog open={true} titleId="del-lib-title" title={t('libraries.delete')} onClose={() => (deleting = null)} width={420}>
    <div class="confirm">
      <p>{t('libraries.deleteConfirm', { name: deleting.name })}</p>
      <div class="footer">
        <button type="button" onclick={() => (deleting = null)}>{t('common.cancel')}</button>
        <button type="button" class="danger-btn" onclick={confirmDelete}>{t('common.delete')}</button>
      </div>
    </div>
  </Dialog>
{/if}

{#if qrFor}
  <Dialog open={true} titleId="qr-title" title={t('libraries.opds')} onClose={() => (qrFor = null)} width={320}>
    <div class="qr-wrap">
      <QrCode text={`${location.origin}${qrFor.opdsUrl}`} size={220} />
      <code>{location.origin}{qrFor.opdsUrl}</code>
    </div>
  </Dialog>
{/if}

<style>
  .libraries-page { flex-grow: 1; overflow-y: auto; padding: 28px; background: var(--surface); }
  .head { display: flex; align-items: center; justify-content: space-between; margin-bottom: 20px; }
  h1 { margin: 0; font-family: var(--font-display); font-size: 26px; }
  button.primary { display: flex; align-items: center; gap: 8px; height: 40px; padding: 0 16px; border: none; border-radius: 8px; background: var(--accent); color: #fff; font-size: 14px; font-weight: 500; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 16px; }
  .card { display: flex; flex-direction: column; gap: 8px; padding: 18px; border-radius: 12px; border: 1px solid var(--line); background: var(--surface-alt); }
  .card-head { display: flex; align-items: center; gap: 8px; }
  .dot { width: 8px; height: 8px; border-radius: 4px; }
  h2 { margin: 0; font-size: 16px; flex-grow: 1; }
  .badge { font-size: 11px; padding: 2px 6px; border-radius: 4px; background: var(--accent-soft); color: var(--accent-soft-ink); }
  .stats { display: flex; gap: 12px; font-size: 13px; color: var(--muted-2); }
  .muted { color: var(--muted); font-size: 13px; }
  .muted.small { font-size: 12px; }
  .bar { height: 4px; border-radius: 2px; background: var(--line); overflow: hidden; }
  .fill { height: 100%; background: var(--amber); }
  .opds-row { display: flex; align-items: center; gap: 8px; }
  .link-btn { all: unset; display: flex; align-items: center; gap: 6px; color: var(--accent); font-size: 13px; cursor: pointer; }
  .actions { display: flex; flex-wrap: wrap; gap: 6px; margin-top: 6px; }
  .actions button, .reimport button { height: 30px; padding: 0 10px; border-radius: 6px; border: 1px solid var(--border); background: var(--surface); font-size: 12px; white-space: nowrap; }
  .actions .danger { color: var(--danger); }
  .reimport { display: flex; flex-wrap: wrap; gap: 4px; }
  @media (max-width: 900px) {
    .libraries-page { padding: 16px; }
    .head { flex-wrap: wrap; gap: 10px; }
    h1 { font-size: 22px; }
    button.primary { white-space: nowrap; }
    .grid { grid-template-columns: 1fr; }
  }
  .form { padding: 4px 24px 24px; display: flex; flex-direction: column; gap: 12px; }
  .form label { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--muted-2); }
  .form input[type='text'] { height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; }
  .form .row { display: flex; gap: 8px; }
  .form .row input { flex-grow: 1; }
  .checkbox { flex-direction: row !important; align-items: center; gap: 8px !important; font-size: 14px !important; }
  .checkbox input { width: 16px; height: 16px; accent-color: var(--accent); }
  .footer { display: flex; justify-content: flex-end; gap: 8px; margin-top: 8px; }
  .footer button { height: 38px; padding: 0 16px; border-radius: 7px; border: 1px solid var(--border); background: var(--surface); font-size: 14px; }
  .footer .primary, .footer .danger-btn { border: none; color: #fff; }
  .footer .primary { background: var(--accent); }
  .footer .danger-btn { background: var(--danger); }
  .fs { padding: 4px 24px 24px; display: flex; flex-direction: column; gap: 2px; max-height: 320px; overflow-y: auto; }
  .fs-path { font-size: 12px; color: var(--muted); margin-bottom: 8px; }
  .fs-row { all: unset; display: flex; align-items: center; gap: 10px; padding: 8px 6px; border-radius: 6px; font-size: 14px; cursor: pointer; }
  .fs-row:hover { background: var(--surface-hover); }
  .confirm { padding: 4px 24px 24px; }
  .qr-wrap { padding: 4px 24px 24px; display: flex; flex-direction: column; align-items: center; gap: 12px; }
  .qr-wrap code { font-size: 12px; color: var(--muted); word-break: break-all; text-align: center; }
</style>
