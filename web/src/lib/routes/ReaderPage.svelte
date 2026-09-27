<script lang="ts">
  // In-browser paginated reader built on foliate-js (vendored under
  // src/vendor/foliate-js — see that folder's README). This whole route is
  // lazily imported (see App.svelte), and foliate-js itself is only pulled in
  // once this component mounts, so the reader's weight never touches the main
  // bundle.
  import { dismissable } from '../utils/dismiss';
  import { api, errorText } from '../api/client';
  import { ApiError, type BookDetail } from '../api/types';
  import Icon from '../components/Icon.svelte';
  import StateCard from '../components/StateCard.svelte';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';

  let { lib, id }: { lib: number; id: number } = $props();
  let detail = $state<BookDetail | null>(null);
  let error = $state<string | null>(null);
  /** the book does not exist (a bad or stale link) */
  let notFound = $state(false);

  type TocItem = { label: string; href: string; subitems?: TocItem[] };
  type FoliateView = HTMLElement & {
    book: { toc?: TocItem[]; dir?: string; metadata?: { title?: string } };
    renderer: {
      setStyles?: (css: string) => void;
      next: () => Promise<void>;
      prev: () => Promise<void>;
      setAttribute: (n: string, v: string) => void;
    };
    open: (url: string) => Promise<void>;
    init: (opts: { lastLocation?: string; showTextStart?: boolean }) => Promise<void>;
    goTo: (target: string) => Promise<void>;
    goLeft: () => Promise<void>;
    goRight: () => Promise<void>;
    goToFraction: (f: number) => Promise<void>;
    close: () => void;
  };

  let container: HTMLDivElement | undefined = $state();
  let view: FoliateView | null = null;
  let ready = $state(false);
  let tocOpen = $state(false);
  let tocBtn = $state<HTMLButtonElement | undefined>();
  let toc = $state<TocItem[]>([]);
  let fraction = $state(0);
  let currentHref = $state<string | null>(null);

  type Theme = 'light' | 'sepia' | 'dark';
  const THEMES: Record<Theme, { bg: string; fg: string }> = {
    light: { bg: '#FFFFFF', fg: '#1D1C19' },
    sepia: { bg: '#F3ECDB', fg: '#3A2F1F' },
    dark: { bg: '#16181A', fg: '#DDDDDD' },
  };

  const POS_KEY = () => `freelib.reader.pos.${lib}.${id}`;
  const PREFS_KEY = 'freelib.reader.prefs';

  function loadPrefs(): { fontSize: number; theme: Theme } {
    try {
      const raw = localStorage.getItem(PREFS_KEY);
      if (raw) return { fontSize: 100, theme: 'light', ...JSON.parse(raw) };
    } catch { /* ignore */ }
    return { fontSize: 100, theme: 'light' };
  }
  const prefs0 = loadPrefs();
  let fontSize = $state(prefs0.fontSize);
  let theme = $state<Theme>(prefs0.theme);

  function savePrefs() {
    try { localStorage.setItem(PREFS_KEY, JSON.stringify({ fontSize, theme })); } catch { /* ignore */ }
  }

  function css(): string {
    const th = THEMES[theme];
    return `
      html { color-scheme: ${theme === 'dark' ? 'dark' : 'light'}; }
      body { background: ${th.bg} !important; color: ${th.fg} !important; font-size: ${fontSize}% !important; }
      p, li, blockquote, dd { line-height: 1.6; }
    `;
  }

  function applyStyles() {
    view?.renderer.setStyles?.(css());
  }

  $effect(() => { fontSize; theme; applyStyles(); savePrefs(); });

  $effect(() => {
    const [l, b] = [lib, id];
    let cancelled = false;
    detail = null; error = null; notFound = false; ready = false;
    api.book(l, b)
      .then((d) => { if (!cancelled) detail = d; })
      .catch((e) => {
        if (cancelled) return;
        if (e instanceof ApiError && e.status === 404) notFound = true;
        else error = errorText(e);
      });
    return () => { cancelled = true; };
  });

  // Reading position: the CFI of the page start, from foliate's `relocate` events. While the
  // layout changes (the table of contents opens or closes, the window resizes) foliate
  // re-paginates and may land a page early; the position from before the change is held and
  // restored once the layout settles.
  let cfi: string | null = null;
  /** the page shown now (tests read it from the DOM) */
  let shownCfi = $state<string | null>(null);
  let holdTimer: ReturnType<typeof setTimeout> | undefined;
  let holding = false;
  function holdPosition() {
    if (!view || !ready || !cfi) return;
    holding = true;
    clearTimeout(holdTimer);
    holdTimer = setTimeout(restorePosition, 250);
  }
  async function restorePosition() {
    const v = view, at = cfi;
    if (!v || !at) { holding = false; return; }
    try { await v.goTo(at); } catch { /* the book is gone */ }
    // the goTo's own relocate arrives before this resolves; later ones are the reader's again
    holding = false;
  }

  $effect(() => {
    if (!container || !detail) return;
    let cancelled = false;
    let el: FoliateView | null = null;
    const onRelocate = (e: Event) => {
      const d = (e as CustomEvent).detail;
      fraction = d?.fraction ?? 0;
      currentHref = d?.tocItem?.href ?? null;
      if (d?.cfi) shownCfi = d.cfi;
      if (holding || !d?.cfi) return;
      cfi = d.cfi;
      try { localStorage.setItem(POS_KEY(), d.cfi); } catch { /* ignore */ }
    };
    (async () => {
      try {
        await import('../../vendor/foliate-js/view.js');
        if (cancelled) return;
        el = document.createElement('foliate-view') as FoliateView;
        container!.appendChild(el);
        view = el;
        const fileUrl = api.fileUrl(lib, id, 'epub', { inline: true });
        await el.open(fileUrl);
        if (cancelled) return;
        applyStyles();
        el.renderer.setAttribute('flow', 'paginated');
        toc = el.book.toc ?? [];
        let saved: string | undefined;
        try { saved = localStorage.getItem(POS_KEY()) ?? undefined; } catch { /* ignore */ }
        cfi = saved ?? null;
        el.addEventListener('relocate', onRelocate);
        await el.init({ lastLocation: saved, showTextStart: !saved });
        if (cancelled) return;
        ready = true;
      } catch (err) {
        if (cancelled) return;
        console.error(err);
        error = errorText(err);
      }
    })();
    return () => {
      // leaving the reader (or another book): stop listening, stop pending restores, tear down
      // foliate (its resize observers and late callbacks, see the vendored paginator patch)
      cancelled = true;
      clearTimeout(holdTimer);
      holding = false;
      ready = false;
      if (el) {
        el.removeEventListener('relocate', onRelocate);
        try { el.close?.(); } catch { /* half-opened */ }
        el.remove();
      }
      view = null;
    };
  });

  // the table of contents changes the text column's width
  function setToc(open: boolean) {
    if (open === tocOpen) return;
    holdPosition();
    tocOpen = open;
  }

  function prevPage() { view?.goLeft(); }
  function nextPage() { view?.goRight(); }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowLeft') { e.preventDefault(); prevPage(); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); nextPage(); }
    else if (e.key === 'Escape') { setToc(false); }
  }

  function onSliderInput(e: Event) {
    const v = parseFloat((e.target as HTMLInputElement).value);
    fraction = v;
    view?.goToFraction(v);
  }

  function goToTocItem(href: string) {
    tocOpen = false;
    // the new place wins over a held one
    clearTimeout(holdTimer);
    holding = false;
    view?.goTo(href);
  }

  // Swipe on touch: a horizontal drag past a small threshold turns a page.
  let touchStartX = 0;
  function onTouchStart(e: TouchEvent) { touchStartX = e.touches[0]?.clientX ?? 0; }
  function onTouchEnd(e: TouchEvent) {
    const dx = (e.changedTouches[0]?.clientX ?? touchStartX) - touchStartX;
    if (Math.abs(dx) < 40) return;
    if (dx > 0) prevPage(); else nextPage();
  }

  function renderTocItems(items: TocItem[], depth = 0): { item: TocItem; depth: number }[] {
    return items.flatMap((it) => [{ item: it, depth }, ...(it.subitems ? renderTocItems(it.subitems, depth + 1) : [])]);
  }
  const flatToc = $derived(renderTocItems(toc));
</script>

<svelte:window onkeydown={onKeydown} onresize={holdPosition} />

{#if notFound || (error && !detail)}
  <div class="reader-state">
    <StateCard tone={notFound ? 'info' : 'error'} icon={notFound ? 'read' : 'alert'} testid="reader-not-found"
      title={notFound ? t('notFound.book') : t('common.error')} text={notFound ? t('notFound.bookText') : undefined} detail={notFound ? null : error}>
      {#snippet actions()}
        <button type="button" onclick={() => (history.length > 1 ? history.back() : navigate(`/l/${lib}/home`))}><Icon name="chevronLeft" size={16} />{t('common.back')}</button>
        <a class="primary" href="/l/{lib}/home" data-link>{t('notFound.toLibrary')}</a>
      {/snippet}
    </StateCard>
  </div>
{:else}

<div class="reader" data-theme={theme} data-cfi={shownCfi} data-ready={ready}>
  <header>
    <button type="button" aria-label={t('common.back')} onclick={() => navigate(`/l/${lib}/book/${id}`)}>
      <Icon name="chevronLeft" size={18} />
    </button>
    <button type="button" class="toc-btn" bind:this={tocBtn} aria-label={t('reader.toc')} aria-expanded={tocOpen} onclick={() => setToc(!tocOpen)}>
      <Icon name="genres" size={18} />
    </button>
    <span class="title">{detail?.title ?? t('common.loading')}</span>
    <div class="font-ctl" role="group" aria-label={t('reader.fontSize')}>
      <button type="button" aria-label="A-" onclick={() => (fontSize = Math.max(60, fontSize - 10))}>A-</button>
      <span class="pct">{fontSize}%</span>
      <button type="button" aria-label="A+" onclick={() => (fontSize = Math.min(220, fontSize + 10))}>A+</button>
    </div>
    <div class="theme-ctl" role="group" aria-label={t('reader.theme')}>
      {#each ['light', 'sepia', 'dark'] as th (th)}
        <button type="button" class:on={theme === th} aria-pressed={theme === th} onclick={() => (theme = th as Theme)}>
          {t(`reader.theme.${th}`)}
        </button>
      {/each}
    </div>
  </header>

  <div class="body">
    {#if tocOpen}
      <aside class="toc-drawer" aria-label={t('reader.toc')} use:dismissable={{ onClose: () => setToc(false), trigger: () => tocBtn }}>
        <nav>
          {#each flatToc as { item, depth } (item.href + item.label)}
            <button
              type="button"
              class="toc-item"
              class:active={currentHref === item.href}
              style="padding-left: {12 + depth * 14}px"
              onclick={() => goToTocItem(item.href)}
            >{item.label}</button>
          {/each}
        </nav>
      </aside>
    {/if}

    <div class="content-wrap">
      {#if error}
        <div class="msg error">{t('common.error')}: {error}</div>
      {:else if !ready}
        <div class="msg">{t('common.loading')}</div>
      {/if}
      <div
        bind:this={container}
        class="viewer"
        role="presentation"
        ontouchstart={onTouchStart}
        ontouchend={onTouchEnd}
      ></div>
      {#if ready}
        <button type="button" class="edge left" aria-label={t('reader.prev')} onclick={prevPage}>
          <Icon name="chevronLeft" size={22} />
        </button>
        <button type="button" class="edge right" aria-label={t('reader.next')} onclick={nextPage}>
          <Icon name="chevronRight" size={22} />
        </button>
      {/if}
    </div>
  </div>

  <footer>
    <input
      type="range"
      min="0"
      max="1"
      step="0.001"
      value={fraction}
      oninput={onSliderInput}
      aria-label={t('reader.progress')}
    />
    <span class="pct">{Math.round(fraction * 100)}%</span>
  </footer>
</div>
{/if}

<style>
  .reader-state { display: flex; flex-grow: 1; min-height: 0; }
  .reader { display: flex; flex-direction: column; flex-grow: 1; min-height: 0; background: var(--surface); }
  header { display: flex; align-items: center; gap: 8px; padding: 8px 12px; border-bottom: 1px solid var(--line); flex-shrink: 0; }
  header button { height: 36px; border: none; background: transparent; border-radius: 8px; display: flex; align-items: center; justify-content: center; color: var(--ink); font-size: 13px; padding: 0 8px; }
  header button:hover { background: var(--surface-hover); }
  .title { font-family: var(--font-display); font-size: 15px; font-weight: 600; flex-grow: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .font-ctl, .theme-ctl { display: flex; align-items: center; gap: 2px; border: 1px solid var(--border); border-radius: 6px; padding: 2px; }
  .font-ctl button, .theme-ctl button { height: 28px; padding: 0 8px; border-radius: 4px; }
  .theme-ctl button.on { background: var(--accent-soft); color: var(--accent-soft-ink); }
  .pct { font-size: 12px; color: var(--muted); padding: 0 4px; min-width: 36px; text-align: center; }
  .body { flex-grow: 1; min-height: 0; display: flex; position: relative; }
  .toc-drawer { width: 280px; flex-shrink: 0; border-right: 1px solid var(--line); background: var(--surface-alt); overflow-y: auto; }
  .toc-drawer nav { display: flex; flex-direction: column; padding: 8px; }
  .toc-item { all: unset; cursor: pointer; padding: 8px 12px; border-radius: 6px; font-size: 13px; color: var(--ink); }
  .toc-item:hover { background: var(--surface-hover); }
  .toc-item.active { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 500; }
  .content-wrap { flex-grow: 1; min-width: 0; position: relative; }
  .viewer { width: 100%; height: 100%; }
  .msg { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; color: var(--muted); font-size: 14px; }
  .msg.error { color: #B3413B; }
  .edge { position: absolute; top: 0; bottom: 0; width: 48px; border: none; background: transparent; color: var(--muted); display: flex; align-items: center; justify-content: center; opacity: 0; transition: opacity .15s; }
  .content-wrap:hover .edge { opacity: 1; }
  .edge:hover { background: rgba(0,0,0,.04); }
  .edge.left { left: 0; }
  .edge.right { right: 0; }
  footer { display: flex; align-items: center; gap: 10px; padding: 8px 16px; border-top: 1px solid var(--line); flex-shrink: 0; }
  footer input[type='range'] { flex-grow: 1; }
  @media (max-width: 900px) {
    .font-ctl, .theme-ctl { display: none; }
    .toc-drawer { position: absolute; inset: 0; z-index: 4; width: 100%; }
  }
</style>
