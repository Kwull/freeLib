<script lang="ts">
  // `/oauth/consent?request=…`: an app (Claude, Claude Code, …) asks for access to the MCP
  // endpoint. The server has already checked the app and its redirect URI; the user chooses
  // the permissions and allows or denies. `?error=…` shows why a request was refused.
  import { api, errorText } from '../api/client';
  import type { OAuthRequest, TokenScope } from '../api/types';
  import { t } from '../i18n';
  import Icon from '../components/Icon.svelte';
  import { sessionState, logout } from '../stores/session.svelte';

  let { request, error: errorCode }: { request: string | null; error: string | null } = $props();

  let req = $state<OAuthRequest | null>(null);
  let loadError = $state<string | null>(null);
  let chosen = $state<Record<string, boolean>>({});
  let busy = $state(false);
  let leaving = $state<string | null>(null);

  $effect(() => {
    if (errorCode || !request) return;
    api.oauthRequest(request)
      .then((r) => {
        req = r;
        chosen = Object.fromEntries(r.scopes.map((s) => [s, true]));
      })
      .catch((e) => (loadError = errorText(e)));
  });

  function errorMessage(code: string): string {
    const key = `oauth.error.${code}`;
    const m = t(key);
    return m === key ? t('oauth.error.generic') : m;
  }

  const selected = $derived((req?.scopes ?? []).filter((s) => chosen[s]) as TokenScope[]);

  async function decide(approve: boolean) {
    if (!req || !request) return;
    busy = true;
    try {
      const { redirect } = await api.oauthDecide(request, { approve, scopes: approve ? selected : [], csrf: req.csrf });
      leaving = approve ? req.redirectHost : null;
      location.assign(redirect);
    } catch (e) {
      loadError = errorText(e);
      busy = false;
    }
  }

  async function switchAccount() {
    await logout().catch(() => {});
    location.reload();
  }
</script>

<div class="consent-page">
  <div class="card" data-testid="oauth-consent">
    <div class="brand"><Icon name="library" size={26} strokeWidth={1.8} /><span>{t('app.name')}</span></div>
    {#if errorCode || (!request && !loadError)}
      <h1>{t('oauth.errorTitle')}</h1>
      <p class="error" role="alert" data-testid="oauth-error">{errorMessage(errorCode ?? 'invalid_request')}</p>
    {:else if loadError}
      <h1>{t('oauth.errorTitle')}</h1>
      <p class="error" role="alert" data-testid="oauth-error">{loadError}</p>
    {:else if !req}
      <p class="muted">{t('common.loading')}</p>
    {:else if leaving !== null || busy}
      <p class="muted" role="status">{leaving ? t('oauth.redirecting', { host: leaving }) : t('common.loading')}</p>
    {:else}
      <h1>{t('oauth.titleBefore')}<b data-testid="oauth-app-name">{req.client.name}</b>{t('oauth.titleAfter')}</h1>
      <div class="who">
        {#if req.client.verifiedHost}
          <p class="verified" data-testid="oauth-verified"><Icon name="check" size={15} />{t('oauth.verified', { host: req.client.verifiedHost })}</p>
        {:else}
          <p class="unverified" data-testid="oauth-unverified"><Icon name="alert" size={15} />{t('oauth.unverified')}</p>
        {/if}
        <p class="redirect">{t('oauth.redirectTo')} <b data-testid="oauth-redirect-host">{req.redirectHost}</b></p>
        {#if req.loopback}
          <p class="warn" role="note" data-testid="oauth-loopback">{t('oauth.loopback')}</p>
        {/if}
      </div>
      <fieldset class="scopes">
        <legend>{t('oauth.permissions')}</legend>
        {#each req.scopes as s (s)}
          <label class="scope">
            <input type="checkbox" bind:checked={chosen[s]} data-testid="oauth-scope-{s}" />
            <span><b>{s}</b> — {t(`tokens.scope.${s}`)}</span>
          </label>
        {/each}
      </fieldset>
      <p class="muted small">
        {#if sessionState.openMode}{t('oauth.openMode')}{:else}{t('oauth.signedInAs', { user: sessionState.user?.username ?? '' })}
          <button type="button" class="link" onclick={switchAccount}>{t('oauth.switchAccount')}</button>{/if}
      </p>
      <p class="muted small">{t('oauth.revokeHint')}</p>
      <div class="actions">
        <button type="button" class="secondary" onclick={() => decide(false)} disabled={busy} data-testid="oauth-deny">{t('oauth.deny')}</button>
        <button type="button" class="primary" onclick={() => decide(true)} disabled={busy || selected.length === 0} data-testid="oauth-allow">{t('oauth.allow')}</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .consent-page { min-height: 100%; display: flex; align-items: center; justify-content: center; background: var(--page); padding: 16px; overflow-y: auto; }
  .card { width: 100%; max-width: 440px; display: flex; flex-direction: column; gap: 14px; padding: 28px 32px; background: var(--surface); border-radius: 12px; border: 1px solid var(--line); box-shadow: 0 12px 32px rgba(0,0,0,.08); }
  .brand { display: flex; align-items: center; gap: 10px; color: var(--accent); }
  .brand span { font-family: var(--font-display); font-size: 19px; font-weight: 600; color: var(--ink); }
  h1 { margin: 0; font-size: 18px; font-weight: 500; line-height: 1.35; color: var(--ink); }
  p { margin: 0; font-size: 14px; line-height: 1.45; color: var(--ink); }
  .who { display: flex; flex-direction: column; gap: 6px; padding: 12px; border-radius: 8px; background: var(--surface-alt); border: 1px solid var(--line); }
  .verified, .unverified { display: flex; align-items: center; gap: 6px; font-size: 13px; }
  .verified { color: var(--accent); }
  .unverified { color: var(--amber); }
  .redirect { font-size: 13px; color: var(--muted-2); }
  .redirect b { color: var(--ink); font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }
  .warn { font-size: 13px; color: var(--amber); }
  .error { color: var(--danger); }
  .muted { color: var(--muted); }
  .small { font-size: 12.5px; }
  .scopes { border: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 6px; }
  .scopes legend { font-size: 13px; color: var(--muted-2); margin-bottom: 6px; padding: 0; }
  .scope { display: flex; align-items: flex-start; gap: 8px; font-size: 14px; color: var(--ink); }
  .scope input { width: 16px; height: 16px; margin-top: 2px; accent-color: var(--accent); flex-shrink: 0; }
  .actions { display: flex; gap: 10px; justify-content: flex-end; margin-top: 4px; }
  .actions button { height: 40px; padding: 0 18px; border-radius: 8px; font-size: 14px; font-weight: 500; }
  button.primary { border: none; background: var(--accent); color: #fff; }
  button.primary:hover:not(:disabled) { background: var(--accent-hover); }
  button.secondary { border: 1px solid var(--border); background: var(--surface); color: var(--ink); }
  button.secondary:hover { background: var(--surface-hover); }
  button:disabled { opacity: .55; }
  .link { border: none; background: none; padding: 0; color: var(--accent); font-size: inherit; text-decoration: underline; cursor: pointer; }
</style>
