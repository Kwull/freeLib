import en from './en.json';
import ru from './ru.json';
import uk from './uk.json';
import { createI18nState } from './state.svelte';

type Dict = Record<string, string>;
const dicts: Record<string, Dict> = { en, ru, uk };

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

const pluralRulesCache: Record<string, Intl.PluralRules> = {};
function pluralRules(lang: string): Intl.PluralRules {
  return (pluralRulesCache[lang] ??= new Intl.PluralRules(lang));
}

/**
 * Translates a count-dependent string. Looks up `${key}.${form}` where `form` is the
 * CLDR plural category (`one`/`few`/`many`/`other`) `Intl.PluralRules` picks for `count`
 * in the current language (en only ever produces `one`/`other`; ru/uk also use
 * `few`/`many`). Falls back to `.other`, then to the English dictionary, then to the
 * bare `key` (for callers that haven't been given plural forms yet).
 * `count` is always available as `{count}` in the template; `vars` can add more.
 */
export function tn(key: string, count: number, vars?: Record<string, string | number>): string {
  const lang = i18nState.lang;
  const dict = dicts[lang] ?? dicts.en;
  const form = pluralRules(dicts[lang] ? lang : 'en').select(count);
  const s0 =
    dict[`${key}.${form}`] ??
    dict[`${key}.other`] ??
    dicts.en[`${key}.${form}`] ??
    dicts.en[`${key}.other`] ??
    dict[key] ??
    dicts.en[key] ??
    key;
  let s = s0;
  const allVars: Record<string, string | number> = { count, ...vars };
  for (const [k, v] of Object.entries(allVars)) s = s.replaceAll(`{${k}}`, String(v));
  return s;
}
