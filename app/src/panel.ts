// Side panel: the contents of the shown folder, or search results, as a list
// sorted by size. Names are inserted as text, never as HTML.

import type { NodeInfo } from "./api";
import { formatPercent, formatSize } from "./format";
import { S } from "./strings";

const ROW_ICONS: Record<string, string> = {
  dir: '<svg viewBox="0 0 24 24"><path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
  file: '<svg viewBox="0 0 24 24"><path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/></svg>',
  link: '<svg viewBox="0 0 24 24"><path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3a4 4 0 0 0 5.7 5.7l1-1"/></svg>',
  drive: '<svg viewBox="0 0 24 24"><rect x="3" y="6" width="18" height="12" rx="2"/><circle cx="17" cy="12" r="1.2"/></svg>',
  space: '<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="7" stroke-dasharray="3 3"/></svg>',
};

export interface PanelHandlers {
  /** Single click: zoom into a folder, or show where a search result is. */
  activate(info: NodeInfo, fromSearch: boolean): void;
  /** Double click on a file. */
  open(info: NodeInfo): void;
  menu(info: NodeInfo, x: number, y: number): void;
  hover(info: NodeInfo | null): void;
  /** Chart colour of the node, if it is drawn. */
  colorOf(info: NodeInfo): string | null;
  title(info: NodeInfo): string;
}

export class Panel {
  private head = document.createElement("div");
  private list = document.createElement("div");
  private foot = document.createElement("div");

  constructor(
    root: HTMLElement,
    private handlers: PanelHandlers,
  ) {
    this.head.className = "panel-head";
    this.list.className = "panel-list";
    this.foot.className = "panel-foot";
    root.replaceChildren(this.head, this.list, this.foot);
    this.list.addEventListener("mouseleave", () => this.handlers.hover(null));
  }

  showChildren(items: NodeInfo[], total: number, restAllocated: number, whole: number, summary: boolean) {
    this.setHead(summary ? S.drives : S.contents, S.items(total));
    this.renderRows(items, whole, false);
    this.foot.textContent = total > items.length ? `${S.andMore(total - items.length)} · ${formatSize(restAllocated)}` : "";
    this.foot.hidden = !this.foot.textContent;
  }

  showSearch(total: number, allocated: number, items: NodeInfo[], scope: string, whole: number) {
    this.setHead(total ? S.found(total, formatSize(allocated)) : S.nothingFound, scope ? `${S.inScope} ${scope}` : "");
    this.renderRows(items, whole, true);
    this.foot.textContent = total > items.length ? S.largestShown(items.length) : "";
    this.foot.hidden = !this.foot.textContent;
  }

  showMessage(text: string) {
    this.setHead(text, "");
    this.list.replaceChildren();
    this.foot.hidden = true;
  }

  private setHead(title: string, sub: string) {
    const t = document.createElement("div");
    t.className = "panel-title";
    t.textContent = title;
    const s = document.createElement("div");
    s.className = "panel-sub";
    s.textContent = sub;
    this.head.replaceChildren(t, s);
  }

  private renderRows(items: NodeInfo[], whole: number, fromSearch: boolean) {
    this.list.replaceChildren(...items.map((info) => this.row(info, whole, fromSearch)));
    this.list.scrollTop = 0;
  }

  private row(info: NodeInfo, whole: number, fromSearch: boolean): HTMLElement {
    const row = document.createElement("div");
    row.className = "row";
    row.classList.toggle("clickable", info.kind === "dir" || fromSearch);

    const swatch = document.createElement("span");
    swatch.className = "swatch";
    const color = this.handlers.colorOf(info);
    if (color) swatch.style.background = color;
    else swatch.classList.add("hidden-seg");

    const icon = document.createElement("span");
    icon.className = "row-icon";
    const iconKey =
      info.kind === "free" || info.kind === "unscanned" ? "space" : info.parent === null ? "drive" : info.kind;
    icon.innerHTML = ROW_ICONS[iconKey] ?? ROW_ICONS.file;

    const text = document.createElement("span");
    text.className = "row-text";
    const name = document.createElement("span");
    name.className = "row-name";
    name.textContent = this.handlers.title(info);
    text.append(name);
    if (fromSearch) {
      const where = document.createElement("span");
      where.className = "row-path";
      where.textContent = info.path.replace(/\\[^\\]*$/, "") || info.path;
      text.append(where);
    }
    row.title = info.path;

    const size = document.createElement("span");
    size.className = "row-size";
    size.textContent = formatSize(info.used);
    const share = document.createElement("span");
    share.className = "row-share";
    share.textContent = formatPercent(info.allocated, whole);

    const bar = document.createElement("span");
    bar.className = "row-bar";
    const fill = document.createElement("span");
    fill.style.width = `${whole > 0 ? Math.min(100, (info.allocated * 100) / whole) : 0}%`;
    if (color) fill.style.background = color;
    bar.append(fill);

    row.append(swatch, icon, text, size, share, bar);
    row.addEventListener("click", () => this.handlers.activate(info, fromSearch));
    row.addEventListener("dblclick", () => {
      if (info.kind === "file") this.handlers.open(info);
    });
    row.addEventListener("contextmenu", (e) => {
      e.preventDefault();
      this.handlers.menu(info, e.clientX, e.clientY);
    });
    row.addEventListener("mouseenter", () => this.handlers.hover(info));
    return row;
  }
}
