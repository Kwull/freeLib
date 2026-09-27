<script lang="ts">
  import { login, sessionState } from '../stores/session.svelte';
  import { navigate, routerState } from '../router.svelte';
  import { t } from '../i18n';
  import Icon from '../components/Icon.svelte';
  import { api } from '../api/client';
  import { ApiError, type SsoPending } from '../api/types';
  import { showToast } from '../stores/toast.svelte';

  let username = $state('');
  let password = $state('');
  let error = $state('');
  let busy = $state(false);

  const params = $derived(new URLSearchParams(routerState.search));
  // `/login?ssoError=<code>`: a failed single sign-on (see API.md)
  const ssoError = $derived(params.get('ssoError'));
  // `/login?ssoLink=1`: a first single sign-on whose user name belongs to an existing account
  // (never linked by name): sign in with that account's password to link it
  let pending = $state<SsoPending | null>(null);
  $effect(() => {
    if (!params.get('ssoLink')) { pending = null; return; }
    api.oidcPending().then((p) => {
      pending = p;
      username = p.username;
    }).catch(() => { pending = null; });
  });
  const sso = $derived(sessionState.auth.oidc);
  const passwordOffered = $derived(sessionState.auth.password);
  // with password sign-in disabled, the administrator account can still open the form
  let showForm = $state(false);
  const formVisible = $derived(pending ? pending.canLink && (pending.passwordLogin || showForm) : passwordOffered || showForm);

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

  async function createSeparate() {
    busy = true;
    try {
      const { user } = await api.oidcPendingCreate();
      sessionState.user = user;
      navigate('/', { replace: true });
    } catch (e) {
      error = e instanceof ApiError ? e.message : t('login.error');
    } finally {
      busy = false;
    }
  }

  async function cancelPending() {
    try { await api.oidcPendingCancel(); } catch { /* gone already */ }
    pending = null;
    navigate('/login', { replace: true });
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
      const linked = await login(username, password);
      if (linked) showToast(t('account.sso.linkedToast'));
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
    {#if pending}
      <div class="notice" role="status" data-testid="sso-link-notice">
        {#if pending.canLink}
          <p>{t('login.sso.link.exists', { name: pending.username })}</p>
          <p class="muted">{t('login.sso.link.how', { label: pending.label })}</p>
        {:else}
          <p>{t('login.sso.link.taken', { name: pending.username })}</p>
        {/if}
        {#if pending.canCreate}
          <button type="button" class="secondary" onclick={createSeparate} disabled={busy} data-testid="sso-create-separate">{t('login.sso.link.create')}</button>
        {/if}
        <button type="button" class="link" onclick={cancelPending}>{t('common.cancel')}</button>
      </div>
    {/if}
    {#if sso && !pending}
      <button type="button" class="sso" class:primary={true} onclick={startSso} disabled={busy} data-testid="sso-button">
        <Icon name="key" size={18} />{sso.label}
      </button>
      {#if formVisible}<div class="or"><span>{t('login.or')}</span></div>{/if}
    {/if}
    {#if formVisible}
      <form onsubmit={submit}>
        <!-- svelte-ignore a11y_autofocus -->
        <label>{t('login.username')}<input type="text" autocomplete="username" autocapitalize="none" spellcheck="false" autofocus={!sso} bind:value={username} readonly={!!pending} required /></label>
        <!-- svelte-ignore a11y_autofocus -->
        <label>{t('login.password')}<input type="password" autocomplete="current-password" bind:value={password} autofocus={!!pending} required /></label>
        {#if error}<p class="error" role="alert">{error}</p>{/if}
        <button type="submit" class:secondary={!!sso && !pending} class:primary={!sso || !!pending} disabled={busy}>{pending ? t('login.sso.link.submit') : t('login.submit')}</button>
      </form>
    {:else if !pending || pending.canLink}
      <button type="button" class="link" onclick={() => (showForm = true)}>{t('login.adminLogin')}</button>
    {/if}
    {#if error && !formVisible}<p class="error" role="alert">{error}</p>{/if}
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
  .notice { display: flex; flex-direction: column; gap: 8px; padding: 12px; border: 1px solid var(--line); border-radius: 8px; background: var(--page); font-size: 13px; line-height: 1.45; }
  .notice p { margin: 0; }
  .notice .muted { color: var(--muted); }
  .link { height: auto; border: none; background: none; color: var(--muted); font-size: 13px; font-weight: 400; text-decoration: underline; margin: 0; }
</style>
