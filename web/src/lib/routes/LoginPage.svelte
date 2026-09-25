<script lang="ts">
  import { login } from '../stores/session.svelte';
  import { navigate } from '../router.svelte';
  import { t } from '../i18n';
  import Icon from '../components/Icon.svelte';
  import { ApiError } from '../api/types';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  async function submit(e: Event) {
    e.preventDefault();
    busy = true;
    error = '';
    try {
      await login(username, password);
      navigate('/', { replace: true });
    } catch (err) {
      error = err instanceof ApiError && err.code === 'rate_limited' ? err.message : t('login.error');
    } finally {
      busy = false;
    }
  }
</script>

<div class="login-page">
  <form class="card" onsubmit={submit}>
    <div class="brand"><Icon name="library" size={28} strokeWidth={1.8} /><span>{t('app.name')}</span></div>
    <h1>{t('login.title')}</h1>
    <!-- svelte-ignore a11y_autofocus -->
    <label>{t('login.username')}<input type="text" autocomplete="username" autocapitalize="none" spellcheck="false" autofocus bind:value={username} required /></label>
    <label>{t('login.password')}<input type="password" autocomplete="current-password" bind:value={password} required /></label>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <button type="submit" disabled={busy}>{t('login.submit')}</button>
  </form>
</div>

<style>
  .login-page { height: 100%; display: flex; align-items: center; justify-content: center; background: var(--page); }
  .card { width: 320px; display: flex; flex-direction: column; gap: 14px; padding: 32px; background: var(--surface); border-radius: 12px; border: 1px solid var(--line); box-shadow: 0 12px 32px rgba(0,0,0,.08); }
  .brand { display: flex; align-items: center; gap: 10px; color: var(--accent); }
  .brand span { font-family: var(--font-display); font-size: 20px; font-weight: 600; color: var(--ink); }
  h1 { margin: 0 0 4px; font-size: 18px; }
  label { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--muted-2); }
  input { height: 38px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 14px; color: var(--ink); }
  .error { color: var(--danger); font-size: 13px; margin: 0; }
  button { height: 42px; border: none; border-radius: 8px; background: var(--accent); color: #fff; font-size: 14px; font-weight: 500; margin-top: 4px; }
  button:disabled { opacity: .6; }
</style>
