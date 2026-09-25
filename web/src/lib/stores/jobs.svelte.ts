import { api } from '../api/client';
import { connectEvents } from '../api/events';
import type { Job } from '../api/types';
import { upsertLibrary } from './libraries.svelte';
import { showToast } from './toast.svelte';
import { t } from '../i18n';

export const jobsState = $state<{ items: Job[]; activityOpen: boolean }>({ items: [], activityOpen: false });

export const runningCount = () => jobsState.items.filter((j) => j.state === 'running' || j.state === 'queued').length;

export async function loadJobs() {
  jobsState.items = await api.jobs();
}

/** Jobs started from this tab: their outcome is announced (and downloads start by themselves). */
const watched = new Set<string>();

export function watchJob(job: Job) {
  watched.add(job.id);
  upsertJob(job);
}

function finished(job: Job) {
  if (!watched.has(job.id)) return;
  if (job.state === 'done') {
    watched.delete(job.id);
    if (job.downloadUrl && job.kind === 'download') {
      const a = document.createElement('a');
      a.href = job.downloadUrl;
      a.download = '';
      document.body.appendChild(a);
      a.click();
      a.remove();
    }
    showToast(t(`jobs.done.${job.kind}`, { title: job.title }));
  } else if (job.state === 'failed') {
    watched.delete(job.id);
    showToast(`${job.title}: ${job.message || t('common.error')}`, 'error');
  } else if (job.state === 'cancelled') {
    watched.delete(job.id);
  }
}

function upsertJob(job: Job) {
  finished(job);
  const i = jobsState.items.findIndex((j) => j.id === job.id);
  if (i >= 0) jobsState.items[i] = job;
  else jobsState.items.unshift(job);
}

let unsubscribe: (() => void) | null = null;

export function startJobEvents() {
  if (unsubscribe) return;
  unsubscribe = connectEvents({
    onJob: upsertJob,
    onLibrary: upsertLibrary,
  });
}

export function stopJobEvents() {
  unsubscribe?.();
  unsubscribe = null;
}

export async function cancelJob(id: string) {
  const job = await api.cancelJob(id);
  upsertJob(job);
}

export async function clearFinished() {
  await api.clearFinishedJobs();
  jobsState.items = jobsState.items.filter((j) => j.state === 'running' || j.state === 'queued');
}

export function toggleActivity(open?: boolean) {
  jobsState.activityOpen = open ?? !jobsState.activityOpen;
}
