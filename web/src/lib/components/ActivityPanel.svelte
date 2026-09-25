<script lang="ts">
  import Icon from './Icon.svelte';
  import { jobsState, toggleActivity, cancelJob, clearFinished } from '../stores/jobs.svelte';
  import { api } from '../api/client';
  import { t } from '../i18n';
  import type { Job } from '../api/types';

  function stateColor(s: Job['state']): string {
    if (s === 'done') return 'var(--accent)';
    if (s === 'failed') return 'var(--danger)';
    if (s === 'cancelled') return 'var(--muted)';
    return 'var(--amber)';
  }
</script>

{#if jobsState.activityOpen}
  <div class="scrim" onclick={() => toggleActivity(false)} role="presentation"></div>
  <aside class="panel" aria-label={t('activity.title')}>
    <div class="head">
      <h2>{t('activity.title')}</h2>
      <button type="button" class="link-btn" onclick={() => clearFinished()}>{t('activity.clearFinished')}</button>
      <button type="button" class="close" aria-label={t('common.close')} onclick={() => toggleActivity(false)}><Icon name="close" size={16} /></button>
    </div>
    <div class="list">
      {#if jobsState.items.length === 0}
        <p class="empty">{t('activity.empty')}</p>
      {/if}
      {#each jobsState.items as j (j.id)}
        <div class="job">
          <div class="row">
            <span class="dot" style="background:{stateColor(j.state)}"></span>
            <span class="title">{j.title}</span>
          </div>
          <div class="bar"><div class="fill" style="width:{Math.round(j.progress * 100)}%; background:{stateColor(j.state)}"></div></div>
          <div class="row">
            <span class="msg">{j.message || j.state}</span>
            <span class="actions">
              {#if j.state === 'running' || j.state === 'queued'}
                <button type="button" onclick={() => cancelJob(j.id)}>{t('activity.cancel')}</button>
              {/if}
              {#if j.state === 'failed'}
                <button type="button" onclick={() => cancelJob(j.id)}>{t('activity.retry')}</button>
              {/if}
              {#if j.downloadUrl}
                <a href={api.jobDownloadUrl(j.id)} data-link={false}>{t('activity.download')}</a>
              {/if}
            </span>
          </div>
        </div>
      {/each}
    </div>
  </aside>
{/if}

<style>
  .scrim { position: fixed; inset: 0; background: var(--scrim); z-index: 90; }
  .panel {
    position: fixed; top: 0; right: 0; bottom: 0; width: 360px; max-width: 92vw; background: var(--surface);
    border-left: 1px solid var(--line); z-index: 91; display: flex; flex-direction: column; box-shadow: -8px 0 24px rgba(0,0,0,.15);
  }
  .head { display: flex; align-items: center; gap: 10px; padding: 16px 16px; border-bottom: 1px solid var(--line); }
  h2 { margin: 0; font-size: 16px; flex-grow: 1; }
  .link-btn { all: unset; color: var(--accent); font-size: 12px; cursor: pointer; }
  .close { width: 30px; height: 30px; border: none; background: transparent; border-radius: 6px; display: flex; align-items: center; justify-content: center; }
  .close:hover { background: var(--surface-hover); }
  .list { overflow-y: auto; padding: 8px 16px; display: flex; flex-direction: column; gap: 14px; }
  .empty { color: var(--muted); font-size: 13px; padding: 16px 0; }
  .job { display: flex; flex-direction: column; gap: 6px; padding-bottom: 10px; border-bottom: 1px solid var(--line-soft); }
  .row { display: flex; align-items: center; gap: 8px; font-size: 13px; }
  .dot { width: 8px; height: 8px; border-radius: 4px; flex-shrink: 0; }
  .title { font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .bar { height: 4px; border-radius: 2px; background: var(--line); overflow: hidden; }
  .fill { height: 100%; transition: width .2s; }
  .msg { color: var(--muted); flex-grow: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .actions { display: flex; gap: 8px; flex-shrink: 0; }
  .actions button { all: unset; color: var(--accent); font-size: 12px; cursor: pointer; }
  .actions a { color: var(--accent); font-size: 12px; }
</style>
