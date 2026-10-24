// Number formatting, matching scanner_core::format_size but with Russian units.

const UNITS = ["КБ", "МБ", "ГБ", "ТБ", "ПБ", "ЭБ"];

/** Binary units, three significant digits: `512 Б`, `1,50 КБ`, `15,0 МБ`, `150 ГБ`. */
export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} Б`;
  let value = bytes / 1024;
  let unit = 0;
  // Switch units before rounding could print "1024 КБ".
  while (value >= 999.5 && unit + 1 < UNITS.length) {
    value /= 1024;
    unit++;
  }
  const decimals = value < 9.995 ? 2 : value < 99.95 ? 1 : 0;
  return `${value.toFixed(decimals).replace(".", ",")} ${UNITS[unit]}`;
}

/** A change in size with its sign: `+1,20 ГБ`, `−300 МБ`, `0 Б`. */
export function formatDelta(bytes: number): string {
  if (bytes === 0) return "0 Б";
  return `${bytes > 0 ? "+" : "−"}${formatSize(Math.abs(bytes))}`;
}

/** Date and time of a Unix timestamp: `24 окт. 2026 г., 21:30`. */
export function formatDate(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString("ru-RU", {
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
  return d.toLocaleDateString("ru-RU", { day: "numeric", month: "short", year: sameYear ? undefined : "numeric" });
}

export function formatPercent(part: number, whole: number): string {
  if (whole <= 0) return "0 %";
  const p = (part * 100) / whole;
  return `${p < 10 ? p.toFixed(1).replace(".", ",") : Math.round(p)} %`;
}

/** Russian plural: `plural(5, ["файл", "файла", "файлов"])` → "5 файлов". */
export function plural(n: number, forms: [string, string, string]): string {
  const mod10 = n % 10;
  const mod100 = n % 100;
  const form = mod10 === 1 && mod100 !== 11 ? 0 : mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14) ? 1 : 2;
  return `${n.toLocaleString("ru-RU")} ${forms[form]}`;
}
