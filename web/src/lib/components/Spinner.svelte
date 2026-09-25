<script lang="ts">
  // A ring spinner in the accent color; `label` is announced to screen readers.
  let { size = 28, label = '' }: { size?: number; label?: string } = $props();
</script>

<span class="spinner" style:--s="{size}px" role={label ? 'status' : undefined} aria-label={label || undefined}></span>

<style>
  .spinner {
    display: inline-block; width: var(--s); height: var(--s); flex-shrink: 0; border-radius: 50%;
    border: max(2px, calc(var(--s) / 10)) solid var(--accent-soft); border-top-color: var(--accent);
    animation: spin .8s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation-duration: 2.4s; } }
</style>
