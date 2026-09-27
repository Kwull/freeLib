<script lang="ts">
  // A bad or stale link (a page, book, author, series, genre or shelf that does not exist):
  // what happened and a way back. A <main>, so the phone layouts that show one pane show this
  // one. Also used outside the shell (signed out), where `lib` is null.
  import StateCard from './StateCard.svelte';
  import Icon from './Icon.svelte';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';

  let { lib, title, text, back, path, testid = 'not-found' }: {
    lib: number | null;
    title: string;
    text?: string;
    /** where "Back" goes when there is no history to go back to */
    back?: string;
    /** the address that was asked for, shown under the text */
    path?: string;
    testid?: string;
  } = $props();

  const start = $derived(lib === null ? '/' : `/l/${lib}/home`);

  function goBack() {
    if (history.length > 1) history.back();
    else navigate(back ?? start);
  }
</script>

<main class="not-found-pane">
  <StateCard icon="alert" {title} text={text ?? t('notFound.text')} {testid}>
    {#if path}<code class="path" data-testid="not-found-path">{path}</code>{/if}
    {#snippet actions()}
      <button type="button" onclick={goBack}><Icon name="chevronLeft" size={16} />{t('common.back')}</button>
      <a class="primary" href={back ?? start} data-link>{back ? t('notFound.toList') : t('notFound.toLibrary')}</a>
    {/snippet}
  </StateCard>
</main>

<style>
  .not-found-pane { flex: 1 1 0; min-width: 0; min-height: 0; display: flex; }
  .path {
    max-width: 100%; padding: 4px 10px; border-radius: 6px; background: var(--surface-alt); border: 1px solid var(--line-soft);
    font-size: 13px; color: var(--muted-2); overflow-wrap: anywhere;
  }
</style>
