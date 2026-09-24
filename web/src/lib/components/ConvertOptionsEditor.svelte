<script lang="ts">
  import type { ConvertOptions } from '../api/types';
  import { t } from '../i18n';

  let { options = $bindable(), fonts }: { options: ConvertOptions; fonts: string[] } = $props();
</script>

<div class="grid">
  <label>{t('convertOptions.hyphenate')}
    <select bind:value={options.hyphenate}>
      <option value="none">{t('convertOptions.hyphenate.none')}</option>
      <option value="soft">{t('convertOptions.hyphenate.soft')}</option>
      <option value="full">{t('convertOptions.hyphenate.full')}</option>
    </select>
  </label>
  <label>{t('convertOptions.footnotes')}
    <select bind:value={options.footnotes}>
      <option value="end">{t('convertOptions.footnotes.end')}</option>
      <option value="inline">{t('convertOptions.footnotes.inline')}</option>
      <option value="popup">{t('convertOptions.footnotes.popup')}</option>
    </select>
  </label>
  <label>{t('convertOptions.tocPlacement')}
    <select bind:value={options.tocPlacement}>
      <option value="start">{t('convertOptions.tocPlacement.start')}</option>
      <option value="end">{t('convertOptions.tocPlacement.end')}</option>
      <option value="none">{t('convertOptions.tocPlacement.none')}</option>
    </select>
  </label>
  <label>{t('convertOptions.createCover')}
    <select bind:value={options.createCover}>
      <option value="never">{t('convertOptions.createCover.never')}</option>
      <option value="missing">{t('convertOptions.createCover.missing')}</option>
      <option value="always">{t('convertOptions.createCover.always')}</option>
    </select>
  </label>
  <label>{t('convertOptions.coverLabel')}
    <input type="text" bind:value={options.coverLabel} placeholder="%s %n" />
  </label>
  <label>{t('convertOptions.fontFamily')}
    <select bind:value={options.fontFamily}>
      <option value={null}>—</option>
      {#each fonts as f (f)}<option value={f}>{f}</option>{/each}
    </select>
  </label>
  <label class="checkbox"><input type="checkbox" bind:checked={options.dropCaps} />{t('convertOptions.dropCaps')}</label>
  <label class="checkbox"><input type="checkbox" bind:checked={options.breakAfterChapter} />{t('convertOptions.breakAfterChapter')}</label>
  <label class="checkbox"><input type="checkbox" bind:checked={options.joinSeries} />{t('convertOptions.joinSeries')}</label>
  <label class="checkbox"><input type="checkbox" bind:checked={options.transliterate} />{t('convertOptions.transliterate')}</label>
  <label class="checkbox"><input type="checkbox" bind:checked={options.annotation} />{t('convertOptions.annotation')}</label>
  <label class="full">{t('convertOptions.userCss')}
    <textarea bind:value={options.userCss} rows="3"></textarea>
  </label>
</div>

<style>
  .grid { display: grid; grid-template-columns: repeat(2, minmax(0,1fr)); gap: 12px 16px; }
  label { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--muted-2); }
  label.full { grid-column: span 2; }
  label.checkbox { flex-direction: row; align-items: center; gap: 8px; font-size: 14px; }
  label.checkbox input { width: 16px; height: 16px; accent-color: var(--accent); }
  input[type='text'], select, textarea { height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 14px; color: var(--ink); }
  textarea { height: auto; padding: 8px 10px; resize: vertical; font-family: monospace; }
</style>
