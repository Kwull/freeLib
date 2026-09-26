<script lang="ts">
  import { highlightParts } from '../utils/highlight';

  /** Text with the matched words (see `highlightParts`) in <mark>. */
  let { text, words }: { text: string; words: Set<string> | null | undefined } = $props();
  const parts = $derived(highlightParts(text, words));
</script>

{#each parts as p, i (i)}{#if p.hit}<mark>{p.text}</mark>{:else}{p.text}{/if}{/each}

<style>
  mark { background: var(--mark-bg, rgba(184,116,26,.25)); color: inherit; border-radius: 2px; }
</style>
