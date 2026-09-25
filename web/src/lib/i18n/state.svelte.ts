// The default UI language is English. We only ever deviate from it when the user
// explicitly picked a language before (stored in localStorage), never from the
// browser's navigator.language — a browser set to Russian should not silently
// flip a first-time visitor's UI to Russian.
function readStored(has: (lang: string) => boolean): string {
  try {
    const v = localStorage.getItem('freelib.lang');
    if (v && has(v)) return v;
  } catch { /* ignore */ }
  return 'en';
}

export function createI18nState(has: (lang: string) => boolean) {
  const state = $state<{ lang: string }>({ lang: readStored(has) });
  return state;
}
