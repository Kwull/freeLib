<script lang="ts" generics="T">
  // Minimal fixed-row-height virtualizer — enough for the 50k-row authors/series
  // lists and long book tables without pulling in a Svelte-5-uncertain dependency.
  import type { Snippet } from 'svelte';

  let {
    items,
    itemHeight,
    overscan = 8,
    row,
    scrollToIndex = $bindable<number | null>(null),
    onRangeChange,
    class: className = '',
  }: {
    items: T[];
    itemHeight: number;
    overscan?: number;
    row: Snippet<[T, number]>;
    scrollToIndex?: number | null;
    onRangeChange?: (start: number, end: number) => void;
    class?: string;
  } = $props();

  let viewport: HTMLDivElement | undefined = $state();
  let scrollTop = $state(0);
  let viewportHeight = $state(0);

  const total = $derived(items.length * itemHeight);
  const startIndex = $derived(Math.max(0, Math.floor(scrollTop / itemHeight) - overscan));
  const endIndex = $derived(Math.min(items.length, Math.ceil((scrollTop + viewportHeight) / itemHeight) + overscan));
  const visible = $derived(items.slice(startIndex, endIndex));
  const padTop = $derived(startIndex * itemHeight);

  $effect(() => {
    if (onRangeChange) onRangeChange(startIndex, endIndex);
  });

  $effect(() => {
    if (scrollToIndex !== null && scrollToIndex !== undefined && viewport) {
      viewport.scrollTop = scrollToIndex * itemHeight;
      scrollToIndex = null;
    }
  });

  function onScroll() {
    if (viewport) scrollTop = viewport.scrollTop;
  }

  function onResize(node: HTMLDivElement) {
    const ro = new ResizeObserver(() => { viewportHeight = node.clientHeight; });
    ro.observe(node);
    return { destroy: () => ro.disconnect() };
  }
</script>

<div
  bind:this={viewport}
  onscroll={onScroll}
  use:onResize
  class="vlist {className}"
  style="overflow-y: auto; overflow-x: hidden; height: 100%; position: relative;"
>
  <div style="height: {total}px; position: relative;">
    <div style="position: absolute; top: {padTop}px; left: 0; right: 0;">
      {#each visible as item, i (startIndex + i)}
        {@render row(item, startIndex + i)}
      {/each}
    </div>
  </div>
</div>
