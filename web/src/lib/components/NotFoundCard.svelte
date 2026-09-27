<script lang="ts">
  // A bad or stale link (a book, author, series, genre or shelf that does not exist): what
  // happened and a way back. A <main>, so the phone layouts that show one pane show this one.
  import StateCard from './StateCard.svelte';
  import Icon from './Icon.svelte';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';

  let { lib, title, text, back, testid = 'not-found' }: {
    lib: number;
    title: string;
    text?: string;
    /** where "Back" goes when there is no history to go back to */
    back?: string;
    testid?: string;
  } = $props();

  function goBack() {
    if (history.length > 1) history.back();
    else navigate(back ?? `/l/${lib}/home`);
  }
</script>

<main class="not-found-pane">
  <StateCard icon="alert" {title} text={text ?? t('notFound.text')} {testid}>
    {#snippet actions()}
      <button type="button" onclick={goBack}><Icon name="chevronLeft" size={16} />{t('common.back')}</button>
      <a class="primary" href={back ?? `/l/${lib}/home`} data-link>{back ? t('notFound.toList') : t('notFound.toLibrary')}</a>
    {/snippet}
  </StateCard>
</main>

<style>
  .not-found-pane { flex: 1 1 0; min-width: 0; min-height: 0; display: flex; }
</style>
