<script lang="ts">
  // Settings → Account → "API tokens & MCP": personal tokens for MCP clients (Claude Desktop,
  // Claude Code, …), the MCP URL with copy-paste configs, and the audit log of tool calls.
  import { api, errorText } from '../api/client';
  import type { ApiToken, AuditRow, OAuthApp, TokenScope, TokensResponse } from '../api/types';
  import { t } from '../i18n';
  import { showToast } from '../stores/toast.svelte';
  import { formatDate } from '../utils/format';
  import { i18nState } from '../i18n';
  import Icon from './Icon.svelte';

  let data = $state<TokensResponse | null>(null);
  let audit = $state<AuditRow[]>([]);
  let apps = $state<OAuthApp[] | null>(null);
  let error = $state<string | null>(null);
  let name = $state('');
  let scopes = $state<Record<TokenScope, boolean>>({ read: true, write: false, send: false });
  let expires = $state<number>(0);
  let busy = $state(false);
  /** the secret of the token created last: shown once */
  let fresh = $state<{ token: ApiToken; secret: string } | null>(null);
  let snippet = $state<'code' | 'desktop' | 'other'>('code');

  function load() {
    api.tokens().then((d) => { data = d; error = null; }).catch((e) => (error = errorText(e)));
    api.tokenAudit().then((a) => (audit = a)).catch(() => {});
    api.oauthApps().then((a) => (apps = a)).catch(() => (apps = []));
  }
  $effect(() => { load(); });

  const url = $derived(data?.mcp.url ?? `${location.origin}/mcp`);
  const oauth = $derived(data?.mcp.oauth ?? false);
  const secretShown = $derived(fresh?.secret ?? 'fl_YOUR_TOKEN');
  const snippets = $derived({
    code: oauth && !fresh
      ? `claude mcp add --transport http freelib ${url}`
      : `claude mcp add --transport http freelib ${url} --header "Authorization: Bearer ${secretShown}"`,
    desktop: JSON.stringify({
      mcpServers: {
        freelib: {
          command: 'npx',
          args: ['-y', 'mcp-remote', url, '--header', 'Authorization:${FREELIB_AUTH}'],
          env: { FREELIB_AUTH: `Bearer ${secretShown}` },
        },
      },
    }, null, 2),
    other: JSON.stringify({
      mcpServers: { freelib: { type: 'http', url, headers: { Authorization: `Bearer ${secretShown}` } } },
    }, null, 2),
  });

  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      showToast(t('tokens.copied'));
    } catch {
      showToast(t('tokens.copyFailed'), 'error');
    }
  }

  async function create(e: Event) {
    e.preventDefault();
    const sc = (Object.keys(scopes) as TokenScope[]).filter((s) => scopes[s]);
    if (!name.trim() || !sc.length) return;
    busy = true;
    try {
      fresh = await api.createToken({ name: name.trim(), scopes: sc, expiresInDays: expires || undefined });
      name = '';
      load();
    } catch (err) {
      showToast(errorText(err), 'error');
    } finally {
      busy = false;
    }
  }

  async function revoke(tok: ApiToken) {
    if (!confirm(t('tokens.revokeConfirm', { name: tok.name }))) return;
    try {
      await api.revokeToken(tok.id);
      if (fresh?.token.id === tok.id) fresh = null;
      load();
    } catch (err) {
      showToast(errorText(err), 'error');
    }
  }

  async function revokeApp(a: OAuthApp) {
    if (!confirm(t('oauthApps.revokeConfirm', { name: a.clientName }))) return;
    try {
      await api.revokeOAuthApp(a.id);
      load();
    } catch (err) {
      showToast(errorText(err), 'error');
    }
  }

  const when = (s: string | null) => (s ? formatDate(s.slice(0, 10), i18nState.lang) : '—');
  const time = (s: string) => {
    try { return new Date(s).toLocaleString(i18nState.lang); } catch { return s; }
  };
</script>

<section class="acc-block" data-testid="api-tokens">
  <h3><Icon name="key" size={16} />{t('tokens.title')}</h3>
  <p class="muted hint">{t('tokens.intro')}</p>
  {#if error}<p class="warn">{error}</p>{/if}
  {#if data && !data.mcp.enabled}<p class="warn" role="status">{t('tokens.mcpDisabled')}</p>{/if}

  <div class="url-row">
    <span class="lbl">{t('tokens.mcpUrl')}</span>
    <code data-testid="mcp-url">{url}</code>
    <button type="button" class="btn" onclick={() => copy(url)}>{t('tokens.copy')}</button>
  </div>

  <div class="connect" data-testid="oauth-connect">
    <h4>{t('oauthApps.connectTitle')}</h4>
    {#if oauth}
      <ol>
        <li>{t('oauthApps.step1')}</li>
        <li>{t('oauthApps.step2')} <code>{url}</code></li>
        <li>{t('oauthApps.step3')}</li>
      </ol>
      <p class="muted hint">{t('oauthApps.codeHint')} <code>claude mcp add --transport http freelib {url}</code></p>
    {:else}
      <p class="muted hint" data-testid="oauth-off">{t('oauthApps.off')}</p>
    {/if}
  </div>

  <h4>{t('oauthApps.title')}</h4>
  <div class="list" data-testid="oauth-apps">
    {#if apps && apps.length === 0}<p class="muted">{t('oauthApps.none')}</p>{/if}
    {#each apps ?? [] as a (a.id)}
      <div class="tok" data-testid="oauth-app-row">
        <div class="tinfo">
          <span class="tname">{a.clientName}
            {#if a.verifiedHost}<span class="host" title={t('oauthApps.verifiedTitle')}>✓ {a.verifiedHost}</span>{:else}<span class="host unver">{t('oauthApps.unverified')}</span>{/if}
          </span>
          <span class="muted small">{t('oauthApps.redirect')} <code>{a.redirectHost}</code> · {t('oauthApps.authorized')} {when(a.createdAt)} · {t('tokens.lastUsed')} {when(a.lastUsedAt)}</span>
        </div>
        {#each a.scopes as s (s)}<span class="badge">{s}</span>{/each}
        <button type="button" class="danger" onclick={() => revokeApp(a)}>{t('tokens.revoke')}</button>
      </div>
    {/each}
  </div>

  <h4>{t('tokens.personal')}</h4>
  <p class="muted hint">{t('tokens.personalHint')}</p>
  <form class="create" onsubmit={create}>
    <label class="field">{t('tokens.name')}
      <input type="text" bind:value={name} placeholder={t('tokens.namePlaceholder')} maxlength="100" required data-testid="token-name" />
    </label>
    <fieldset class="scopes">
      <legend>{t('tokens.scopes')}</legend>
      {#each ['read', 'write', 'send'] as s (s)}
        <label class="scope">
          <input type="checkbox" bind:checked={scopes[s as TokenScope]} data-testid="scope-{s}" />
          <span><b>{s}</b> — {t(`tokens.scope.${s}`)}</span>
        </label>
      {/each}
    </fieldset>
    <label class="field">{t('tokens.expires')}
      <select bind:value={expires}>
        <option value={0}>{t('tokens.never')}</option>
        <option value={30}>{t('tokens.days', { n: 30 })}</option>
        <option value={90}>{t('tokens.days', { n: 90 })}</option>
        <option value={365}>{t('tokens.days', { n: 365 })}</option>
      </select>
    </label>
    <button type="submit" class="primary" disabled={busy || !name.trim() || !Object.values(scopes).some(Boolean)} data-testid="token-create">
      <Icon name="plus" size={16} />{t('tokens.create')}
    </button>
  </form>

  {#if fresh}
    <div class="secret" role="status" data-testid="token-secret">
      <p><b>{t('tokens.copyNow')}</b></p>
      <div class="url-row"><code class="sec">{fresh.secret}</code><button type="button" class="btn" onclick={() => copy(fresh!.secret)}>{t('tokens.copy')}</button></div>
    </div>
  {/if}

  <div class="snippets">
    <div class="seg" role="tablist" aria-label={t('tokens.connect')}>
      <button type="button" role="tab" aria-selected={snippet === 'code'} class:on={snippet === 'code'} onclick={() => (snippet = 'code')}>Claude Code</button>
      <button type="button" role="tab" aria-selected={snippet === 'desktop'} class:on={snippet === 'desktop'} onclick={() => (snippet = 'desktop')}>Claude Desktop</button>
      <button type="button" role="tab" aria-selected={snippet === 'other'} class:on={snippet === 'other'} onclick={() => (snippet = 'other')}>{t('tokens.otherClients')}</button>
    </div>
    <p class="muted hint">{t(`tokens.hint.${snippet}`)}</p>
    <div class="code-wrap">
      <pre data-testid="snippet-{snippet}">{snippets[snippet]}</pre>
      <button type="button" class="btn copy" onclick={() => copy(snippets[snippet])}>{t('tokens.copy')}</button>
    </div>
  </div>

  <h4>{t('tokens.yours')}</h4>
  {#if data && data.tokens.length === 0}<p class="muted">{t('tokens.none')}</p>{/if}
  <div class="list">
    {#each data?.tokens ?? [] as tok (tok.id)}
      <div class="tok" data-testid="token-row">
        <div class="tinfo">
          <span class="tname">{tok.name}</span>
          <span class="muted small"><code>{tok.prefix}…</code> · {t('tokens.created')} {when(tok.createdAt)} · {t('tokens.lastUsed')} {when(tok.lastUsedAt)}{#if tok.expiresAt} · {t('tokens.expiresOn')} {when(tok.expiresAt)}{/if}</span>
        </div>
        {#each tok.scopes as s (s)}<span class="badge">{s}</span>{/each}
        <button type="button" class="danger" onclick={() => revoke(tok)}>{t('tokens.revoke')}</button>
      </div>
    {/each}
  </div>

  <h4>{t('tokens.audit')}</h4>
  {#if audit.length === 0}
    <p class="muted">{t('tokens.auditEmpty')}</p>
  {:else}
    <div class="audit" data-testid="token-audit">
      {#each audit as a (a.id)}
        <div class="arow" class:denied={!a.ok}>
          <span class="muted small">{time(a.at)}</span>
          <code>{a.tool}</code>
          <span class="small">{a.tokenName ?? a.appName ?? t('tokens.revoked')}</span>
          <span class="small" class:warn={!a.ok}>{a.ok ? t('tokens.ok') : t('tokens.failed')}</span>
          <span class="muted small det" title={a.detail}>{a.detail}</span>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .acc-block { display: flex; flex-direction: column; gap: 12px; padding: 16px; border: 1px solid var(--line); border-radius: 10px; }
  .acc-block h3 { margin: 0; display: flex; align-items: center; gap: 8px; font-size: 15px; }
  h4 { margin: 8px 0 0; font-size: 13px; color: var(--muted-2); }
  p { margin: 0; font-size: 14px; }
  .hint, .small { font-size: 12px; }
  .muted { color: var(--muted); }
  .warn { color: var(--amber); }
  .url-row { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; min-width: 0; }
  .url-row .lbl { font-size: 13px; color: var(--muted-2); }
  code { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12.5px; background: var(--surface-hover); padding: 2px 6px; border-radius: 4px; overflow-wrap: anywhere; }
  .create { display: flex; flex-direction: column; gap: 10px; }
  .field { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--muted-2); max-width: 320px; }
  input[type='text'], select { height: 34px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 14px; color: var(--ink); }
  .scopes { border: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 4px; }
  .scopes legend { font-size: 13px; color: var(--muted-2); margin-bottom: 4px; padding: 0; }
  .scope { display: flex; align-items: flex-start; gap: 8px; font-size: 13px; }
  .scope input { width: 15px; height: 15px; margin-top: 2px; accent-color: var(--accent); }
  .secret { padding: 12px; border-radius: 8px; background: var(--accent-soft); color: var(--accent-soft-ink); display: flex; flex-direction: column; gap: 8px; }
  .secret code.sec { background: var(--surface); color: var(--ink); font-size: 13px; }
  .snippets { display: flex; flex-direction: column; gap: 8px; }
  .seg { display: flex; gap: 6px; flex-wrap: wrap; }
  .seg button { height: 30px; padding: 0 12px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font-size: 13px; color: var(--ink); }
  .seg button.on { background: var(--accent-soft); border-color: var(--accent); color: var(--accent-soft-ink); }
  .code-wrap { position: relative; }
  pre { margin: 0; padding: 12px 70px 12px 12px; background: var(--surface-alt); border: 1px solid var(--line); border-radius: 8px; font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 12px; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--ink); }
  .copy { position: absolute; top: 8px; right: 8px; }
  .list { display: flex; flex-direction: column; gap: 4px; }
  .tok { display: flex; align-items: center; gap: 8px; padding: 8px 10px; border: 1px solid var(--line); border-radius: 8px; }
  .tinfo { display: flex; flex-direction: column; gap: 2px; flex-grow: 1; min-width: 0; }
  .tname { font-weight: 500; font-size: 14px; }
  .connect { display: flex; flex-direction: column; gap: 6px; padding: 12px; border-radius: 8px; background: var(--surface-alt); border: 1px solid var(--line); }
  .connect h4 { margin: 0; color: var(--ink); }
  .connect ol { margin: 0; padding-left: 20px; display: flex; flex-direction: column; gap: 4px; font-size: 13.5px; color: var(--ink); }
  .host { margin-left: 6px; font-size: 11.5px; font-weight: 400; color: var(--accent); }
  .host.unver { color: var(--amber); }
  .badge { font-size: 11px; padding: 2px 6px; border-radius: 4px; background: var(--accent-soft); color: var(--accent-soft-ink); }
  .audit { display: flex; flex-direction: column; border: 1px solid var(--line); border-radius: 8px; max-height: 320px; overflow-y: auto; }
  .arow { display: grid; grid-template-columns: 150px 150px 100px 60px minmax(0, 1fr); gap: 8px; align-items: center; padding: 6px 10px; border-bottom: 1px solid var(--line-soft); }
  .arow:last-child { border-bottom: none; }
  .arow.denied { background: var(--row-hover); }
  .det { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: ui-monospace, monospace; }
  .btn { height: 30px; padding: 0 10px; border-radius: 6px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); font-size: 13px; }
  .btn:hover { background: var(--surface-hover); }
  button.primary { align-self: flex-start; display: flex; align-items: center; gap: 8px; height: 36px; padding: 0 14px; border: none; border-radius: 8px; background: var(--accent); color: #fff; font-size: 14px; }
  button.primary:disabled { opacity: .5; }
  button.danger { color: var(--danger); border: 1px solid var(--border); background: var(--surface); height: 30px; padding: 0 10px; border-radius: 6px; }
  @media (max-width: 900px) {
    .arow { grid-template-columns: 1fr 1fr; }
    .det { grid-column: span 2; }
  }
</style>
