<script lang="ts">
  import { initialsBg } from '../utils/format';

  let {
    lib, bookId, title, size = 'thumb', width = 44, height = 64,
  }: { lib: number; bookId: number; title: string; size?: 'thumb' | 'full'; width?: number; height?: number } = $props();

  let loaded = $state(false);
  let errored = $state(false);
  const src = $derived(`/api/v1/libraries/${lib}/books/${bookId}/cover?size=${size}`);
</script>

<div class="cover" style="width: {width}px; height: {height}px; background: {initialsBg(title)}">
  {#if !errored}
    <img
      {src}
      alt=""
      loading="lazy"
      class:loaded
      onload={() => (loaded = true)}
      onerror={() => (errored = true)}
    />
  {/if}
  {#if !loaded}
    <span class="ph">{title}</span>
  {/if}
</div>

<style>
  .cover {
    position: relative; border-radius: 4px; overflow: hidden; flex-shrink: 0;
    display: flex; align-items: flex-end; padding: 8px; box-sizing: border-box;
  }
  img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: cover; opacity: 0; transition: opacity .15s; }
  img.loaded { opacity: 1; }
  .ph {
    position: relative; font-family: var(--font-display); font-size: 11px; line-height: 1.2;
    color: #FFFFFF; z-index: 1;
  }
</style>
