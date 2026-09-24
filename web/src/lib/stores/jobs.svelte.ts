import { api } from '../api/client';
import { connectEvents } from '../api/events';
import type { Job } from '../api/types';
import { upsertLibrary } from './libraries.svelte';

export const jobsState = $state<{ items: Job[]; activityOpen: boolean }>({ items: [], activityOpen: false });

export const runningCount = () => jobsState.items.filter((j) => j.state === 'running' || j.state === 'queued').length;

export async function loadJobs() {
  jobsState.items = await api.jobs();
}

function upsertJob(job: Job) {
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
