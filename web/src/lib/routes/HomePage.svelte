<script lang="ts">
  // Start page: what to read next — the next books of started series and new books by the
  // authors the user reads or follows. New arrivals, authors, series and genres stay one
  // click away.
  import { api, errorText } from '../api/client';
  import type { Book, HomeResponse } from '../api/types';
  import { navigate } from '../router.svelte';
  import { t, tn, i18nState } from '../i18n';
  import { getPref, setPref } from '../stores/prefs.svelte';
  import { formatDate } from '../utils/format';
  import { quickSend } from '../utils/quickSend';
  import { defaultDevice, deviceVerb } from '../stores/devices.svelte';
  import { showToast } from '../stores/toast.svelte';
  import CoverThumb from '../components/CoverThumb.svelte';
  import Icon from '../components/Icon.svelte';
  import SendDialog from '../components/SendDialog.svelte';
  import StateCard from '../components/StateCard.svelte';

  let { lib }: { lib: number } = $props();

  const WINDOWS = [null, 7, 30, 90] as const;
  type Win = (typeof WINDOWS)[number];
  const win = $derived(getPref<Win>('home.newWindow', null));

  let home = $state<HomeResponse | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let reload = $state(0);
  let send = $state<number[] | null>(null);
  let hidden = $state<Set<number>>(new Set());

  $effect(() => {
    reload;
    const w = win;
    let cancelled = false;
    loading = true;
    error = null;
    api.home(lib, w).then((h) => { if (!cancelled) { home = h; hidden = new Set(); } })
      .catch((e) => { if (!cancelled) error = errorText(e); })
      .finally(() => { if (!cancelled) loading = false; });
    return () => { cancelled = true; };
  });

  const dev = $derived(defaultDevice());
  const sendLabel = $derived(dev ? t(`device.action.${deviceVerb(dev)}`) : t('selection.sendTo'));

  function open(b: Book) {
    if (window.innerWidth < 900) { navigate(`/l/${lib}/book/${b.id}`); return; }
    if (b.series) navigate(`/l/${lib}/series/${b.series.id}?book=${b.id}`);
    else if (b.authors[0]) navigate(`/l/${lib}/authors/${b.authors[0].id}?book=${b.id}`);
    else navigate(`/l/${lib}/book/${b.id}`);
  }
  async function sendNow(ids: number[]) {
    if (!(await quickSend(lib, ids))) send = ids;
  }
  async function dismiss(seriesId: number, name: string) {
    hidden = new Set([...hidden, seriesId]);
    try {
      await api.dismissSeries(lib, seriesId, true);
      showToast(t('home.dismissed', { name }));
    } catch (e) {
      hidden = new Set([...hidden].filter((x) => x !== seriesId));
      showToast(errorText(e), 'error');
    }
  }
  const authorsLine = (b: Book) =>
    b.authors.length > 2 ? `${b.authors[0].name} ${t('books.andMore', { count: b.authors.length - 1 })}` : b.authors.map((a) => a.name).join(', ');
  const series = $derived((home?.continueSeries ?? []).filter((s) => !hidden.has(s.series.id)));
  const winLabel = (w: Win) => (w === null ? t('home.sinceVisit') : tn('home.lastDays', w));
</script>

{#snippet bookCard(b: Book, extra: string, reason: string | null)}
  <article class="card" data-testid="home-book">
    <button type="button" class="cover-btn" onclick={() => open(b)} aria-label={b.title}>
      <CoverThumb {lib} bookId={b.id} title={b.title} width={96} height={138} />
    </button>
    <div class="card-body">
      {#if extra}<span class="kicker">{extra}</span>{/if}
      <button type="button" class="title" onclick={() => open(b)} title={b.title}>{b.title}</button>
      <span class="muted ellipsis">{authorsLine(b)}</span>
      {#if reason}<span class="reason">{reason}</span>{/if}
      {#if b.editions}<span class="muted small">{tn('editions.count', b.editions.count)}</span>{/if}
      <span class="grow"></span>
      <button type="button" class="send" data-testid="home-send" onclick={() => sendNow([b.id])}>
        <Icon name={dev && deviceVerb(dev) !== 'send' ? 'download' : 'send'} size={14} /><span>{sendLabel}</span>
      </button>
    </div>
  </article>
{/snippet}

<div class="home" aria-busy={loading}>
  <header class="head">
    <h1>{t('home.title')}</h1>
    <nav class="quick" aria-label={t('home.browse')}>
      <a href="/l/{lib}/new" data-link><Icon name="newArrivals" size={15} />{t('nav.newArrivals')}</a>
      <a href="/l/{lib}/authors" data-link><Icon name="authors" size={15} />{t('nav.authors')}</a>
      <a href="/l/{lib}/series" data-link><Icon name="series" size={15} />{t('nav.series')}</a>
      <a href="/l/{lib}/genres" data-link><Icon name="genres" size={15} />{t('nav.genres')}</a>
    </nav>
  </header>

  {#if error}
    <StateCard tone="error" icon="alert" title={t('common.error')} text={error} testid="home-error">
      {#snippet actions()}<button type="button" class="primary" onclick={() => reload++}>{t('common.retry')}</button>{/snippet}
    </StateCard>
  {:else if !home}
    <div class="skeleton" aria-label={t('common.loading')}>
      {#each Array(4) as _, i (i)}<div class="sk-card"></div>{/each}
    </div>
  {:else}
    {#if home.empty}
      <section class="welcome" data-testid="home-empty">
        <Icon name="read" size={28} />
        <div>
          <h2 class="plain">{t('home.emptyTitle')}</h2>
          <p>{t('home.emptyText')}</p>
        </div>
      </section>
    {/if}

    {#if series.length}
      <section aria-labelledby="h-continue" data-testid="home-continue">
        <h2 id="h-continue">{t('home.continue')}</h2>
        <div class="cards">
          {#each series as s (s.series.id)}
            {@const b = s.next[0]}
            <div class="series-block" data-testid="continue-series">
              <div class="series-head">
                <a href="/l/{lib}/series/{s.series.id}" data-link class="sname" title={s.series.name}>{s.series.name}</a>
                <span class="muted small">{t('home.progress', { done: s.done, works: s.works })}</span>
                <button type="button" class="dismiss" data-testid="dismiss-series" aria-label={t('home.dismiss', { name: s.series.name })} title={t('home.notInterested')} onclick={() => dismiss(s.series.id, s.series.name)}>
                  <Icon name="close" size={14} />
                </button>
              </div>
              {@render bookCard(b, b.serno ? t('home.nextNumber', { n: b.serno }) : t('home.next'), null)}
              {#if s.next[1]}
                <button type="button" class="then" onclick={() => open(s.next[1])}>
                  {t('home.then')} {s.next[1].serno ? `#${s.next[1].serno} ` : ''}«{s.next[1].title}»
                </button>
              {/if}
            </div>
          {/each}
        </div>
      </section>
    {/if}

    <section aria-labelledby="h-new" data-testid="home-new">
      <div class="sec-head">
        <h2 id="h-new">{t('home.newFrom')}</h2>
        <div class="wins" role="group" aria-label={t('home.window')}>
          {#each WINDOWS as w (String(w))}
            <button type="button" class:on={win === w} aria-pressed={win === w} onclick={() => setPref('home.newWindow', w)}>{winLabel(w)}</button>
          {/each}
        </div>
      </div>
      {#if home.newFromAuthors.books.length}
        <p class="muted small">{t('home.newSince', { date: formatDate(home.newFromAuthors.since, i18nState.lang) })}{#if home.following.authors + home.following.series} · {t('home.followingCount', { authors: home.following.authors, series: home.following.series })}{/if}</p>
        <div class="cards">
          {#each home.newFromAuthors.books as b (b.id)}
            {@const r = b.reason}
            {@render bookCard(b, formatDate(b.date, i18nState.lang), r.id ? (r.followed ? t(`home.reason.follow.${r.kind}`, { name: r.name }) : t('home.reason.read', { name: r.name })) : null)}
          {/each}
        </div>
        {#if home.newFromAuthors.total > home.newFromAuthors.books.length}
          <p class="muted small">{t('home.more', { count: home.newFromAuthors.total - home.newFromAuthors.books.length })}</p>
        {/if}
      {:else}
        <p class="muted">{home.empty ? t('home.newEmptyNew') : t('home.newEmpty')}</p>
      {/if}
    </section>

    {#if home.picks.length}
      <section aria-labelledby="h-picks" data-testid="home-picks">
        <div class="sec-head">
          <h2 id="h-picks">{t('home.picks')}</h2>
          <a href="/l/{lib}/new" data-link class="all">{t('home.allNew')}<Icon name="chevronRight" size={14} /></a>
        </div>
        <div class="cards">
          {#each home.picks as b (b.id)}
            {@render bookCard(b, b.libRating ? `★ ${b.libRating}/5` : formatDate(b.date, i18nState.lang), null)}
          {/each}
        </div>
      </section>
    {/if}
  {/if}
</div>

{#if send}<SendDialog {lib} bookIds={send} open={true} onClose={() => (send = null)} />{/if}

<style>
  .home { flex-grow: 1; min-width: 0; overflow-y: auto; padding: 22px 28px 40px; display: flex; flex-direction: column; gap: 26px; }
  .head { display: flex; align-items: center; gap: 16px; flex-wrap: wrap; }
  h1 { margin: 0; font-family: var(--font-display); font-size: 26px; font-weight: 600; }
  .quick { display: flex; gap: 8px; flex-wrap: wrap; margin-left: auto; }
  .quick a { display: inline-flex; align-items: center; gap: 6px; height: 32px; padding: 0 12px; border-radius: 16px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 13px; text-decoration: none; }
  .quick a:hover { border-color: var(--accent); color: var(--accent); }
  h2 { margin: 0 0 12px; font-size: 12px; font-weight: 600; color: var(--muted); letter-spacing: .04em; text-transform: uppercase; }
  h2.plain { text-transform: none; letter-spacing: 0; font-size: 18px; color: var(--ink); font-family: var(--font-display); margin: 0 0 4px; }
  .sec-head { display: flex; align-items: baseline; gap: 12px; flex-wrap: wrap; margin-bottom: 4px; }
  .sec-head h2 { margin-bottom: 8px; }
  .wins { display: inline-flex; border: 1px solid var(--border); border-radius: 7px; overflow: hidden; }
  .wins button { border: none; background: var(--surface); color: var(--muted-2); font-size: 12px; height: 26px; padding: 0 10px; }
  .wins button + button { border-left: 1px solid var(--border); }
  .wins button.on { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 600; }
  .all { margin-left: auto; display: inline-flex; align-items: center; gap: 4px; font-size: 13px; }
  .welcome { display: flex; gap: 14px; align-items: flex-start; padding: 16px 18px; border-radius: 12px; background: var(--accent-soft); color: var(--accent-soft-ink); }
  .welcome p { margin: 0; max-width: 720px; color: var(--ink); font-size: 14px; line-height: 1.5; }
  .cards { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 14px; }
  .series-block { display: flex; flex-direction: column; gap: 6px; min-width: 0; }
  .series-head { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .sname { font-weight: 600; color: var(--ink); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; min-width: 0; }
  .dismiss { margin-left: auto; flex-shrink: 0; display: inline-flex; align-items: center; justify-content: center; width: 26px; height: 26px; border-radius: 6px; border: none; background: transparent; color: var(--muted); }
  .dismiss:hover { background: var(--surface-hover); color: var(--ink); }
  .card { display: flex; gap: 12px; padding: 10px; border-radius: 10px; background: var(--surface); border: 1px solid var(--line); min-width: 0; }
  .cover-btn { all: unset; cursor: pointer; flex-shrink: 0; border-radius: 4px; overflow: hidden; }
  .cover-btn:focus-visible { outline: 2px solid var(--focus); }
  .card-body { display: flex; flex-direction: column; gap: 3px; min-width: 0; flex-grow: 1; }
  .kicker { font-size: 12px; color: var(--accent); font-weight: 600; }
  .title { all: unset; cursor: pointer; font-family: var(--font-display); font-size: 16px; font-weight: 600; color: var(--ink); display: -webkit-box; -webkit-line-clamp: 2; line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
  .title:hover { color: var(--accent); }
  .title:focus-visible { outline: 2px solid var(--focus); }
  .reason { font-size: 12px; color: var(--muted-2); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .grow { flex-grow: 1; }
  .send { align-self: flex-start; display: inline-flex; align-items: center; gap: 6px; height: 30px; padding: 0 12px; border-radius: 7px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 13px; }
  .send:hover { background: var(--surface-hover); }
  .then { all: unset; cursor: pointer; font-size: 12px; color: var(--muted); padding-left: 4px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .then:hover { color: var(--accent); }
  .muted { color: var(--muted); font-size: 14px; margin: 0; }
  .small { font-size: 12px; }
  .ellipsis { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-size: 13px; }
  .skeleton { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 14px; }
  .sk-card { height: 160px; border-radius: 10px; background: var(--surface-alt); }
  @media (max-width: 900px) {
    .home { padding: 14px 12px 24px; gap: 20px; }
    h1 { font-size: 22px; }
    .quick { margin-left: 0; }
    .cards, .skeleton { grid-template-columns: 1fr; }
  }
</style>
