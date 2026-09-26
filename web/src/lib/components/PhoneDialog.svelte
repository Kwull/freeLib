<script lang="ts">
  import Dialog from './Dialog.svelte';
  import HandoffPanel from './HandoffPanel.svelte';
  import type { Device } from '../api/types';
  import { t } from '../i18n';

  let { lib, bookId, device = null, onClose }: {
    lib: number; bookId: number; device?: Device | null; onClose: () => void;
  } = $props();
</script>

<Dialog open={true} titleId="phone-title" title={t('phone.title')} {onClose} width={560}>
  <div class="body">
    <HandoffPanel {lib} {bookId} {device} />
  </div>
  <div class="footer">
    <button type="button" class="secondary" onclick={onClose}>{t('common.close')}</button>
  </div>
</Dialog>

<style>
  .body { padding: 16px 24px 0; }
  .footer { display: flex; justify-content: flex-end; padding: 16px 24px; }
  .secondary { height: 38px; padding: 0 16px; border-radius: 8px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 14px; }
</style>
