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
  // Context menus
  zoom: "Приблизить",
  hideContents: "Скрыть содержимое",
  showContents: "Показать содержимое",
  rescanFolder: "Пересканировать папку",
  rescanDrive: "Пересканировать диск",
  open: "Открыть",
  openDrive: "Открыть диск",
  openInExplorer: "Открыть в Проводнике",
  reveal: "Показать в Проводнике",
  copyPath: "Копировать путь",
  recycle: "В корзину",
  deleteForever: "Удалить навсегда…",
  deleteLink: "Удалить ссылку…",
  emptyBin: "Очистить корзину…",
  contentsHidden: "Содержимое скрыто",
  // Toolbar
  recycleBin: "Корзина",
  recycleBinEmpty: "Корзина пуста",
  apps: "Установка и удаление программ",
  appsHint: "После удаления программ пересканируйте диск",
  more: "Ещё",
  explorerMenu: "Пункт «Показать занятое место» в Проводнике",
  // Dialogs
  cancel: "Отмена",
  recycleTitle: "Переместить в корзину?",
  deleteTitle: "Удалить навсегда?",
  deleteLinkTitle: "Удалить ссылку?",
  deleteLinkNote: "Удаляется только сама ссылка, папка, на которую она указывает, останется.",
  cannotUndo: "Это действие нельзя отменить.",
  deleteButton: "Удалить навсегда",
  emptyBinTitle: "Очистить корзину?",
  emptyBinButton: "Очистить",
  // Results
  recycled: (name: string) => `«${name}» перемещено в корзину`,
  deleted: (name: string) => `«${name}» удалено`,
  binEmptied: "Корзина очищена",
  pathCopied: "Путь скопирован",
  scanningFolder: (path: string) => `Сканирование папки ${path}…`,
  explorerMenuOn: "Пункт добавлен в контекстное меню папок и дисков",
  explorerMenuOff: "Пункт убран из контекстного меню",
  failed: (what: string, err: unknown) => `${what}: ${String(err)}`,
  items: (n: number) => plural(n, ["объект", "объекта", "объектов"]),
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
