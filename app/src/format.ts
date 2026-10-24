// Number formatting, matching scanner_core::format_size, in the UI's language:
// Russian units and decimal comma, or English ones with a point.

const UNITS = {
  ru: { bytes: "Б", units: ["КБ", "МБ", "ГБ", "ТБ", "ПБ", "ЭБ"], comma: true },
  en: { bytes: "B", units: ["KB", "MB", "GB", "TB", "PB", "EB"], comma: false },
};

let locale = "ru-RU";
let units = UNITS.ru;

/** Called by `setLanguage`: `tag` is a BCP 47 locale such as `en-US`. */
export function setFormatLocale(tag: string, lang: "ru" | "en") {
  locale = tag;
  units = UNITS[lang];
}

function decimal(text: string): string {
  return units.comma ? text.replace(".", ",") : text;
}

/** Binary units, three significant digits: `512 Б`, `1,50 КБ`, `15,0 МБ`, `150 ГБ`. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} ${units.bytes}`;
  let value = bytes / 1024;
  let unit = 0;
  // Switch units before rounding could print "1024 КБ".
  while (value >= 999.5 && unit + 1 < units.units.length) {
    value /= 1024;
    unit++;
  }
  const decimals = value < 9.995 ? 2 : value < 99.95 ? 1 : 0;
  return `${decimal(value.toFixed(decimals))} ${units.units[unit]}`;
}

/** A change in size with its sign: `+1,20 ГБ`, `−300 МБ`, `0 Б`. */
export function formatDelta(bytes: number): string {
  if (bytes === 0) return `0 ${units.bytes}`;
  return `${bytes > 0 ? "+" : "−"}${formatSize(Math.abs(bytes))}`;
}

/** A count with digit grouping: `1 234 567` or `1,234,567`. */
export function formatCount(n: number): string {
  return n.toLocaleString(locale);
}

/** Date and time of a Unix timestamp: `24 окт. 2026 г., 21:30`. */
export function formatDate(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString(locale, {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** Short date of a Unix timestamp: `24 окт.`, with the year if it is not this one. */
export function formatShortDate(seconds: number): string {
  const d = new Date(seconds * 1000);
  const sameYear = d.getFullYear() === new Date().getFullYear();
  return d.toLocaleDateString(locale, { day: "numeric", month: "short", year: sameYear ? undefined : "numeric" });
}

export function formatPercent(part: number, whole: number): string {
  if (whole <= 0) return "0 %";
  const p = (part * 100) / whole;
  return `${p < 10 ? decimal(p.toFixed(1)) : Math.round(p)} %`;
}

/** Russian plural: `plural(5, ["файл", "файла", "файлов"])` → "5 файлов". */
export function plural(n: number, forms: [string, string, string]): string {
  const mod10 = n % 10;
  const mod100 = n % 100;
  const form = mod10 === 1 && mod100 !== 11 ? 0 : mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14) ? 1 : 2;
  return `${formatCount(n)} ${forms[form]}`;
}

/** English plural: `pluralEn(5, "file", "files")` → "5 files". */
export function pluralEn(n: number, one: string, many: string): string {
  return `${formatCount(n)} ${n === 1 ? one : many}`;
}
