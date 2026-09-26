<script lang="ts">
  import { api, errorText } from '../api/client';
  import { t } from '../i18n';
  import { showToast } from '../stores/toast.svelte';
  import { followsState, loadFollows, isFollowed, setFollows } from '../stores/follows.svelte';
  import Icon from './Icon.svelte';

  /** Follow / unfollow an author or series: its new books show up on the start page. */
  let { lib, kind, id, compact = false }: { lib: number; kind: 'author' | 'series'; id: number; compact?: boolean } = $props();

  $effect(() => { loadFollows(lib); });
  const on = $derived(isFollowed(lib, kind, id));
  let busy = $state(false);

  async function toggle() {
    busy = true;
    try {
      setFollows(lib, await api.setFollow(lib, kind, id, !on));
      showToast(on ? t('follow.stopped') : t('follow.started'));
    } catch (e) {
      showToast(errorText(e), 'error');
    } finally {
      busy = false;
    }
  }
</script>

<button
  type="button"
  class="follow"
  class:on
  class:compact
  data-testid="follow-{kind}"
  aria-pressed={on}
  disabled={busy || !followsState.loaded[lib]}
  title={on ? t('follow.hintOn') : t(`follow.hint.${kind}`)}
  onclick={toggle}
>
  <Icon name={on ? 'check' : 'bell'} size={14} />
  <span>{on ? t('follow.following') : t('follow.follow')}</span>
</button>

<style>
  .follow { display: inline-flex; align-items: center; gap: 6px; height: 28px; padding: 0 12px; border-radius: 14px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 13px; white-space: nowrap; flex-shrink: 0; }
  .follow:hover:not(:disabled) { background: var(--surface-hover); }
  .follow.on { background: var(--accent-soft); color: var(--accent-soft-ink); border-color: transparent; }
  .follow:disabled { opacity: .6; }
  .follow.compact { height: 26px; padding: 0 10px; font-size: 12px; }
</style>
