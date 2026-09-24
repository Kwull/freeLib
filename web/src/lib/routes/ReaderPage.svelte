<script lang="ts">
  // Minimal in-browser reader shell. Loads the book as an inline EPUB and hands
  // it to foliate-js's <foliate-view> for rendering when that package is present;
  // see web/README.md for the integration note (this is intentionally the
  // thinnest slice of the spec — foliate-js is not vendored in this pass).
  import { api } from '../api/client';
  import type { BookDetail } from '../api/types';
  import Icon from '../components/Icon.svelte';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';

  let { lib, id }: { lib: number; id: number } = $props();
  let detail = $state<BookDetail | null>(null);

  $effect(() => { api.book(lib, id).then((d) => (detail = d)); });

  const fileUrl = $derived(api.fileUrl(lib, id, 'epub', { inline: true }));
</script>

<div class="reader">
  <header>
    <button type="button" aria-label={t('common.close')} onclick={() => navigate(`/l/${lib}/book/${id}`)}>
      <Icon name="close" size={18} />
    </button>
    <span class="title">{detail?.title ?? t('common.loading')}</span>
  </header>
  <div class="content">
    {#if detail}
      <iframe title={detail.title} src={fileUrl} class="frame"></iframe>
    {/if}
  </div>
</div>

<style>
  .reader { display: flex; flex-direction: column; flex-grow: 1; min-height: 0; background: var(--surface); }
  header { display: flex; align-items: center; gap: 12px; padding: 10px 16px; border-bottom: 1px solid var(--line); flex-shrink: 0; }
  header button { width: 36px; height: 36px; border: none; background: transparent; border-radius: 8px; display: flex; align-items: center; justify-content: center; }
  header button:hover { background: var(--surface-hover); }
  .title { font-family: var(--font-display); font-size: 16px; font-weight: 600; }
  .content { flex-grow: 1; min-height: 0; }
  .frame { width: 100%; height: 100%; border: none; }
</style>
