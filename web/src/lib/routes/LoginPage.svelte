<script lang="ts">
  import { login, sessionState } from '../stores/session.svelte';
  import { navigate, routerState } from '../router.svelte';
  import { t } from '../i18n';
  import Icon from '../components/Icon.svelte';
  import { api } from '../api/client';
  import { ApiError } from '../api/types';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  const params = $derived(new URLSearchParams(routerState.search));
  // `/login?ssoError=<code>`: a failed single sign-on (see API.md)
  const ssoError = $derived(params.get('ssoError'));
  const sso = $derived(sessionState.auth.oidc);
  const passwordOffered = $derived(sessionState.auth.password);
  // with password sign-in disabled, the administrator account can still open the form
  let showForm = $state(false);
  const formVisible = $derived(passwordOffered || showForm);

  /** Where to go after signing in: the page that asked for the login, else home. */
  function returnPath(): string {
    const p = location.pathname + location.search;
    return p.startsWith('/login') ? '/' : p;
  }

  function ssoMessage(code: string): string {
    const key = `login.sso.error.${code}`;
    const m = t(key);
    return m === key ? t('login.sso.error.generic') : m;
  }

  function startSso() {
    busy = true;
    location.href = api.oidcLoginUrl(returnPath());
  }

  async function submit(e: Event) {
    e.preventDefault();
    busy = true;
    error = '';
    try {
      await login(username, password);
      navigate(returnPath(), { replace: true });
    } catch (err) {
      error = err instanceof ApiError && (err.code === 'rate_limited' || err.code === 'forbidden') ? err.message : t('login.error');
    } finally {
      busy = false;
    }
  }
</script>

<div class="login-page">
  <div class="card">
    <div class="brand"><Icon name="library" size={28} strokeWidth={1.8} /><span>{t('app.name')}</span></div>
    <h1>{t('login.title')}</h1>
    {#if ssoError}<p class="error" role="alert" data-testid="sso-error">{ssoMessage(ssoError)}</p>{/if}
    {#if sso}
      <button type="button" class="sso" class:primary={true} onclick={startSso} disabled={busy} data-testid="sso-button">
        <Icon name="key" size={18} />{sso.label}
      </button>
      {#if formVisible}<div class="or"><span>{t('login.or')}</span></div>{/if}
    {/if}
    {#if formVisible}
      <form onsubmit={submit}>
        <!-- svelte-ignore a11y_autofocus -->
        <label>{t('login.username')}<input type="text" autocomplete="username" autocapitalize="none" spellcheck="false" autofocus={!sso} bind:value={username} required /></label>
        <label>{t('login.password')}<input type="password" autocomplete="current-password" bind:value={password} required /></label>
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <button type="submit" class:secondary={!!sso} class:primary={!sso} disabled={busy}>{t('login.submit')}</button>
      </form>
    {:else}
      <button type="button" class="link" onclick={() => (showForm = true)}>{t('login.adminLogin')}</button>
    {/if}
  </div>
</div>

<style>
  .login-page { height: 100%; display: flex; align-items: center; justify-content: center; background: var(--page); padding: 16px; }
  .card { width: 100%; max-width: 340px; display: flex; flex-direction: column; gap: 14px; padding: 32px; background: var(--surface); border-radius: 12px; border: 1px solid var(--line); box-shadow: 0 12px 32px rgba(0,0,0,.08); }
  form { display: flex; flex-direction: column; gap: 14px; }
  .brand { display: flex; align-items: center; gap: 10px; color: var(--accent); }
  .brand span { font-family: var(--font-display); font-size: 20px; font-weight: 600; color: var(--ink); }
  h1 { margin: 0 0 4px; font-size: 18px; }
  label { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--muted-2); }
  input { height: 38px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 14px; color: var(--ink); }
  .error { color: var(--danger); font-size: 13px; margin: 0; line-height: 1.4; }
  button { height: 42px; border-radius: 8px; font-size: 14px; font-weight: 500; margin-top: 4px; display: flex; align-items: center; justify-content: center; gap: 8px; }
  button.primary { border: none; background: var(--accent); color: #fff; }
  button.primary:hover { background: var(--accent-hover); }
  button.secondary { border: 1px solid var(--border); background: var(--surface); color: var(--ink); }
  button.secondary:hover { background: var(--surface-hover); }
  button:disabled { opacity: .6; }
  .or { display: flex; align-items: center; gap: 10px; color: var(--muted); font-size: 12px; }
  .or::before, .or::after { content: ''; flex: 1; height: 1px; background: var(--line); }
  .link { height: auto; border: none; background: none; color: var(--muted); font-size: 13px; font-weight: 400; text-decoration: underline; margin: 0; }
</style>
