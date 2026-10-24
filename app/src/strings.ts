// All UI text in one place, so a second language is a matter of adding a table.

import type { NodeInfo } from "./api";
import { plural } from "./format";

export const S = {
  appTitle: "Disk Scanner",
  summary: "Сводка",
  back: "Назад",
  up: "Вверх",
  rescan: "Пересканировать",
  drives: "Диски",
  scanning: "Сканирование…",
  notScanned: "Не просканирован",
  notReady: "Нет носителя",
  free: "свободно",
  of: "из",
  used: "Занято",
  freeSpace: "Свободное место",
  notScannedYet: "Ещё не просканировано",
  systemAndUnreadable: "Системное и недоступное",
  accessDenied: "Нет доступа — размер неполный",
  link: "Ссылка (не учитывается)",
  inSummary: "Показывать в сводке",
  emptySummary: "В сводке нет дисков. Щёлкните диск правой кнопкой и включите «Показывать в сводке».",
  emptyFolder: "Папка пуста",
  scanError: "Ошибка сканирования",
  files: (n: number) => plural(n, ["файл", "файла", "файлов"]),
  folders: (n: number) => plural(n, ["папка", "папки", "папок"]),
  scannedFiles: (n: number) => `${plural(n, ["файл", "файла", "файлов"])} просканировано`,
};

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
