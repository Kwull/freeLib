<script lang="ts">
  import Icon from './Icon.svelte';
  import { jobsState, toggleActivity, cancelJob, retryJob, clearFinished } from '../stores/jobs.svelte';
  import { api, errorText } from '../api/client';
  import { t } from '../i18n';
  import { dismissable } from '../utils/dismiss';
  import { showToast } from '../stores/toast.svelte';
  import { formatSize } from '../utils/format';
  import type { Job, JobItem } from '../api/types';

  // jobs whose book list is open (running jobs and small jobs open by default)
  let expanded = $state<Record<string, boolean>>({});

  function stateColor(s: Job['state']): string {
    if (s === 'done') return 'var(--accent)';
    if (s === 'failed') return 'var(--danger)';
    if (s === 'cancelled') return 'var(--muted)';
    return 'var(--amber)';
  }
  function itemColor(s: JobItem['state']): string {
    if (s === 'accepted' || s === 'saved' || s === 'ready') return 'var(--accent)';
    if (s === 'failed') return 'var(--danger)';
    if (s === 'retrying') return 'var(--amber)';
    if (s === 'queued') return 'var(--muted-2)';
    return 'var(--sky, var(--amber))';
  }
  function isOpen(j: Job): boolean {
    return expanded[j.id] ?? ((j.items?.length ?? 0) > 0 && (j.items!.length <= 5 || j.state === 'running' || j.state === 'failed'));
  }
  /** "Accepted by mail server · e-mail 1 · 292 KB" */
  function itemLine(it: JobItem): string {
    const parts = [t(`activity.item.${it.state}`)];
    if (it.mail && (it.state === 'accepted' || it.state === 'sending' || it.state === 'retrying')) parts.push(t('activity.mailNo', { n: it.mail }));
    if (it.size && it.state !== 'queued') parts.push(formatSize(it.size));
    return parts.join(' · ');
  }
  async function retry(id: string) {
    try { await retryJob(id); } catch (e) { showToast(errorText(e), 'error'); }
  }
</script>

{#if jobsState.activityOpen}
  <div class="scrim" onclick={() => toggleActivity(false)} role="presentation"></div>
  <aside class="panel" aria-label={t('activity.title')}
    use:dismissable={{ onClose: () => toggleActivity(false), trigger: () => document.querySelector<HTMLElement>('.activity-btn') }}>
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
        <div class="job" data-testid="job" data-state={j.state}>
          <div class="row">
            <span class="dot" style="background:{stateColor(j.state)}"></span>
            <span class="title" title={j.title}>{j.title}</span>
          </div>
          <div class="bar"><div class="fill" style="width:{Math.round(j.progress * 100)}%; background:{stateColor(j.state)}"></div></div>
          <div class="row">
            <span class="msg" title={j.message}>{j.message || j.state}</span>
            <span class="actions">
              {#if j.state === 'running' || j.state === 'queued'}
                <button type="button" onclick={() => cancelJob(j.id)}>{t('activity.cancel')}</button>
              {/if}
              {#if j.retryable}
                <button type="button" data-testid="job-retry" onclick={() => retry(j.id)}>{t('activity.retry')}</button>
              {/if}
              {#if j.downloadUrl}
                <a href={api.jobDownloadUrl(j.id)} data-link={false}>{t('activity.download')}</a>
              {/if}
            </span>
          </div>
          {#if j.hint?.code === 'kindle_approved_sender'}
            <p class="hint" data-testid="kindle-hint">
              <Icon name="alert" size={14} />
              <span>{t('activity.kindleHint', { from: j.hint.from })}
                <a href={j.hint.url} target="_blank" rel="noopener noreferrer" data-link={false}>{t('activity.kindleHintLink')}</a></span>
            </p>
          {/if}
          {#if j.items?.length}
            <button type="button" class="toggle" aria-expanded={isOpen(j)} onclick={() => (expanded[j.id] = !isOpen(j))}>
              <Icon name={isOpen(j) ? 'chevronUp' : 'chevronDown'} size={14} />{t('activity.showBooks', { count: j.items.length })}
            </button>
            {#if isOpen(j)}
              <ul class="items" data-testid="job-items">
                {#each j.items as it, i (i)}
                  <li data-state={it.state}>
                    <span class="idot" style="background:{itemColor(it.state)}"></span>
                    <span class="ititle" title={it.title}>{it.title}</span>
                    <span class="istate" data-testid="item-state">{itemLine(it)}</span>
                    {#if it.detail}<span class="idetail" title={it.detail}>{it.detail}</span>{/if}
                  </li>
                {/each}
              </ul>
            {/if}
          {/if}
        </div>
      {/each}
    </div>
  </aside>
{/if}

<style>
  .scrim { position: fixed; inset: 0; background: var(--scrim); z-index: 90; }
  .panel {
    position: fixed; top: 0; right: 0; bottom: 0; width: 400px; max-width: 94vw; background: var(--surface);
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
  .hint { margin: 2px 0 0; display: flex; gap: 6px; align-items: flex-start; font-size: 12px; line-height: 1.4; color: var(--muted-2); background: var(--page); padding: 8px 10px; border-radius: 8px; }
  .hint :global(svg) { flex-shrink: 0; margin-top: 2px; color: var(--amber); }
  .hint a { color: var(--accent); white-space: nowrap; }
  .toggle { all: unset; display: inline-flex; align-items: center; gap: 4px; font-size: 12px; color: var(--muted); cursor: pointer; align-self: flex-start; }
  .toggle:hover { color: var(--ink); }
  .items { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .items li { display: grid; grid-template-columns: 8px minmax(0, 1fr); column-gap: 8px; font-size: 12px; }
  .idot { width: 6px; height: 6px; border-radius: 3px; margin-top: 5px; }
  .ititle { color: var(--ink); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .istate { grid-column: 2; color: var(--muted-2); }
  .idetail { grid-column: 2; color: var(--muted); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 11px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  li[data-state='failed'] .istate { color: var(--danger); }
</style>
