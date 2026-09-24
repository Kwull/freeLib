import en from './en.json';
import ru from './ru.json';

type Dict = Record<string, string>;
const dicts: Record<string, Dict> = { en, ru };

function readStored(): string {
  try {
    const v = localStorage.getItem('freelib.lang');
    if (v && dicts[v]) return v;
  } catch { /* ignore */ }
  const nav = navigator.language.slice(0, 2);
  return dicts[nav] ? nav : 'en';
}

export const i18nState = $state<{ lang: string }>({ lang: readStored() });

export function setLang(lang: string) {
  if (!dicts[lang]) return;
  i18nState.lang = lang;
  try { localStorage.setItem('freelib.lang', lang); } catch { /* ignore */ }
  document.documentElement.lang = lang;
}

export function t(key: string, vars?: Record<string, string | number>): string {
  const dict = dicts[i18nState.lang] ?? dicts.en;
  let s = dict[key] ?? dicts.en[key] ?? key;
  if (vars) {
    for (const [k, v] of Object.entries(vars)) s = s.replaceAll(`{${k}}`, String(v));
  }
  return s;
}
