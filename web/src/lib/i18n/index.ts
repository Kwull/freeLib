import en from './en.json';
import ru from './ru.json';
import { createI18nState } from './state.svelte';

type Dict = Record<string, string>;
const dicts: Record<string, Dict> = { en, ru };

export const i18nState = createI18nState((lang) => !!dicts[lang]);

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
