<script lang="ts">
  import { untrack } from 'svelte';
  import { api, errorText } from '../api/client';
  import type { Account, Device, Settings, UserRow } from '../api/types';
  import { routerState } from '../router.svelte';
  import { navigate } from '../router.svelte';
  import { t, i18nState, setLang } from '../i18n';
  import { themeState, setTheme } from '../stores/theme.svelte';
  import { librariesState, loadLibraries, setCurrentLibrary } from '../stores/libraries.svelte';
  import { devicesState, loadDevices, reorderDevices } from '../stores/devices.svelte';
  import { sessionState } from '../stores/session.svelte';
  import Icon from '../components/Icon.svelte';
  import Dialog from '../components/Dialog.svelte';
  import ConvertOptionsEditor from '../components/ConvertOptionsEditor.svelte';
  import ApiTokens from '../components/ApiTokens.svelte';
  import { showToast } from '../stores/toast.svelte';

  let { section }: { section: string | null } = $props();
  const sec = $derived(section ?? 'general');

  const sections = ['general', 'account', 'devices', 'mail', 'server', 'users', 'about'] as const;
  const isAdmin = $derived(sessionState.openMode || sessionState.user?.role === 'admin');

  let settings = $state<Settings | null>(null);
  let fonts = $state<string[]>([]);
  let users = $state<UserRow[]>([]);
  let editingDevice = $state<Device | null>(null);
  let testTo = $state('');
  let recipientsText = $state('');
  // The SMTP password is write-only: the server only says whether one is set.
  let smtpPassword = $state('');

  $effect(() => { loadDevices(); api.fonts().then((f) => (fonts = f)); });
  $effect(() => {
    if (isAdmin) {
      api.settings().then((s) => {
        settings = s;
        recipientsText = (s.smtp.allowedRecipients ?? []).join('\n');
      });
      api.users().then((u) => (users = u));
    }
  });

  function newDevice(): Device {
    return {
      id: 0, name: '', kind: 'download', format: 'epub', target: null, fileName: '%a - %s %n - %b', shared: false,
      options: {
        hyphenate: 'soft', footnotes: 'end', dropCaps: false, breakAfterChapter: true, tocPlacement: 'start',
        createCover: 'missing', coverLabel: '%s %n', joinSeries: false, transliterate: false, annotation: true,
        fontFamily: null, userCss: null,
      },
    };
  }

  async function saveDevice() {
    if (!editingDevice) return;
    try {
      if (editingDevice.id === 0) {
        const { id, ...rest } = editingDevice;
        const d = await api.createDevice(rest);
        devicesState.items.push(d);
      } else {
        const d = await api.updateDevice(editingDevice.id, editingDevice);
        const i = devicesState.items.findIndex((x) => x.id === d.id);
        if (i >= 0) devicesState.items[i] = d;
      }
      editingDevice = null;
    } catch (e) {
      showToast(errorText(e), 'error');
    }
  }
  async function deleteDevice(d: Device) {
    if (!confirm(t('settings.devices.deleteConfirm', { name: d.name }))) return;
    try {
      await api.deleteDevice(d.id);
      devicesState.items = devicesState.items.filter((x) => x.id !== d.id);
    } catch (e) {
      showToast(errorText(e), 'error');
    }
  }
  async function setDefaultLibrary(id: number) {
    setCurrentLibrary(id);
    if (!isAdmin) return;
    try {
      await api.updateLibrary(id, { isDefault: true });
      await loadLibraries();
      showToast(t('settings.saved'));
    } catch (e) {
      showToast(errorText(e), 'error');
    }
  }
  const kindLabel = (k: string) => t(`settings.devices.kind.${k}`);

  // ---- device order (per user; the first device is the default)
  let dragId = $state<number | null>(null);
  let dropIndex = $state<number | null>(null);
  async function moveDevice(id: number, to: number) {
    const ids = devicesState.items.map((d) => d.id);
    const from = ids.indexOf(id);
    if (from < 0 || to < 0 || to >= ids.length || from === to) return;
    ids.splice(from, 1);
    ids.splice(to, 0, id);
    try {
      await reorderDevices(ids);
    } catch (e) {
      showToast(errorText(e), 'error');
      loadDevices();
    }
  }
  function onDrop(e: DragEvent, index: number) {
    e.preventDefault();
    if (dragId !== null) {
      const from = devicesState.items.findIndex((d) => d.id === dragId);
      moveDevice(dragId, from < index ? index - 1 : index);
    }
    dragId = null; dropIndex = null;
  }

  async function saveSettings(opts: { clearPassword?: boolean; quiet?: boolean } = {}): Promise<boolean> {
    if (!settings) return false;
    settings.smtp.allowedRecipients = recipientsText
      .split(/[\n,]/)
      .map((p) => p.trim())
      .filter((p) => p !== '');
    const smtp = { ...settings.smtp };
    delete smtp.password;
    if (opts.clearPassword) smtp.password = '';
    else if (smtpPassword !== '') smtp.password = smtpPassword;
    try {
      settings = await api.updateSettings({ ...settings, smtp });
      recipientsText = settings.smtp.allowedRecipients.join('\n');
      smtpPassword = '';
      if (!opts.quiet) showToast(t('settings.saved'));
      return true;
    } catch (e) {
      showToast(errorText(e), 'error');
      return false;
    }
  }
  async function testSmtp() {
    // the test uses the stored settings, so save what is on screen first
    if (!(await saveSettings({ quiet: true }))) return;
    try {
      await api.testSmtp(testTo);
      showToast(t('settings.mail.testSent', { to: testTo }));
    } catch (e) {
      showToast(errorText(e), 'error');
    }
  }

  let newUsername = $state(''), newPassword = $state(''), newRole = $state('reader');
  async function addUser() {
    if (!newUsername.trim() || !newPassword) return;
    try {
      const u = await api.createUser({ username: newUsername.trim(), password: newPassword, role: newRole });
      users.push({ ...u, hasPassword: true, sso: null });
      newUsername = ''; newPassword = '';
    } catch (e) {
      showToast(errorText(e), 'error');
    }
  }
  // ---- Account: single sign-on link, own password (for OPDS apps)
  let account = $state<Account | null>(null);
  let accountError = $state<string | null>(null);
  let pwCurrent = $state(''), pwNew = $state(''), pwConfirm = $state('');
  let accountBusy = $state(false);
  const accountParams = $derived(new URLSearchParams(routerState.search));
  const linkError = $derived(sec === 'account' ? accountParams.get('ssoError') : null);
  function loadAccount() {
    api.account().then((a) => { account = a; accountError = null; }).catch((e) => (accountError = errorText(e)));
  }
  $effect(() => {
    if (sec !== 'account' || sessionState.openMode) return;
    const linked = accountParams.get('sso') === 'linked';
    untrack(() => {
      loadAccount();
      if (linked) {
        showToast(t('account.sso.linkedToast'));
        navigate('/settings/account', { replace: true });
      }
    });
  });
  function ssoMessage(code: string): string {
    const key = `login.sso.error.${code}`;
    const m = t(key);
    return m === key ? t('login.sso.error.generic') : m;
  }
  async function linkSso() {
    accountBusy = true;
    try {
      const { url } = await api.oidcLink();
      location.href = url;
    } catch (e) {
      showToast(errorText(e), 'error');
      accountBusy = false;
    }
  }
  async function unlinkSso() {
    if (!confirm(t('account.sso.unlinkConfirm'))) return;
    accountBusy = true;
    try {
      await api.oidcUnlink();
      loadAccount();
    } catch (e) {
      showToast(errorText(e), 'error');
    } finally {
      accountBusy = false;
    }
  }
  async function savePassword(e: Event) {
    e.preventDefault();
    if (pwNew !== pwConfirm) { showToast(t('account.password.mismatch'), 'error'); return; }
    accountBusy = true;
    try {
      await api.setPassword(pwNew, account?.hasPassword ? pwCurrent : undefined);
      pwCurrent = ''; pwNew = ''; pwConfirm = '';
      showToast(t('account.password.saved'));
      loadAccount();
    } catch (err) {
      showToast(errorText(err), 'error');
    } finally {
      accountBusy = false;
    }
  }

  async function deleteUser(u: UserRow) {
    if (!confirm(t('settings.users.deleteConfirm', { name: u.username }))) return;
    try {
      await api.deleteUser(u.id);
      users = users.filter((x) => x.id !== u.id);
    } catch (e) {
      showToast(errorText(e), 'error');
    }
  }
</script>

<main class="settings-page">
  <nav class="tabs" aria-label={t('settings.title')}>
    {#each sections as s (s)}
      {#if s !== 'users' && s !== 'mail' && s !== 'server' || isAdmin}
        <button type="button" class:active={sec === s} onclick={() => navigate(`/settings/${s}`)}>{t(`settings.${s}`)}</button>
      {/if}
    {/each}
  </nav>

  <div class="panel">
    {#if sec === 'general'}
      <h2>{t('settings.general')}</h2>
      <label class="field">{t('settings.general.theme')}
        <div class="seg">
          <button type="button" class:on={themeState.value === 'light'} onclick={() => setTheme('light')}>{t('theme.light')}</button>
          <button type="button" class:on={themeState.value === 'dark'} onclick={() => setTheme('dark')}>{t('theme.dark')}</button>
          <button type="button" class:on={themeState.value === 'system'} onclick={() => setTheme('system')}>{t('theme.system')}</button>
        </div>
      </label>
      <label class="field">{t('settings.general.language')}
        <div class="seg">
          <button type="button" class:on={i18nState.lang === 'en'} onclick={() => setLang('en')}>English</button>
          <button type="button" class:on={i18nState.lang === 'ru'} onclick={() => setLang('ru')}>Русский</button>
          <button type="button" class:on={i18nState.lang === 'uk'} onclick={() => setLang('uk')}>Українська</button>
        </div>
      </label>
      <label class="field">{t('settings.general.defaultLibrary')}
        <select onchange={(e) => setDefaultLibrary(Number((e.currentTarget as HTMLSelectElement).value))} disabled={!isAdmin && librariesState.items.length < 2}>
          {#each librariesState.items as l (l.id)}<option value={l.id} selected={l.isDefault}>{l.name}</option>{/each}
        </select>
      </label>
    {:else if sec === 'account'}
      <h2>{t('settings.account')}</h2>
      {#if sessionState.openMode}
        <p class="muted">{t('tokens.openMode')}</p>
      {:else if accountError}
        <p class="warn">{accountError}</p>
      {:else if account}
        <p class="muted" data-testid="account-user">{t('account.signedInAs', { name: account.user.username })} · {t(`settings.users.role.${account.user.role}`)}</p>
        {#if account.sso}
          <section class="acc-block" data-testid="account-sso">
            <h3><Icon name="key" size={16} />{t('account.sso.title')}</h3>
            {#if linkError}<p class="warn" role="alert">{ssoMessage(linkError)}</p>{/if}
            {#if account.sso.linked}
              <p>{t('account.sso.linked')}{#if account.sso.email}{': '}<b>{account.sso.email}</b>{/if}</p>
              <div class="row">
                <button type="button" class="btn" disabled={accountBusy || !account.hasPassword || !account.passwordLogin} onclick={unlinkSso}>{t('account.sso.unlink')}</button>
              </div>
              {#if !account.hasPassword}<p class="muted hint">{t('account.sso.unlinkNeedsPassword')}</p>
              {:else if !account.passwordLogin}<p class="muted hint">{t('account.sso.unlinkPasswordOff')}</p>{/if}
            {:else}
              <p class="muted">{t('account.sso.notLinked', { label: account.sso.label })}</p>
              <div class="row"><button type="button" class="primary" disabled={accountBusy} onclick={linkSso}><Icon name="link" size={16} />{t('account.sso.link')}</button></div>
            {/if}
          </section>
        {/if}
        <section class="acc-block" data-testid="account-password">
          <h3><Icon name="user" size={16} />{account.hasPassword ? t('account.password.title') : t('account.password.setTitle')}</h3>
          <p class="muted hint">{account.hasPassword ? t('account.password.hint') : t('account.password.ssoHint')}</p>
          <form class="pw-form" onsubmit={savePassword}>
            <input type="text" autocomplete="username" value={account.user.username} hidden readonly />
            {#if account.hasPassword}
              <label class="field">{t('account.password.current')}<input type="password" autocomplete="current-password" bind:value={pwCurrent} required /></label>
            {/if}
            <label class="field">{t('account.password.new')}<input type="password" autocomplete="new-password" minlength="4" bind:value={pwNew} required /></label>
            <label class="field">{t('account.password.confirm')}<input type="password" autocomplete="new-password" minlength="4" bind:value={pwConfirm} required /></label>
            <button type="submit" class="primary" disabled={accountBusy}>{account.hasPassword ? t('account.password.change') : t('account.password.set')}</button>
          </form>
        </section>
      {/if}
      <ApiTokens />
    {:else if sec === 'devices'}
      <h2>{t('settings.devices')}</h2>
      <p class="muted hint">{t('settings.devices.orderHint')}</p>
      <div class="device-list" role="list" aria-label={t('settings.devices')}>
        {#each devicesState.items as d, i (d.id)}
          {@const canEdit = isAdmin || (!d.shared && d.kind !== 'folder')}
          <!-- svelte-ignore a11y_no_static_element_interactions -->
          <div
            class="device-row"
            role="listitem"
            data-testid="device-row"
            class:dragging={dragId === d.id}
            class:drop-before={dropIndex === i && dragId !== null && dragId !== d.id}
            ondragover={(e) => { if (dragId !== null) { e.preventDefault(); const r = (e.currentTarget as HTMLElement).getBoundingClientRect(); dropIndex = e.clientY > r.top + r.height / 2 ? i + 1 : i; } }}
            ondrop={(e) => onDrop(e, dropIndex ?? i)}
          >
            <span
              class="handle"
              draggable="true"
              role="img"
              aria-label={t('settings.devices.dragHandle')}
              title={t('settings.devices.dragHandle')}
              ondragstart={(e) => { dragId = d.id; e.dataTransfer?.setData('text/plain', String(d.id)); if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move'; }}
              ondragend={() => { dragId = null; dropIndex = null; }}
            ><Icon name="grip" size={16} strokeWidth={3} /></span>
            <span class="order-btns">
              <button type="button" class="mini" disabled={i === 0} aria-label={t('settings.devices.moveUp', { name: d.name })} title={t('settings.devices.moveUp', { name: d.name })} onclick={() => moveDevice(d.id, i - 1)}><Icon name="chevronUp" size={14} /></button>
              <button type="button" class="mini" disabled={i === devicesState.items.length - 1} aria-label={t('settings.devices.moveDown', { name: d.name })} title={t('settings.devices.moveDown', { name: d.name })} onclick={() => moveDevice(d.id, i + 1)}><Icon name="chevronDown" size={14} /></button>
            </span>
            <div class="dinfo">
              <span class="name">{d.name}</span>
              <span class="muted">
                {kindLabel(d.kind)} · {d.format.toUpperCase()}{#if d.kind !== 'download'}{' · '}{#if d.target}<span class="target">{d.target}</span>{:else}<span class="warn">{d.kind === 'email' ? t('settings.devices.noAddress') : t('settings.devices.noFolder')}</span>{/if}{/if}
              </span>
            </div>
            {#if i === 0}<span class="badge default-badge" title={t('settings.devices.defaultHint')}>{t('settings.devices.default')}</span>{/if}
            {#if d.shared}<span class="badge" title={t('settings.devices.shared')}>{t('settings.devices.sharedShort')}</span>{/if}
            {#if canEdit}
              <button type="button" class="btn" onclick={() => (editingDevice = { ...d, options: { ...d.options } })}>{t('common.edit')}</button>
              <button type="button" class="danger" onclick={() => deleteDevice(d)}>{t('common.delete')}</button>
            {/if}
          </div>
        {/each}
      </div>
      <button type="button" class="primary" onclick={() => (editingDevice = newDevice())}><Icon name="plus" size={16} />{t('settings.devices.add')}</button>
    {:else if sec === 'mail' && settings}
      <h2>{t('settings.mail')}</h2>
      <div class="fields-grid">
        <label class="field">{t('settings.mail.host')}<input type="text" bind:value={settings.smtp.host} /></label>
        <label class="field">{t('settings.mail.port')}<input type="number" bind:value={settings.smtp.port} /></label>
        <label class="field">{t('settings.mail.security')}
          <select bind:value={settings.smtp.security}>
            <option value="none">none</option><option value="starttls">starttls</option><option value="tls">tls</option>
          </select>
        </label>
        <label class="field">{t('settings.mail.username')}<input type="text" autocomplete="off" bind:value={settings.smtp.username} /></label>
        <label class="field">{t('settings.mail.password')}
          <input
            type="password"
            autocomplete="new-password"
            placeholder={settings.smtp.passwordSet ? t('settings.mail.passwordKeep') : ''}
            bind:value={smtpPassword}
          />
          {#if settings.smtp.passwordSet}
            <span class="muted hint">{t('settings.mail.passwordSet')}
              <button type="button" class="link" onclick={() => saveSettings({ clearPassword: true })}>{t('settings.mail.passwordClear')}</button>
            </span>
          {/if}
        </label>
        <label class="field">{t('settings.mail.from')}<input type="text" bind:value={settings.smtp.from} /></label>
        <label class="field">{t('settings.mail.pause')}<input type="number" bind:value={settings.smtp.pauseSeconds} /></label>
        <label class="field">{t('settings.mail.dailyLimit')}<input type="number" min="0" bind:value={settings.smtp.dailyLimitPerUser} /></label>
        <label class="field">{t('settings.mail.subject')}
          <input type="text" placeholder="%b" bind:value={settings.smtp.subject} />
          <span class="muted hint">{t('settings.mail.subjectHint')}</span>
        </label>
      </div>
      <label class="field recipients">{t('settings.mail.allowedRecipients')}
        <textarea rows="4" spellcheck="false" placeholder="*@kindle.com" bind:value={recipientsText}></textarea>
        <span class="muted hint">{t('settings.mail.allowedRecipientsHint')}</span>
      </label>
      <div class="row">
        <button type="button" class="primary" onclick={() => saveSettings()}>{t('common.save')}</button>
      </div>
      <div class="test-row">
        <input type="text" placeholder={t('settings.mail.testTo')} bind:value={testTo} />
        <button type="button" onclick={testSmtp}>{t('settings.mail.test')}</button>
      </div>
    {:else if sec === 'server' && settings}
      <h2>{t('settings.server')}</h2>
      <label class="checkbox"><input type="checkbox" bind:checked={settings.opds.enabled} />{t('settings.server.opdsEnabled')}</label>
      <label class="checkbox"><input type="checkbox" bind:checked={settings.opds.requireAuth} />{t('settings.server.opdsAuth')}</label>
      {#if settings.mcp}
        <label class="checkbox"><input type="checkbox" bind:checked={settings.mcp.enabled} data-testid="mcp-enabled" />{t('settings.server.mcpEnabled')}</label>
        <p class="muted hint">{t('settings.server.mcpHint')}</p>
      {/if}
      {#if settings.externalRatings}
        {@const er = settings.externalRatings}
        <section class="acc-block" data-testid="ext-ratings">
          <h3>{t('settings.server.extTitle')}</h3>
          <label class="checkbox"><input type="checkbox" bind:checked={er.enabled} data-testid="ext-enabled" />{t('settings.server.extEnabled')}</label>
          <p class="muted hint">{t('settings.server.extPrivacy')}</p>
          {#if er.progress}
            {@const pct = er.progress.total ? Math.min(100, (er.progress.lookedUp / er.progress.total) * 100) : 0}
            <div class="progress" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={Math.round(pct)} aria-label={t('settings.server.extProgress')}>
              <div class="bar" style:width="{pct}%"></div>
            </div>
            <p class="small" data-testid="ext-progress">
              {t('settings.server.extCounts', { looked: er.progress.lookedUp.toLocaleString(), total: er.progress.total.toLocaleString(), found: er.progress.found.toLocaleString(), rated: er.progress.rated.toLocaleString() })}
              {#if er.queued}· {t('settings.server.extQueued', { n: er.queued })}{/if}
            </p>
            {#if er.pausedFor}<p class="warn small">{t('settings.server.extPaused', { s: er.pausedFor })}</p>{/if}
            {#if er.lastError}<p class="muted small">{t('settings.server.extLastError')}: {er.lastError}</p>{/if}
            {#if !er.contactSet}<p class="muted small">{t('settings.server.extContact')}</p>{/if}
          {/if}
        </section>
      {/if}
      <div class="row">
        <span>{t('settings.server.calibre')}:</span>
        <span>{settings.calibre.available ? `${t('settings.server.calibreAvailable')} (${settings.calibre.version})` : t('settings.server.calibreMissing')}</span>
      </div>
      <div class="row"><button type="button" class="primary" onclick={() => saveSettings()}>{t('common.save')}</button></div>
    {:else if sec === 'users'}
      <h2>{t('settings.users')}</h2>
      <div class="device-list">
        {#each users as u (u.id)}
          <div class="device-row">
            <span class="name">{u.username}</span>
            {#if u.sso}<span class="badge" title={u.sso.email ?? ''}>{t('settings.users.sso')}{u.sso.email ? ` · ${u.sso.email}` : ''}</span>{/if}
            {#if !u.hasPassword}<span class="badge muted-badge" title={t('settings.users.noPasswordHint')}>{t('settings.users.noPassword')}</span>{/if}
            <span class="grow"></span>
            <select bind:value={u.role} onchange={() => api.updateUser(u.id, { role: u.role })}>
              <option value="admin">{t('settings.users.role.admin')}</option>
              <option value="reader">{t('settings.users.role.reader')}</option>
            </select>
            <button type="button" class="danger" onclick={() => deleteUser(u)}>{t('common.delete')}</button>
          </div>
        {/each}
      </div>
      <div class="test-row">
        <input type="text" placeholder={t('login.username')} bind:value={newUsername} />
        <input type="password" placeholder={t('login.password')} bind:value={newPassword} />
        <select bind:value={newRole} aria-label={t('settings.users.role')}><option value="reader">{t('settings.users.role.reader')}</option><option value="admin">{t('settings.users.role.admin')}</option></select>
        <button type="button" onclick={addUser}>{t('settings.users.add')}</button>
      </div>
    {:else if sec === 'about'}
      <h2>{t('settings.about')}</h2>
      <p class="muted">freeLib web edition</p>
      <p class="muted">{t('settings.about.version')}: 1.0.0-dev</p>
    {/if}
  </div>
</main>

{#if editingDevice}
  <Dialog open={true} titleId="device-title" title={editingDevice.id ? t('common.edit') : t('settings.devices.add')} onClose={() => (editingDevice = null)} width={560}>
    <form class="form" onsubmit={(e) => { e.preventDefault(); saveDevice(); }}>
      <div class="fields-grid">
        <label class="field">{t('libraries.addDialog.name')}<input type="text" bind:value={editingDevice.name} required /></label>
        <label class="field">{t('settings.devices.kind')}
          <select bind:value={editingDevice.kind}>
            <option value="email">{kindLabel('email')}</option><option value="download">{kindLabel('download')}</option>
            {#if isAdmin}<option value="folder">{kindLabel('folder')}</option>{/if}
          </select>
        </label>
        <label class="field">{t('settings.devices.format')}
          <select bind:value={editingDevice.format}>
            <option value="original">{t('details.original')}</option><option value="epub">EPUB</option><option value="kepub">KEPUB (Kobo)</option>
            <option value="azw3">AZW3 (Kindle)</option><option value="mobi">MOBI</option><option value="pdf">PDF</option>
          </select>
        </label>
        {#if editingDevice.kind !== 'download'}
          <label class="field">{editingDevice.kind === 'email' ? t('send.dest.email') : t('send.dest.folder')}
            <input type="text" bind:value={editingDevice.target} placeholder={editingDevice.kind === 'email' ? 'name@kindle.com' : 'incoming'} />
          </label>
        {/if}
        <label class="field full">{t('send.fileName')}<input type="text" bind:value={editingDevice.fileName} /></label>
        {#if isAdmin}
          <label class="checkbox"><input type="checkbox" bind:checked={editingDevice.shared} />{t('settings.devices.shared')}</label>
        {/if}
      </div>
      <ConvertOptionsEditor bind:options={editingDevice.options} {fonts} />
      <div class="footer">
        <button type="button" onclick={() => (editingDevice = null)}>{t('common.cancel')}</button>
        <button type="submit" class="primary">{t('common.save')}</button>
      </div>
    </form>
  </Dialog>
{/if}

<style>
  .settings-page { flex-grow: 1; overflow-y: auto; background: var(--surface); display: flex; }
  @media (max-width: 900px) {
    .settings-page { flex-direction: column; }
    .tabs { width: auto; flex-direction: row; overflow-x: auto; border-right: none; border-bottom: 1px solid var(--line); padding: 8px 12px; }
    .tabs button { white-space: nowrap; flex-shrink: 0; }
    .panel { padding: 16px; }
    .fields-grid { grid-template-columns: 1fr; }
    .field.full { grid-column: auto; }
  }
  .tabs { width: 200px; flex-shrink: 0; display: flex; flex-direction: column; gap: 2px; padding: 20px 12px; border-right: 1px solid var(--line); }
  .tabs button { text-align: left; height: 38px; padding: 0 12px; border: none; border-radius: 8px; background: transparent; font-size: 14px; color: var(--muted-2); }
  .tabs button:hover { background: var(--surface-hover); }
  .tabs button.active { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 500; }
  .panel { flex-grow: 1; padding: 24px 32px; max-width: 640px; display: flex; flex-direction: column; gap: 16px; }
  h2 { margin: 0 0 8px; font-family: var(--font-display); font-size: 22px; }
  .field { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--muted-2); max-width: 320px; }
  .field.full { max-width: none; grid-column: span 2; }
  .field.recipients { max-width: 520px; margin-top: 12px; }
  .field.recipients textarea { padding: 8px 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 14px; color: var(--ink); resize: vertical; }
  .field .hint { font-size: 12px; }
  .link { border: 0; background: none; padding: 0 0 0 4px; color: var(--accent); cursor: pointer; font: inherit; text-decoration: underline; }
  .fields-grid { display: grid; grid-template-columns: repeat(2, minmax(0,1fr)); gap: 12px 16px; }
  select, input[type='text'], input[type='number'], input[type='password'] { height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 14px; color: var(--ink); }
  .seg { display: flex; gap: 6px; }
  .seg button { height: 34px; padding: 0 14px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font-size: 13px; }
  .seg button.on { background: var(--accent-soft); border-color: var(--accent); color: var(--accent-soft-ink); }
  .device-list { display: flex; flex-direction: column; gap: 4px; }
  .device-row { display: flex; align-items: center; gap: 10px; padding: 8px; border: 1px solid var(--line); border-radius: 8px; font-size: 14px; }
  .device-row { padding: 10px 12px; }
  .device-row.dragging { opacity: .5; }
  .device-row.drop-before { box-shadow: 0 -2px 0 var(--accent); }
  .handle { cursor: grab; color: var(--muted); display: flex; align-items: center; padding: 4px 2px; }
  .handle:active { cursor: grabbing; }
  .order-btns { display: flex; flex-direction: column; gap: 1px; }
  .mini { width: 22px; height: 16px; padding: 0; border: 1px solid var(--border); border-radius: 4px; background: var(--surface); color: var(--muted-2); display: flex; align-items: center; justify-content: center; }
  .mini:disabled { opacity: .35; }
  .mini:hover:not(:disabled) { background: var(--surface-hover); }
  .default-badge { background: var(--accent); color: #fff; }
  .dinfo { display: flex; flex-direction: column; gap: 2px; flex-grow: 1; min-width: 0; }
  .device-row .name { font-weight: 500; }
  .device-row .muted { color: var(--muted); font-size: 12px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .target { color: var(--muted-2); }
  .warn { color: var(--amber); }
  .btn { height: 30px; padding: 0 10px; border-radius: 6px; border: 1px solid var(--border); background: var(--surface); color: var(--ink); }
  .btn:hover, button.danger:hover { background: var(--surface-hover); }
  .badge { font-size: 11px; padding: 2px 6px; border-radius: 4px; background: var(--accent-soft); color: var(--accent-soft-ink); white-space: nowrap; flex-shrink: 0; }
  button.primary { align-self: flex-start; display: flex; align-items: center; gap: 8px; height: 38px; padding: 0 14px; border: none; border-radius: 8px; background: var(--accent); color: #fff; font-size: 14px; }
  button.danger { color: var(--danger); border: 1px solid var(--border); background: var(--surface); height: 30px; padding: 0 10px; border-radius: 6px; }
  .row { display: flex; align-items: center; gap: 10px; }
  .test-row { display: flex; gap: 8px; }
  .test-row input { flex-grow: 1; }
  .checkbox { display: flex; align-items: center; gap: 8px; font-size: 14px; }
  .checkbox input { width: 16px; height: 16px; accent-color: var(--accent); }
  .form { padding: 4px 24px 24px; display: flex; flex-direction: column; gap: 16px; }
  .footer { display: flex; justify-content: flex-end; gap: 8px; }
  .footer button { height: 38px; padding: 0 16px; border-radius: 7px; border: 1px solid var(--border); background: var(--surface); font-size: 14px; }
  .footer .primary { border: none; background: var(--accent); color: #fff; }
  .muted { color: var(--muted); }
  .grow { flex-grow: 1; }
  .muted-badge { background: var(--surface-hover); color: var(--muted-2); }
  .acc-block { display: flex; flex-direction: column; gap: 10px; padding: 16px; border: 1px solid var(--line); border-radius: 10px; }
  .acc-block h3 { margin: 0; display: flex; align-items: center; gap: 8px; font-size: 15px; }
  .acc-block p { margin: 0; font-size: 14px; }
  .pw-form { display: flex; flex-direction: column; gap: 10px; }
  h2 + .muted { margin: -8px 0 0; }
  .hint, .small { font-size: 12px; margin: 0; }
  .progress { height: 8px; border-radius: 4px; background: var(--surface-hover); overflow: hidden; }
  .progress .bar { height: 100%; background: var(--accent); }
</style>
