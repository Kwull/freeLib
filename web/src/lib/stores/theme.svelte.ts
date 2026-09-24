type Theme = 'light' | 'dark' | 'system';

function readStored(): Theme {
  try {
    const v = localStorage.getItem('freelib.theme');
    if (v === 'light' || v === 'dark' || v === 'system') return v;
  } catch { /* ignore */ }
  return 'system';
}

export const themeState = $state<{ value: Theme }>({ value: readStored() });

export function setTheme(t: Theme) {
  themeState.value = t;
  try { localStorage.setItem('freelib.theme', t); } catch { /* ignore */ }
  applyTheme();
}

export function applyTheme() {
  const root = document.documentElement;
  if (themeState.value === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', themeState.value);
}
