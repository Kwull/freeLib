<script lang="ts">
  import Icon from './Icon.svelte';

  let { value, size = 13, onChange }: { value: number; size?: number; onChange?: (v: number) => void } = $props();
  let hover = $state<number | null>(null);
</script>

<span class="rating" role={onChange ? 'group' : undefined} aria-label={`Rating ${value} of 5`}>
  {#each [1, 2, 3, 4, 5] as i (i)}
    {#if onChange}
      <button
        type="button"
        aria-label={`Rate ${i}`}
        onmouseenter={() => (hover = i)}
        onmouseleave={() => (hover = null)}
        onclick={(e) => { e.stopPropagation(); onChange(i === value ? 0 : i); }}
      >
        <Icon name="star" {size} strokeWidth={1.6} class={(hover ?? value) >= i ? 'on' : 'off'} />
      </button>
    {:else}
      <Icon name="star" {size} strokeWidth={1.6} class={value >= i ? 'on' : 'off'} />
    {/if}
  {/each}
</span>

<style>
  .rating { display: inline-flex; gap: 1px; }
  button { all: unset; cursor: pointer; display: flex; }
  :global(.rating svg.on) { fill: var(--amber); stroke: var(--amber); }
  :global(.rating svg.off) { fill: none; stroke: var(--border-dashed); }
</style>
