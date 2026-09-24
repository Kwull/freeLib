<script lang="ts">
  import { api } from '../api/client';
  import type { Device, Settings, User } from '../api/types';
  import { navigate } from '../router.svelte';
  import { t, i18nState, setLang } from '../i18n';
  import { themeState, setTheme } from '../stores/theme.svelte';
  import { librariesState } from '../stores/libraries.svelte';
  import { devicesState, loadDevices } from '../stores/devices.svelte';
  import { sessionState } from '../stores/session.svelte';
  import Icon from '../components/Icon.svelte';
  import Dialog from '../components/Dialog.svelte';
  import ConvertOptionsEditor from '../components/ConvertOptionsEditor.svelte';
  import { showToast } from '../stores/toast.svelte';

  let { section }: { section: string | null } = $props();
  const sec = $derived(section ?? 'general');

  const sections = ['general', 'devices', 'mail', 'server', 'users', 'about'] as const;
  const isAdmin = $derived(sessionState.openMode || sessionState.user?.role === 'admin');

  let settings = $state<Settings | null>(null);
  let fonts = $state<string[]>([]);
  let users = $state<User[]>([]);
  let editingDevice = $state<Device | null>(null);
  let testTo = $state('');

  $effect(() => { loadDevices(); api.fonts().then((f) => (fonts = f)); });
  $effect(() => { if (isAdmin) { api.settings().then((s) => (settings = s)); api.users().then((u) => (users = u)); } });

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
  }
  async function deleteDevice(id: number) {
    await api.deleteDevice(id);
    devicesState.items = devicesState.items.filter((d) => d.id !== id);
  }

  async function saveSettings() {
    if (!settings) return;
    settings = await api.updateSettings(settings);
    showToast(t('common.save'));
  }
  async function testSmtp() {
    try { await api.testSmtp(testTo); showToast('OK'); } catch (e) { showToast(String(e), 'error'); }
  }

  let newUsername = $state(''), newPassword = $state(''), newRole = $state('reader');
  async function addUser() {
    const u = await api.createUser({ username: newUsername, password: newPassword, role: newRole });
    users.push(u);
    newUsername = ''; newPassword = '';
  }
  async function deleteUser(id: number) {
    await api.deleteUser(id);
    users = users.filter((u) => u.id !== id);
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
        </div>
      </label>
      <label class="field">{t('settings.general.defaultLibrary')}
        <select>
          {#each librariesState.items as l (l.id)}<option value={l.id} selected={l.isDefault}>{l.name}</option>{/each}
        </select>
      </label>
    {:else if sec === 'devices'}
      <h2>{t('settings.devices')}</h2>
      <div class="device-list">
        {#each devicesState.items as d (d.id)}
          <div class="device-row">
            <span class="name">{d.name}</span>
            <span class="muted">{d.kind} · {d.format}</span>
            {#if d.shared}<span class="badge">{t('settings.devices.shared')}</span>{/if}
            <button type="button" onclick={() => (editingDevice = { ...d, options: { ...d.options } })}>{t('common.edit')}</button>
            <button type="button" class="danger" onclick={() => deleteDevice(d.id)}>{t('common.delete')}</button>
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
        <label class="field">{t('settings.mail.username')}<input type="text" bind:value={settings.smtp.username} /></label>
        <label class="field">{t('settings.mail.from')}<input type="text" bind:value={settings.smtp.from} /></label>
        <label class="field">{t('settings.mail.pause')}<input type="number" bind:value={settings.smtp.pauseSeconds} /></label>
      </div>
      <div class="row">
        <button type="button" class="primary" onclick={saveSettings}>{t('common.save')}</button>
      </div>
      <div class="test-row">
        <input type="text" placeholder={t('settings.mail.testTo')} bind:value={testTo} />
        <button type="button" onclick={testSmtp}>{t('settings.mail.test')}</button>
      </div>
    {:else if sec === 'server' && settings}
      <h2>{t('settings.server')}</h2>
      <label class="checkbox"><input type="checkbox" bind:checked={settings.opds.enabled} />{t('settings.server.opdsEnabled')}</label>
      <label class="checkbox"><input type="checkbox" bind:checked={settings.opds.requireAuth} />{t('settings.server.opdsAuth')}</label>
      <div class="row">
        <span>{t('settings.server.calibre')}:</span>
        <span>{settings.calibre.available ? `${t('settings.server.calibreAvailable')} (${settings.calibre.version})` : t('settings.server.calibreMissing')}</span>
      </div>
      <div class="row"><button type="button" class="primary" onclick={saveSettings}>{t('common.save')}</button></div>
    {:else if sec === 'users'}
      <h2>{t('settings.users')}</h2>
      <div class="device-list">
        {#each users as u (u.id)}
          <div class="device-row">
            <span class="name">{u.username}</span>
            <select bind:value={u.role} onchange={() => api.updateUser(u.id, { role: u.role })}>
              <option value="admin">{t('settings.users.role.admin')}</option>
              <option value="reader">{t('settings.users.role.reader')}</option>
            </select>
            <button type="button" class="danger" onclick={() => deleteUser(u.id)}>{t('common.delete')}</button>
          </div>
        {/each}
      </div>
      <div class="test-row">
        <input type="text" placeholder={t('login.username')} bind:value={newUsername} />
        <input type="password" placeholder={t('login.password')} bind:value={newPassword} />
        <select bind:value={newRole}><option value="reader">reader</option><option value="admin">admin</option></select>
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
        <label class="field">Kind
          <select bind:value={editingDevice.kind}>
            <option value="email">email</option><option value="download">download</option><option value="folder">folder</option>
          </select>
        </label>
        <label class="field">Format
          <select bind:value={editingDevice.format}>
            <option value="original">original</option><option value="epub">epub</option><option value="kepub">kepub</option>
            <option value="azw3">azw3</option><option value="mobi">mobi</option><option value="pdf">pdf</option>
          </select>
        </label>
        <label class="field">Target<input type="text" bind:value={editingDevice.target} /></label>
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
  .tabs { width: 200px; flex-shrink: 0; display: flex; flex-direction: column; gap: 2px; padding: 20px 12px; border-right: 1px solid var(--line); }
  .tabs button { text-align: left; height: 38px; padding: 0 12px; border: none; border-radius: 8px; background: transparent; font-size: 14px; color: var(--muted-2); }
  .tabs button:hover { background: var(--surface-hover); }
  .tabs button.active { background: var(--accent-soft); color: var(--accent-soft-ink); font-weight: 500; }
  .panel { flex-grow: 1; padding: 24px 32px; max-width: 640px; display: flex; flex-direction: column; gap: 16px; }
  h2 { margin: 0 0 8px; font-family: var(--font-display); font-size: 22px; }
  .field { display: flex; flex-direction: column; gap: 6px; font-size: 13px; color: var(--muted-2); max-width: 320px; }
  .field.full { max-width: none; grid-column: span 2; }
  .fields-grid { display: grid; grid-template-columns: repeat(2, minmax(0,1fr)); gap: 12px 16px; }
  select, input[type='text'], input[type='number'], input[type='password'] { height: 36px; padding: 0 10px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font: inherit; font-size: 14px; color: var(--ink); }
  .seg { display: flex; gap: 6px; }
  .seg button { height: 34px; padding: 0 14px; border: 1px solid var(--border); border-radius: 6px; background: var(--surface); font-size: 13px; }
  .seg button.on { background: var(--accent-soft); border-color: var(--accent); color: var(--accent-soft-ink); }
  .device-list { display: flex; flex-direction: column; gap: 4px; }
  .device-row { display: flex; align-items: center; gap: 10px; padding: 8px; border: 1px solid var(--line); border-radius: 8px; font-size: 14px; }
  .device-row .name { font-weight: 500; flex-grow: 1; }
  .device-row .muted { color: var(--muted); font-size: 12px; }
  .badge { font-size: 11px; padding: 2px 6px; border-radius: 4px; background: var(--accent-soft); color: var(--accent-soft-ink); }
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
</style>
