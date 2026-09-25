<script lang="ts">
  import QRCode from 'qrcode';

  let { text, size = 200 }: { text: string; size?: number } = $props();
  let svg = $state('');

  $effect(() => {
    QRCode.toString(text, { type: 'svg', margin: 1, width: size, color: { dark: '#1D1C19', light: '#FFFFFF00' } })
      .then((s) => (svg = s))
      .catch(() => (svg = ''));
  });
</script>

<div class="qr" style="width:{size}px; height:{size}px" role="img" aria-label={`QR code for ${text}`}>
  {@html svg}
</div>

<style>
  .qr { display: flex; align-items: center; justify-content: center; background: #fff; border-radius: 8px; padding: 8px; box-sizing: border-box; }
  .qr :global(svg) { width: 100%; height: 100%; }
</style>
