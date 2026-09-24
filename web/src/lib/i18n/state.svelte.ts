function readStored(has: (lang: string) => boolean): string {
  try {
    const v = localStorage.getItem('freelib.lang');
    if (v && has(v)) return v;
  } catch { /* ignore */ }
  const nav = navigator.language.slice(0, 2);
  return has(nav) ? nav : 'en';
}

export function createI18nState(has: (lang: string) => boolean) {
  const state = $state<{ lang: string }>({ lang: readStored(has) });
  return state;
}
