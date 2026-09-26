<script lang="ts">
  import { api, errorText } from '../api/client';
  import type { AuthorSummary, BookDetail } from '../api/types';
  import Icon from './Icon.svelte';
  import CoverThumb from './CoverThumb.svelte';
  import Rating from './Rating.svelte';
  import Splitter from './Splitter.svelte';
  import KidsBadge from './KidsBadge.svelte';
  import { formatAvg } from '../utils/ratings';
  import { noteRating } from '../stores/myRatings.svelte';
  import { formatSize, formatDate } from '../utils/format';
  import { t, tn, i18nState } from '../i18n';
  import { navigate } from '../router.svelte';
  import { defaultDevice, devicesState, deviceVerb, deviceCaption, appleBooksDevice, openInBooks } from '../stores/devices.svelte';
  import PhoneDialog from './PhoneDialog.svelte';
  import { dismissable } from '../utils/dismiss';
  import { isIOS } from '../utils/platform';
  import { showToast } from '../stores/toast.svelte';
  import { shelvesState } from '../stores/shelves.svelte';
  import {
    PANE_LIMITS, paneWidth, setPaneWidth, detailsCollapsed, setDetailsCollapsed,
  } from '../stores/layout.svelte';

  let { lib, bookId, onSend, onAddShelf, summary = null, standalone = false }: {
    lib: number; bookId: number | null;
    onSend: (ids: number[], device?: number) => void;
    onAddShelf: (ids: number[]) => void;
    /** shown when no book is selected (author pages) */
    summary?: AuthorSummary | null;
    /** the phone book page: full width, no resize/collapse */
    standalone?: boolean;
  } = $props();

  let detail = $state<BookDetail | null>(null);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let downloadMenuOpen = $state(false);
  let allAuthors = $state(false);
  let allSeries = $state(false);
  let genreNames = $state<Map<number, string>>(new Map());

  $effect(() => {
    const lang = i18nState.lang;
    api.genres(lib, lang).then((gs) => (genreNames = new Map(gs.map((g) => [g.id, g.name])))).catch(() => {});
  });

  $effect(() => {
    allAuthors = false;
    downloadMenuOpen = false;
    deviceMenuOpen = false;
    if (bookId === null) { detail = null; error = null; return; }
    let cancelled = false;
    loading = true;
    error = null;
    api.book(lib, bookId)
      .then((d) => { if (!cancelled) detail = d; })
      .catch((e) => { if (!cancelled) { detail = null; error = errorText(e); } })
      .finally(() => { if (!cancelled) loading = false; });
    return () => { cancelled = true; };
  });
  $effect(() => { summary?.id; allSeries = false; });

  async function copyText(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast(t('tokens.copied'));
    } catch {
      showToast(t('tokens.copyFailed'), 'error');
    }
  }

  async function rate(v: number) {
    if (!detail) return;
    detail = { ...detail, rating: v };
    await api.setRating(lib, detail.id, v);
    noteRating(lib, detail.id, v);
  }

  const dev = $derived(defaultDevice());
  let deviceMenuOpen = $state(false);
  let deviceBtn = $state<HTMLButtonElement | undefined>();
  let downloadBtn = $state<HTMLButtonElement | undefined>();
  let phoneOpen = $state(false);
  let opening = $state(false);
  // on iPhone/iPad the Apple Books device becomes the primary action: "Open in Books"
  const ios = isIOS();
  const apple = $derived(appleBooksDevice());
  const iosBooks = $derived(ios && !!apple && !!detail && detail.formats.includes('epub'));

  async function openBooks() {
    if (!detail || opening) return;
    opening = true;
    try {
      await openInBooks(lib, detail.id, apple);
    } catch (e) {
      showToast(errorText(e), 'error');
    } finally {
      setTimeout(() => (opening = false), 1500);
    }
  }
  const shelfNames = $derived.by(() => {
    if (!detail) return [];
    return detail.shelves.map((id) => shelvesState.items.find((s) => s.id === id)?.name).filter(Boolean) as string[];
  });
  const titleSize = $derived(!detail ? 20 : detail.title.length > 120 ? 15 : detail.title.length > 60 ? 17 : 20);
  const AUTHORS_SHOWN = 3;
  const shownAuthors = $derived(detail ? (allAuthors ? detail.authors : detail.authors.slice(0, AUTHORS_SHOWN)) : []);

  // Resizing / collapsing (desktop only)
  let liveWidth = $state<number | null>(null);
  const width = $derived(liveWidth ?? paneWidth('details'));
  const collapsed = $derived(!standalone && detailsCollapsed());
  const hasContent = $derived(bookId !== null || !!summary);
  const langName = (code: string) => {
    try { return new Intl.DisplayNames([i18nState.lang], { type: 'language' }).of(code) ?? code; } catch { return code; }
  };
  const years = $derived(summary && summary.firstDate ? `${summary.firstDate.slice(0, 4)}–${summary.lastDate.slice(0, 4)}` : '');
  const SERIES_SHOWN = 12;
</script>

{#if standalone || hasContent}
  {#if collapsed}
    <aside class="rail" aria-label={t('details.title')}>
      <button type="button" class="rail-btn" aria-label={t('details.show')} title={t('details.show')} onclick={() => setDetailsCollapsed(false)}>
        <Icon name="chevronLeft" size={16} />
      </button>
      <span class="rail-label">{t('details.title')}</span>
    </aside>
  {:else}
    {#if !standalone}
      <Splitter
        value={width}
        min={PANE_LIMITS.details.min}
        max={PANE_LIMITS.details.max}
        side="after"
        label={t('layout.resizeDetails')}
        onInput={(v) => (liveWidth = v)}
        onCommit={(v) => { setPaneWidth('details', v); liveWidth = null; }}
        onReset={() => { setPaneWidth('details', null); liveWidth = null; }}
      />
    {/if}
    <aside aria-label={t('details.title')} class="details" class:standalone style:--w="{width}px">
      {#if !standalone}
        <button type="button" class="collapse" aria-label={t('details.hide')} title={t('details.hide')} onclick={() => setDetailsCollapsed(true)}>
          <Icon name="chevronRight" size={16} />
        </button>
      {/if}
      {#if bookId !== null && !detail}
        <div class="empty">
          {#if error}
            <p>{t('common.error')}: {error}</p>
          {:else}
            <div class="sk-cover"></div><div class="sk-line"></div><div class="sk-line short"></div>
          {/if}
        </div>
      {:else if detail}
        <h2 class="book-title" style:font-size="{titleSize}px">{detail.title}</h2>
        <div class="top">
          <CoverThumb {lib} bookId={detail.id} title={detail.title} width={96} height={144} />
          <div class="meta">
            {#each shownAuthors as a (a.id)}
              <a href="/l/{lib}/authors/{a.id}" data-link>{a.name}</a>
            {/each}
            {#if detail.authors.length > AUTHORS_SHOWN}
              <button type="button" class="link-btn" onclick={() => (allAuthors = !allAuthors)}>
                {allAuthors ? t('details.fewerAuthors') : t('authors.andMore', { count: detail.authors.length - AUTHORS_SHOWN })}
              </button>
            {/if}
            {#if detail.series}
              <span class="muted">
                <a href="/l/{lib}/series/{detail.series.id}" data-link>{detail.series.name}</a>{#if detail.serno}{` · #${detail.serno}`}{/if}
              </span>
            {/if}
            <span class="muted" title={langName(detail.lang)}>{detail.ext.toUpperCase()} · {formatSize(detail.size)} · {detail.lang}</span>
          </div>
        </div>

        <div class="actions">
          {#if iosBooks && apple}
            <div class="send-wrap">
              <div class="split">
                <button type="button" class="primary main" data-testid="open-in-books" aria-describedby="send-caption" disabled={opening} onclick={openBooks}>
                  <Icon name="read" size={16} /><span class="ellipsis">{opening ? t('books.opening') : t('books.openInBooks')}</span>
                </button>
                <button type="button" class="primary chev" bind:this={deviceBtn} aria-label={t('details.otherDevice')} title={t('details.otherDevice')} aria-haspopup="true" aria-expanded={deviceMenuOpen} onclick={() => { deviceMenuOpen = !deviceMenuOpen; downloadMenuOpen = false; }}>
                  <Icon name="chevronDown" size={14} />
                </button>
                {#if deviceMenuOpen}
                  <div class="dl-menu dev-menu" role="menu" aria-label={t('details.otherDevice')} use:dismissable={{ onClose: () => (deviceMenuOpen = false), trigger: () => deviceBtn }}>
                    {#each devicesState.items as d (d.id)}
                      <button type="button" role="menuitem" onclick={() => { deviceMenuOpen = false; onSend([detail!.id], d.id); }}>
                        <span class="verb">{t(`device.action.${deviceVerb(d)}`)}</span><span class="muted">{deviceCaption(d)}</span>
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>
              <span class="caption" id="send-caption" data-testid="send-caption">{deviceCaption(apple)}</span>
            </div>
          {:else if dev}
            <div class="send-wrap">
              <div class="split">
                <button type="button" class="primary main" data-testid="quick-send" title={deviceCaption(dev)} aria-describedby="send-caption" onclick={() => onSend([detail!.id], dev.id)}>
                  <Icon name={deviceVerb(dev) === 'send' ? 'send' : 'download'} size={16} /><span class="ellipsis">{t(`device.action.${deviceVerb(dev)}`)}</span>
                </button>
                <button type="button" class="primary chev" bind:this={deviceBtn} aria-label={t('details.otherDevice')} title={t('details.otherDevice')} aria-haspopup="true" aria-expanded={deviceMenuOpen} onclick={() => { deviceMenuOpen = !deviceMenuOpen; downloadMenuOpen = false; }}>
                  <Icon name="chevronDown" size={14} />
                </button>
                {#if deviceMenuOpen}
                  <div class="dl-menu dev-menu" role="menu" aria-label={t('details.otherDevice')} use:dismissable={{ onClose: () => (deviceMenuOpen = false), trigger: () => deviceBtn }}>
                    {#each devicesState.items as d (d.id)}
                      <button type="button" role="menuitem" onclick={() => { deviceMenuOpen = false; onSend([detail!.id], d.id); }}>
                        <span class="verb">{t(`device.action.${deviceVerb(d)}`)}</span><span class="muted">{deviceCaption(d)}</span>
                      </button>
                    {/each}
                  </div>
                {/if}
              </div>
              <span class="caption" id="send-caption" data-testid="send-caption">{deviceCaption(dev)}</span>
            </div>
          {/if}
          <div class="dl-wrap">
            <button type="button" class="secondary icon-only" bind:this={downloadBtn} aria-label={t('details.downloadAs')} title={t('details.downloadAs')} aria-haspopup="true" aria-expanded={downloadMenuOpen} onclick={() => (downloadMenuOpen = !downloadMenuOpen)}>
              <Icon name="download" size={16} /><Icon name="chevronDown" size={14} />
            </button>
            {#if downloadMenuOpen}
              <div class="dl-menu" role="menu" aria-label={t('details.downloadAs')} use:dismissable={{ onClose: () => (downloadMenuOpen = false), trigger: () => downloadBtn }}>
                {#each detail.formats as f (f)}
                  <a href={api.fileUrl(lib, detail.id, f)} data-link={false} role="menuitem" onclick={() => (downloadMenuOpen = false)}>{f === 'original' ? `${t('details.original')} (${detail.ext})` : f}</a>
                {/each}
              </div>
            {/if}
          </div>
          {#if !ios}
            <button type="button" class="secondary icon-only" data-testid="send-to-phone" aria-label={t('phone.action')} title={t('phone.action')} onclick={() => (phoneOpen = true)}>
              <Icon name="phone" size={16} />
            </button>
          {/if}
          <button type="button" class="secondary" onclick={() => navigate(`/l/${lib}/read/${detail!.id}`)}>
            <Icon name="read" size={16} />{t('details.read')}
          </button>
        </div>
        {#if phoneOpen}
          <PhoneDialog {lib} bookId={detail.id} device={apple} onClose={() => (phoneOpen = false)} />
        {/if}

        {#if detail.deleted}<p class="warn">{t('details.deletedNote')}</p>{/if}

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
            <div class="annotation">{@html detail.annotation}</div>
          </div>
        {/if}

        <dl class="meta-list">
          <dt>{t('details.added')}</dt><dd>{formatDate(detail.date, i18nState.lang)}</dd>
          {#if detail.isbn?.length || detail.isbnRaw}
            <dt>{t('details.isbn')}</dt>
            <dd class="isbn-cell" data-testid="isbn">
              {#each detail.isbn ?? [] as i (i.isbn13)}
                <span class="isbn-line">
                  <span class="isbn">{i.display}</span>
                  <button type="button" class="copy-btn" aria-label={t('details.copyIsbn', { isbn: i.isbn13 })} title={t('details.copyIsbn', { isbn: i.isbn13 })} onclick={() => copyText(i.isbn13)}>
                    <Icon name="copy" size={13} />
                  </button>
                  {#if i.isbn10}<span class="muted small isbn10" title={t('details.isbn10', { isbn: i.isbn10 })}>ISBN-10 {i.isbn10}</span>{/if}
                </span>
              {/each}
              {#if !detail.isbn?.length && detail.isbnRaw}
                <span class="isbn-line"><span class="ellipsis" title={detail.isbnRaw}>{detail.isbnRaw}</span><span class="muted small">{t('details.isbnUnverified')}</span></span>
              {/if}
            </dd>
          {/if}
          {#if detail.publisher || detail.publishYear}
            <dt>{t('details.publisher')}</dt>
            <dd data-testid="publisher">{[detail.publisher, detail.publishYear].filter(Boolean).join(', ')}</dd>
          {/if}
          <dt>{t('ratings.my')}</dt><dd data-testid="my-rating"><Rating value={detail.rating} onChange={rate} /></dd>
          <dt>{t('ratings.lib')}</dt>
          <dd data-testid="lib-rating" title={t('ratings.libSource')}>
            {#if detail.libRating}<span class="lib-stars"><Rating value={detail.libRating} /></span>{:else}<span class="muted">{t('ratings.none')}</span>{/if}
          </dd>
          <dt>{t('ratings.ext')}</dt>
          <dd data-testid="ext-rating">
            {#if detail.extRating}
              <span class="ext-line" title={t('ratings.extSource')}>
                <span class="ext-stars"><Rating value={Math.round(detail.extRating.avg)} /></span>
                <b>{formatAvg(detail.extRating.avg)}</b>
                <span class="muted">· {tn('ratings.votes', detail.extRating.votes)}</span>
                {#if detail.extRatingInfo?.url}<a href={detail.extRatingInfo.url} target="_blank" rel="noopener noreferrer" data-link={false} class="ol-link">openlibrary.org</a>{/if}
              </span>
            {:else if detail.extRatingInfo?.status === 'found'}
              <span class="muted">{t('ratings.extNoVotes')}</span>
              {#if detail.extRatingInfo.url}<a href={detail.extRatingInfo.url} target="_blank" rel="noopener noreferrer" data-link={false} class="ol-link">openlibrary.org</a>{/if}
            {:else if detail.extRatingInfo?.status === 'not_found'}
              <span class="muted">{t('ratings.extNotFound')}</span>
            {:else}
              <span class="muted">{t('ratings.extPending')}</span>
            {/if}
          </dd>
          <dt>{t('ratings.age')}</dt>
          <dd data-testid="kids-age" title={t('ratings.kidsHint')}>
            {#if detail.kidsAge !== null && detail.kidsAge !== undefined}<KidsBadge age={detail.kidsAge} /> <span class="muted small">{t('ratings.estimate')}</span>{:else}<span class="muted">{t('ratings.unknown')}</span>{/if}
          </dd>
          <dt>{t('details.shelves')}</dt>
          <dd class="shelves-cell">
            {#each shelfNames as name (name)}<span class="chip small">{name}</span>{/each}
            <button type="button" class="link-btn" onclick={() => onAddShelf([detail!.id])}>{t('details.addShelf')}</button>
          </dd>
          <dt>{t('details.file')}</dt><dd class="muted ellipsis" title={detail.file}>{detail.file}</dd>
        </dl>
      {:else if summary}
        <div class="summary">
          <h3>{t('details.aboutAuthor')}</h3>
          <h2>{summary.name}</h2>
          <p class="stats">
            {[
              tn('browse.booksCount', summary.count),
              summary.series.length ? tn('browse.seriesCount', summary.series.length) : '',
              summary.anthologies ? tn('browse.inAnthologies', summary.anthologies) : '',
            ].filter(Boolean).join(' · ')}
          </p>
          {#if years}<p class="muted">{t('details.addedYears', { years })}</p>{/if}
          <p class="hint">{t('details.pickBookHint')}</p>

          {#if summary.langs.length > 1}
            <h4>{t('details.languages')}</h4>
            <div class="chips flat">
              {#each summary.langs as [code, n] (code)}<span class="chip small" title={langName(code)}>{code} <b>{n}</b></span>{/each}
            </div>
          {/if}
          {#if summary.genres.length}
            <h4>{t('details.genres')}</h4>
            <div class="chips flat">
              {#each summary.genres as [g, n] (g)}<a href="/l/{lib}/genres/{g}" data-link class="chip small">{genreNames.get(g) ?? g} <b>{n}</b></a>{/each}
            </div>
          {/if}
          {#if summary.coauthors.some((c) => c.direct > 0)}
            <h4>{t('details.coauthors')}</h4>
            <ul class="plain">
              {#each summary.coauthors.filter((c) => c.direct > 0) as c (c.id)}
                <li><a href="/l/{lib}/authors/{c.id}" data-link>{c.name}</a><span class="n">{c.direct}</span></li>
              {/each}
            </ul>
          {/if}
          {#if summary.series.length}
            <h4>{t('details.seriesList')}</h4>
            <ul class="plain">
              {#each allSeries ? summary.series : summary.series.slice(0, SERIES_SHOWN) as s (s.id)}
                <li><a href="/l/{lib}/series/{s.id}" data-link title={s.name}>{s.name}</a><span class="n">{s.count}</span></li>
              {/each}
              {#if summary.withoutSeries}
                <li class="muted"><span>{t('books.outsideSeries')}</span><span class="n">{summary.withoutSeries}</span></li>
              {/if}
            </ul>
            {#if summary.series.length > SERIES_SHOWN}
              <button type="button" class="link-btn" onclick={() => (allSeries = !allSeries)}>
                {allSeries ? t('details.fewerSeries') : t('details.allSeries', { count: summary.series.length })}
              </button>
            {/if}
          {/if}
        </div>
      {:else}
        <div class="empty"><p>{loading ? t('common.loading') : t('details.noSelection')}</p></div>
      {/if}
    </aside>
  {/if}
{/if}

<style>
  .lib-stars :global(.rating svg.on) { fill: var(--muted); stroke: var(--muted); }
  .ext-stars :global(.rating svg.on) { fill: var(--sky); stroke: var(--sky); }
  .ext-line { display: inline-flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .ol-link { font-size: 12px; color: var(--muted); }
  .ol-link:hover { color: var(--accent); }
  .small { font-size: 12px; }
  .details {
    flex: 0 1 var(--w, 360px); min-width: 280px; position: relative; container-type: inline-size;
    border-left: 1px solid var(--line); background: var(--surface-alt); overflow-y: auto; display: flex; flex-direction: column;
  }
  .details.standalone { flex: 1 1 auto; border-left: none; }
  .rail {
    flex: 0 0 36px; border-left: 1px solid var(--line); background: var(--surface-alt);
    display: flex; flex-direction: column; align-items: center; gap: 10px; padding-top: 10px;
  }
  .rail-btn, .collapse {
    width: 28px; height: 28px; border: none; border-radius: 6px; background: transparent; color: var(--muted);
    display: flex; align-items: center; justify-content: center; flex-shrink: 0;
  }
  .rail-btn:hover, .collapse:hover { background: var(--surface-hover); color: var(--ink); }
  .rail-label { writing-mode: vertical-rl; font-size: 12px; color: var(--muted); letter-spacing: .04em; }
  .collapse { position: absolute; top: 8px; right: 8px; z-index: 2; }
  .empty { flex-grow: 1; display: flex; flex-direction: column; gap: 10px; align-items: center; justify-content: center; color: var(--muted); font-size: 14px; padding: 24px; text-align: center; }
  .empty p { margin: 0; }
  .sk-cover { width: 112px; height: 168px; border-radius: 4px; background: var(--surface-hover); }
  .sk-line { width: 70%; height: 12px; border-radius: 6px; background: var(--surface-hover); }
  .sk-line.short { width: 40%; }
  .book-title { padding: 22px 44px 0 24px; }
  .top { padding: 14px 24px 0; display: flex; gap: 16px; }
  .meta { display: flex; flex-direction: column; gap: 6px; min-width: 0; align-items: flex-start; }
  h2 { margin: 0; font-family: var(--font-display); font-size: 20px; font-weight: 600; line-height: 1.25; overflow-wrap: anywhere; }
  .muted { color: var(--muted); font-size: 13px; }
  .actions { padding: 20px 24px 0; display: flex; gap: 8px; align-items: flex-start; }
  .send-wrap { flex-grow: 1; min-width: 0; display: flex; flex-direction: column; gap: 4px; }
  .split { position: relative; display: flex; }
  .split .main { border-top-right-radius: 0 !important; border-bottom-right-radius: 0 !important; }
  .actions .split button.chev { flex: 0 0 34px; padding: 0; border-top-left-radius: 0; border-bottom-left-radius: 0; border-left: 1px solid rgba(255,255,255,.25); }
  .caption { font-size: 12px; color: var(--muted); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding-left: 2px; }
  .dev-menu { left: 0; right: auto; min-width: 240px; }
  .dev-menu button { all: unset; box-sizing: border-box; display: flex; justify-content: space-between; gap: 12px; width: 100%; padding: 8px 12px; font-size: 13px; color: var(--ink); cursor: pointer; }
  .dev-menu button:hover, .dev-menu button:focus-visible { background: var(--surface-hover); }
  .dev-menu .verb { font-weight: 500; }
  .actions button.primary {
    flex-grow: 1; min-width: 0; display: flex; align-items: center; justify-content: center; gap: 8px; height: 40px;
    border: none; border-radius: 8px; background: var(--accent); color: #fff; font-weight: 500; font-size: 14px; padding: 0 12px;
  }
  .actions button.primary:hover { background: var(--accent-hover); }
  .actions button.primary:disabled { opacity: .7; }
  .actions button.secondary { display: flex; align-items: center; gap: 6px; height: 40px; padding: 0 12px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface); color: var(--ink); font-size: 14px; flex-shrink: 0; }
  .actions button.icon-only { gap: 4px; }
  /* narrow pane: the send button gets its own row */
  @container (max-width: 400px) {
    .actions { flex-wrap: wrap; }
    .actions .send-wrap { flex-basis: 100%; }
    .actions button.secondary { flex-grow: 1; justify-content: center; }
    .actions .dl-wrap { flex-grow: 1; display: flex; }
    .actions .dl-wrap button { flex-grow: 1; justify-content: center; }
  }
  .dl-wrap { position: relative; }
  .dl-menu { position: absolute; top: 44px; right: 0; background: var(--surface); border: 1px solid var(--line); border-radius: 8px; box-shadow: 0 8px 24px rgba(0,0,0,.15); min-width: 160px; z-index: 10; overflow: hidden; }
  .dl-menu a { display: block; padding: 8px 12px; font-size: 13px; color: var(--ink); text-decoration: none; text-transform: uppercase; }
  .dl-menu a:hover { background: var(--surface-hover); }
  .warn { margin: 12px 24px 0; font-size: 13px; color: var(--danger); }
  .chips { padding: 16px 24px 0; display: flex; flex-wrap: wrap; gap: 6px; }
  .chips.flat { padding: 0; }
  .chip {
    display: inline-flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px; border-radius: 13px;
    background: var(--surface-hover); color: var(--muted-2); font-size: 12px; text-decoration: none;
  }
  .chip b { font-weight: 600; color: var(--muted); }
  .chip.small { height: 22px; }
  .section { padding: 18px 24px 0; }
  h3 { margin: 0 0 6px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; text-transform: uppercase; }
  .annotation { margin: 0; font-family: var(--font-display); font-size: 14px; line-height: 1.55; color: var(--ink); }
  .annotation :global(p) { margin: 0 0 .6em; }
  .meta-list { margin: 18px 24px 24px; padding-top: 14px; border-top: 1px solid var(--line); display: grid; grid-template-columns: 96px minmax(0,1fr); row-gap: 8px; font-size: 13px; }
  .meta-list dt { color: var(--muted); }
  .meta-list dd { margin: 0; }
  .isbn-cell { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
  .isbn-line { display: flex; align-items: center; flex-wrap: wrap; column-gap: 6px; min-width: 0; }
  .isbn10 { flex-basis: 100%; font-variant-numeric: tabular-nums; }
  .isbn { font-variant-numeric: tabular-nums; white-space: nowrap; }
  .copy-btn { display: inline-flex; align-items: center; justify-content: center; width: 24px; height: 22px; padding: 0; border: none; border-radius: 4px; background: transparent; color: var(--muted); flex-shrink: 0; }
  .copy-btn:hover { background: var(--surface-hover); color: var(--ink); }
  .copy-btn:focus-visible { outline: 2px solid var(--focus); }
  .shelves-cell { display: flex; align-items: center; gap: 6px; flex-wrap: wrap; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .link-btn { all: unset; color: var(--accent); font-size: 12px; cursor: pointer; }
  .link-btn:hover { text-decoration: underline; }
  .link-btn:focus-visible { outline: 2px solid var(--focus); }
  .summary { padding: 22px 24px 28px; display: flex; flex-direction: column; gap: 8px; }
  .summary h2 { font-size: 20px; padding-right: 24px; }
  .summary h4 { margin: 12px 0 2px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; text-transform: uppercase; }
  .summary p { margin: 0; }
  .stats { font-size: 14px; color: var(--muted-2); }
  .hint { font-size: 13px; color: var(--muted); padding: 8px 10px; border: 1px dashed var(--border); border-radius: 8px; margin-top: 4px !important; }
  .plain { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
  .plain li { display: flex; align-items: baseline; gap: 8px; font-size: 13px; padding: 4px 0; border-bottom: 1px solid var(--line-soft); min-width: 0; }
  .plain li a, .plain li span:first-child { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .plain .n { margin-left: auto; color: var(--muted); font-size: 12px; font-variant-numeric: tabular-nums; flex-shrink: 0; }
  @media (max-width: 900px) {
    .details { flex: 1 1 auto; min-width: 0; border-left: none; }
    .collapse, .rail { display: none; }
    .book-title { padding-right: 24px; }
  }
</style>
