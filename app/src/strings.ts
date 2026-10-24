// All UI text, in the language picked at start: Russian or English. The tables
// live in i18n/; a third language is one more table of the same type.

import type { NodeInfo } from "./api";
import { setFormatLocale } from "./format";
import { en } from "./i18n/en";
import { ru, type Strings } from "./i18n/ru";

export type Lang = "ru" | "en";
export const LANGS: Lang[] = ["ru", "en"];
/** Each language's own name, for the menu. */
export const LANG_NAMES: Record<Lang, string> = { ru: "Русский", en: "English" };

const TABLES: Record<Lang, Strings> = { ru, en };

/** The current table. Modules read it through this binding, so it follows `setLanguage`. */
export let S: Strings = ru;
export let lang: Lang = "ru";

/** The saved choice, else Russian for a Russian Windows and English otherwise. */
export function pickLanguage(saved: string | null): Lang {
  if (saved === "ru" || saved === "en") return saved;
  return navigator.language.toLowerCase().startsWith("ru") ? "ru" : "en";
}

export function setLanguage(l: Lang) {
  lang = l;
  S = TABLES[l];
  setFormatLocale(S.locale, l);
  document.documentElement.lang = l;
}

/** Display name for a node: drive roots as "C:", synthetic blocks by kind. */
export function nodeTitle(info: Pick<NodeInfo, "kind" | "name" | "parent">, driveScanned = true): string {
  switch (info.kind) {
    case "summary":
      return S.summary;
    case "free":
      return S.freeSpace;
    case "unscanned":
      return driveScanned ? S.systemAndUnreadable : S.notScannedYet;
    default:
      return info.parent === null ? info.name.replace(/\\$/, "") : info.name;
  }
}
