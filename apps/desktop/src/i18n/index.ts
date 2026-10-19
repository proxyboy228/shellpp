// i18n catalog (docs/decisions.md, decision 22): English only, flat dotted
// keys such as `hosts.search.placeholder`.
//
// Catalog files: `en.json` (app shell, lock, settings, notifications,
// errors, `ssh_reason.*`, common), `hosts.en.json` (hosts screen and panels;
// S7 in stage 2), `terminal.en.json` (S6: tabs, terminal, splits, terminal
// sidebar, session end, paste, close confirmation) and `connect.en.json`
// (S7: connection screen, prompts, host key dialogs, new tab page, search,
// Known Hosts, "+ -> Key"). The loader merges them; a key must not appear in
// two files (checked by the i18n test).
//
// Placeholders use `{name}` and are replaced from `params`.
//
// Owner: lead.

import connectEn from './connect.en.json';
import en from './en.json';
import hostsEn from './hosts.en.json';
import terminalEn from './terminal.en.json';

export type Catalog = Readonly<Record<string, string>>;

/** The catalog files, for tests (duplicate and usage checks). */
export const CATALOG_FILES: Readonly<Record<string, Catalog>> = {
  'en.json': en,
  'hosts.en.json': hostsEn,
  'terminal.en.json': terminalEn,
  'connect.en.json': connectEn,
};

const catalog: Catalog = { ...en, ...hostsEn, ...terminalEn, ...connectEn };

export type TranslationParams = Readonly<Record<string, string | number>>;

/** True when the key exists. */
export function hasKey(key: string): boolean {
  return Object.hasOwn(catalog, key);
}

/**
 * The string for `key` with `{name}` placeholders filled from `params`.
 * A missing key returns the key itself (and warns in development).
 */
export function t(key: string, params?: TranslationParams): string {
  const template = catalog[key];
  if (template === undefined) {
    if (import.meta.env.DEV) {
      console.warn(`Missing i18n key: ${key}`);
    }
    return key;
  }
  if (params === undefined) {
    return template;
  }
  return template.replace(/\{(\w+)\}/g, (match, name: string) =>
    Object.hasOwn(params, name) ? String(params[name]) : match,
  );
}
