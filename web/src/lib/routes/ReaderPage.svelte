<script lang="ts">
  // In-browser paginated reader built on foliate-js (vendored under
  // src/vendor/foliate-js — see that folder's README). This whole route is
  // lazily imported (see App.svelte), and foliate-js itself is only pulled in
  // once this component mounts, so the reader's weight never touches the main
  // bundle.
  import { api } from '../api/client';
  import type { BookDetail } from '../api/types';
  import Icon from '../components/Icon.svelte';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';

  let { lib, id }: { lib: number; id: number } = $props();
  let detail = $state<BookDetail | null>(null);
  let error = $state<string | null>(null);

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
    api.book(lib, id).then((d) => (detail = d));
  });

  $effect(() => {
    if (!container || !detail) return;
    let cancelled = false;
    (async () => {
      try {
        await import('../../vendor/foliate-js/view.js');
        if (cancelled) return;
        const el = document.createElement('foliate-view') as FoliateView;
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
        await el.init({ lastLocation: saved, showTextStart: !saved });
        el.addEventListener('relocate', (e: Event) => {
          const detailEv = (e as CustomEvent).detail;
          fraction = detailEv?.fraction ?? 0;
          currentHref = detailEv?.tocItem?.href ?? null;
          const cfi = detailEv?.cfi;
          if (cfi) {
            try { localStorage.setItem(POS_KEY(), cfi); } catch { /* ignore */ }
          }
        });
        ready = true;
      } catch (err) {
        console.error(err);
        error = String(err instanceof Error ? err.message : err);
      }
    })();
    return () => {
      cancelled = true;
      view?.close?.();
      view = null;
    };
  });

  function prevPage() { view?.goLeft(); }
  function nextPage() { view?.goRight(); }

  function onKeydown(e: KeyboardEvent) {
    if (e.key === 'ArrowLeft') { e.preventDefault(); prevPage(); }
    else if (e.key === 'ArrowRight') { e.preventDefault(); nextPage(); }
    else if (e.key === 'Escape') { tocOpen = false; }
  }

  function onSliderInput(e: Event) {
    const v = parseFloat((e.target as HTMLInputElement).value);
    fraction = v;
    view?.goToFraction(v);
  }

  function goToTocItem(href: string) {
    view?.goTo(href);
    tocOpen = false;
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

<svelte:window onkeydown={onKeydown} />

<div class="reader" data-theme={theme}>
  <header>
    <button type="button" aria-label={t('common.back')} onclick={() => navigate(`/l/${lib}/book/${id}`)}>
      <Icon name="chevronLeft" size={18} />
    </button>
    <button type="button" class="toc-btn" aria-label={t('reader.toc')} onclick={() => (tocOpen = !tocOpen)}>
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
      <aside class="toc-drawer" aria-label={t('reader.toc')}>
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

<style>
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
