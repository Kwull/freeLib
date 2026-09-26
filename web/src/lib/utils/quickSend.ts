import { api, errorText } from '../api/client';
import { defaultDevice } from '../stores/devices.svelte';
import { getPref } from '../stores/prefs.svelte';
import { watchJob } from '../stores/jobs.svelte';
import { showToast } from '../stores/toast.svelte';
import { t } from '../i18n';

/**
 * One-click send of books to the user's default device (the first in their order). Returns
 * false when that needs a choice first (no device, or an e-mail device without an address):
 * the caller then opens the Send dialog.
 */
export async function quickSend(lib: number, ids: number[]): Promise<boolean> {
  const dev = defaultDevice();
  if (!dev) return false;
  const target = dev.kind === 'email'
    ? (getPref<Record<string, string>>('sendTargets', {})[String(dev.id)] ?? dev.target ?? '')
    : undefined;
  if (dev.kind === 'email' && !target) return false;
  try {
    const job = await api.send({ library: lib, books: ids, device: dev.id, target: target || undefined });
    watchJob(job);
    showToast(t('send.started', { device: dev.name }));
  } catch (e) {
    showToast(errorText(e), 'error');
  }
  return true;
}
