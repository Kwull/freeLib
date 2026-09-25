<script lang="ts">
  import { api } from '../api/client';
  import type { BookDetail } from '../api/types';
  import Icon from './Icon.svelte';
  import CoverThumb from './CoverThumb.svelte';
  import Rating from './Rating.svelte';
  import { formatSize, formatDate } from '../utils/format';
  import { t, i18nState } from '../i18n';
  import { navigate } from '../router.svelte';
  import { defaultDevice } from '../stores/devices.svelte';
  import { shelvesState } from '../stores/shelves.svelte';

  let { lib, bookId, onSend, onAddShelf }: {
    lib: number; bookId: number | null;
    onSend: (ids: number[], device?: number) => void;
    onAddShelf: (ids: number[]) => void;
  } = $props();

  let detail = $state<BookDetail | null>(null);
  let loading = $state(false);
  let downloadMenuOpen = $state(false);
  let genreNames = $state<Map<number, string>>(new Map());

  $effect(() => {
    api.genres(lib).then((gs) => (genreNames = new Map(gs.map((g) => [g.id, g.name]))));
  });

  $effect(() => {
    if (bookId === null) { detail = null; return; }
    let cancelled = false;
    loading = true;
    api.book(lib, bookId).then((d) => { if (!cancelled) { detail = d; loading = false; } });
    return () => { cancelled = true; };
  });

  async function rate(v: number) {
    if (!detail) return;
    detail = { ...detail, rating: v };
    await api.setRating(lib, detail.id, v);
  }

  const dev = $derived(defaultDevice());
  const shelfNames = $derived.by(() => {
    if (!detail) return [];
    return detail.shelves.map((id) => shelvesState.items.find((s) => s.id === id)?.name).filter(Boolean) as string[];
  });
</script>

<aside aria-label="Book details" class="details">
  {#if !detail}
    <div class="empty">{loading ? t('common.loading') : t('details.noSelection')}</div>
  {:else}
    <div class="top">
      <CoverThumb {lib} bookId={detail.id} title={detail.title} width={112} height={168} />
      <div class="meta">
        <h2>{detail.title}</h2>
        {#each detail.authors as a (a.id)}
          <a href="/l/{lib}/authors/{a.id}" data-link>{a.name}</a>
        {/each}
        {#if detail.series}
          <span class="muted">
            <a href="/l/{lib}/series/{detail.series.id}" data-link>{detail.series.name}</a>
            {#if detail.serno}· #{detail.serno}{/if}
          </span>
        {/if}
      </div>
    </div>

    <div class="actions">
      {#if dev}
        <button type="button" class="primary" onclick={() => onSend([detail!.id], dev.id)}>
          <Icon name="send" size={16} />{t('details.sendToDevice', { device: dev.name })}
        </button>
      {/if}
      <div class="dl-wrap">
        <button type="button" class="secondary icon-only" aria-label={t('details.downloadAs')} onclick={() => (downloadMenuOpen = !downloadMenuOpen)}>
          <Icon name="download" size={16} /><Icon name="chevronDown" size={14} />
        </button>
        {#if downloadMenuOpen}
          <div class="dl-menu" role="menu">
            {#each detail.formats as f (f)}
              <a href={api.fileUrl(lib, detail.id, f)} data-link={false} role="menuitem" onclick={() => (downloadMenuOpen = false)}>{f}</a>
            {/each}
          </div>
        {/if}
      </div>
      <button type="button" class="secondary" onclick={() => navigate(`/l/${lib}/read/${detail!.id}`)}>
        <Icon name="read" size={16} />{t('details.read')}
      </button>
    </div>

    {#if detail.genres.length}
      <div class="chips">
        {#each detail.genres as g (g)}
          <a href="/l/{lib}/genres/{g}" data-link class="chip">{genreNames.get(g) ?? g}</a>
        {/each}
      </div>
    {/if}

    {#if detail.annotation}
      <div class="section">
        <h3>{t('details.annotation')}</h3>
        <!-- Server sends sanitized HTML limited to <p><em><strong><br>. -->
        <p class="annotation">{@html detail.annotation}</p>
      </div>
    {/if}

    <dl class="meta-list">
      <dt>{t('details.language')}</dt><dd>{detail.lang}</dd>
      <dt>{t('details.format')}</dt><dd>{detail.ext.toUpperCase()} · {formatSize(detail.size)}</dd>
      <dt>{t('details.added')}</dt><dd>{formatDate(detail.date, i18nState.lang)}</dd>
      <dt>{t('details.rating')}</dt><dd><Rating value={detail.rating} onChange={rate} /></dd>
      <dt>{t('details.shelves')}</dt>
      <dd class="shelves-cell">
        {#each shelfNames as name (name)}<span class="chip small">{name}</span>{/each}
        <button type="button" class="link-btn" onclick={() => onAddShelf([detail!.id])}>{t('details.addShelf')}</button>
      </dd>
      <dt>{t('details.file')}</dt><dd class="muted ellipsis">{detail.file}</dd>
    </dl>
  {/if}
</aside>

<style>
  .details { width: 360px; flex-shrink: 0; border-left: 1px solid var(--line); background: var(--surface-alt); overflow-y: auto; display: flex; flex-direction: column; }
  .empty { flex-grow: 1; display: flex; align-items: center; justify-content: center; color: var(--muted); font-size: 14px; padding: 24px; text-align: center; }
  .top { padding: 24px 24px 0; display: flex; gap: 16px; }
  .meta { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  h2 { margin: 0; font-family: var(--font-display); font-size: 20px; font-weight: 600; line-height: 1.25; }
  .muted { color: var(--muted); font-size: 13px; }
  .actions { padding: 20px 24px 0; display: flex; gap: 8px; }
  .actions button.primary {
    flex-grow: 1; display: flex; align-items: center; justify-content: center; gap: 8px; height: 40px;
    border: none; border-radius: 8px; background: var(--accent); color: #fff; font-weight: 500; font-size: 14px;
  }
  .actions button.secondary { display: flex; align-items: center; gap: 6px; height: 40px; padding: 0 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface); color: var(--ink); font-size: 14px; }
  .actions button.icon-only { gap: 4px; }
  .dl-wrap { position: relative; }
  .dl-menu { position: absolute; top: 44px; right: 0; background: var(--surface); border: 1px solid var(--line); border-radius: 8px; box-shadow: 0 8px 24px rgba(0,0,0,.15); min-width: 140px; z-index: 10; overflow: hidden; }
  .dl-menu a { display: block; padding: 8px 12px; font-size: 13px; color: var(--ink); text-decoration: none; text-transform: uppercase; }
  .dl-menu a:hover { background: var(--surface-hover); }
  .chips { padding: 16px 24px 0; display: flex; flex-wrap: wrap; gap: 6px; }
  .chip {
    display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border-radius: 13px;
    background: var(--surface-hover); color: var(--muted-2); font-size: 12px; text-decoration: none;
  }
  .chip.small { height: 22px; }
  .section { padding: 18px 24px 0; }
  h3 { margin: 0 0 6px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; }
  .annotation { margin: 0; font-family: var(--font-display); font-size: 14px; line-height: 1.55; color: var(--ink); }
  .meta-list { margin: 18px 24px 24px; padding-top: 14px; border-top: 1px solid var(--line); display: grid; grid-template-columns: 96px minmax(0,1fr); row-gap: 8px; font-size: 13px; }
  .meta-list dt { color: var(--muted); }
  .meta-list dd { margin: 0; }
  .shelves-cell { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .link-btn { all: unset; color: var(--accent); font-size: 12px; cursor: pointer; }
  .link-btn:hover { text-decoration: underline; }
</style>
