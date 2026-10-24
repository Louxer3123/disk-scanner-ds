// Side panel: the contents of the shown folder, the space per file type, or
// search results, as lists sorted by size. Names are inserted as text, never as
// HTML.

import type { ChangedNode, Changes, FileTypes, NodeInfo, Snapshot } from "./api";
import { formatDate, formatDelta, formatPercent, formatSize } from "./format";
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
  /** Colour of a file category. */
  categoryColor(category: number): string;
  /** A tab was picked. */
  tab(tab: PanelTab): void;
  /** Click on an extension: find its files. */
  findExtension(ext: string): void;
  /** Colour of a change, as on the chart. */
  changeColor(delta: number, added: boolean, scale: number): string;
  compare(id: string | null): void;
  saveSnapshot(): void;
  deleteSnapshot(id: string): void;
}

export type PanelTab = "contents" | "types" | "changes";

/** What the Changes tab shows for a drive or folder. */
export interface ChangesView {
  snapshots: Snapshot[];
  changes: Changes;
  /** The drive is scanned, so a snapshot can be saved. */
  canSave: boolean;
}

/** One line of the types list. */
interface StatRow {
  color: string;
  name: string;
  sub: string;
  allocated: number;
  /** Shown instead of the size. */
  sizeText?: string;
  title?: string;
  onClick?: () => void;
}

export class Panel {
  private tabs = document.createElement("div");
  private tabButtons = new Map<PanelTab, HTMLButtonElement>();
  private head = document.createElement("div");
  private list = document.createElement("div");
  private foot = document.createElement("div");

  constructor(
    root: HTMLElement,
    private handlers: PanelHandlers,
  ) {
    this.tabs.className = "panel-tabs";
    this.tabs.setAttribute("role", "tablist");
    for (const [tab, label] of [["contents", S.contentsTab], ["types", S.typesTab], ["changes", S.changesTab]] as const) {
      const b = document.createElement("button");
      b.type = "button";
      b.setAttribute("role", "tab");
      b.textContent = label;
      b.addEventListener("click", () => this.handlers.tab(tab));
      this.tabButtons.set(tab, b);
      this.tabs.append(b);
    }
    this.head.className = "panel-head";
    this.list.className = "panel-list";
    this.foot.className = "panel-foot";
    root.replaceChildren(this.tabs, this.head, this.list, this.foot);
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

  /** Shows the tab bar with `active` selected, or hides it (search results). */
  setTab(active: PanelTab | null) {
    this.tabs.hidden = active === null;
    for (const [tab, b] of this.tabButtons) {
      b.classList.toggle("active", tab === active);
      b.setAttribute("aria-selected", String(tab === active));
    }
  }

  showTypes(types: FileTypes, whole: number) {
    this.setHead(S.typesTab, S.extensionCount(types.total));
    const rows: HTMLElement[] = types.categories.map((c) =>
      this.statRow(
        {
          color: this.handlers.categoryColor(c.category),
          name: S.categories[c.category] ?? S.categories[0],
          sub: S.files(c.files),
          allocated: c.allocated,
        },
        whole,
      ),
    );
    if (types.extensions.length) {
      const h = document.createElement("div");
      h.className = "panel-section";
      h.textContent = S.extensions;
      rows.push(h);
    }
    for (const e of types.extensions) {
      rows.push(
        this.statRow(
          {
            color: this.handlers.categoryColor(e.category),
            name: e.extension ? `.${e.extension}` : S.noExtension,
            sub: `${S.categories[e.category] ?? S.categories[0]} · ${S.files(e.files)}`,
            allocated: e.allocated,
            title: e.extension ? S.findExtension(e.extension) : undefined,
            onClick: e.extension ? () => this.handlers.findExtension(e.extension) : undefined,
          },
          whole,
        ),
      );
    }
    this.list.replaceChildren(...rows);
    this.list.scrollTop = 0;
    const rest = types.total - types.extensions.length;
    this.foot.textContent = rest > 0 ? `${S.moreExtensions(rest)} · ${formatSize(types.restAllocated)}` : "";
    this.foot.hidden = !this.foot.textContent;
  }

  showChanges(v: ChangesView) {
    const c = v.changes;
    this.setHead(
      c.since === null ? S.changesTab : `${formatDelta(c.delta)} ${S.sinceDate(formatDate(c.since))}`,
      "",
    );
    const rows: HTMLElement[] = [this.snapshotControls(v)];
    if (c.snapshot === null) {
      rows.push(this.note(v.snapshots.length ? S.pickSnapshot : S.noSnapshots));
    } else if (!c.children.length && !c.removed.length) {
      rows.push(this.note(S.noChanges));
    } else {
      const scale = Math.max(1, ...c.children.map((x) => Math.abs(x.delta)));
      if (c.children.length) rows.push(this.section(S.changedHere));
      for (const x of c.children) rows.push(this.changeRow(x, scale, false));
      if (c.files.length) rows.push(this.section(S.grownFiles));
      const fileScale = Math.max(1, ...c.files.map((x) => x.delta));
      for (const x of c.files) rows.push(this.changeRow(x, fileScale, true));
      if (c.removed.length) rows.push(this.section(`${S.removedHead} · ${formatSize(c.removedAllocated)}`));
      for (const r of c.removed) {
        rows.push(
          this.statRow(
            {
              color: this.handlers.changeColor(-r.allocated, false, c.removedAllocated),
              name: r.name,
              sub: r.path,
              allocated: r.allocated,
              sizeText: formatDelta(-r.allocated),
              title: r.path,
            },
            c.removedAllocated,
          ),
        );
      }
      const more = c.removedTotal - c.removed.length;
      if (more > 0) {
        const rest = c.removedAllocated - c.removed.reduce((sum, r) => sum + r.allocated, 0);
        rows.push(this.note(S.moreRemoved(more, formatSize(rest))));
      }
    }
    this.list.replaceChildren(...rows);
    this.list.scrollTop = 0;
    this.foot.hidden = true;
  }

  private snapshotControls(v: ChangesView): HTMLElement {
    const box = document.createElement("div");
    box.className = "panel-controls";
    const label = document.createElement("label");
    label.textContent = S.compareWith;
    const select = document.createElement("select");
    const none = new Option(S.noCompare, "");
    select.append(none);
    for (const s of v.snapshots) {
      select.append(new Option(S.snapshotOption(formatDate(s.created), formatSize(s.allocated)), s.id));
    }
    select.value = v.changes.snapshot ?? "";
    select.disabled = !v.snapshots.length;
    select.addEventListener("change", () => this.handlers.compare(select.value || null));
    label.append(select);

    const buttons = document.createElement("div");
    buttons.className = "panel-buttons";
    const save = document.createElement("button");
    save.type = "button";
    save.className = "btn small";
    save.textContent = S.saveSnapshot;
    save.disabled = !v.canSave;
    save.addEventListener("click", () => this.handlers.saveSnapshot());
    buttons.append(save);
    if (v.changes.snapshot) {
      const id = v.changes.snapshot;
      const del = document.createElement("button");
      del.type = "button";
      del.className = "btn small";
      del.textContent = S.deleteSnapshot;
      del.addEventListener("click", () => this.handlers.deleteSnapshot(id));
      buttons.append(del);
    }
    box.append(label, buttons);
    return box;
  }

  private section(text: string): HTMLElement {
    const h = document.createElement("div");
    h.className = "panel-section";
    h.textContent = text;
    return h;
  }

  private note(text: string): HTMLElement {
    const p = document.createElement("div");
    p.className = "panel-note";
    p.textContent = text;
    return p;
  }

  /** A changed entry: the change in the size column, before and after below the name. */
  private changeRow(c: ChangedNode, scale: number, withPath: boolean): HTMLElement {
    const row = this.row(c.info, 0, withPath);
    const color = this.handlers.changeColor(c.delta, c.added, scale);
    const swatch = row.querySelector<HTMLElement>(".swatch")!;
    swatch.classList.remove("hidden-seg");
    swatch.style.background = color;
    const size = row.querySelector<HTMLElement>(".row-size")!;
    size.textContent = formatDelta(c.delta);
    size.classList.add(c.delta >= 0 ? "grew" : "shrank");
    row.querySelector<HTMLElement>(".row-share")!.textContent = "";
    if (!withPath) {
      const sub = document.createElement("span");
      sub.className = "row-path";
      sub.textContent = c.added
        ? `${S.newItem}, ${formatSize(c.info.allocated)}`
        : S.wasNow(formatSize(c.info.allocated - c.delta), formatSize(c.info.allocated));
      row.querySelector(".row-text")!.append(sub);
    }
    const fill = row.querySelector<HTMLElement>(".row-bar span")!;
    fill.style.width = `${Math.min(100, (Math.abs(c.delta) * 100) / scale)}%`;
    fill.style.background = color;
    return row;
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

  private statRow(r: StatRow, whole: number): HTMLElement {
    const row = document.createElement("div");
    row.className = "row";
    const swatch = document.createElement("span");
    swatch.className = "swatch";
    swatch.style.background = r.color;
    const text = document.createElement("span");
    text.className = "row-text";
    const name = document.createElement("span");
    name.className = "row-name";
    name.textContent = r.name;
    const sub = document.createElement("span");
    sub.className = "row-path";
    sub.textContent = r.sub;
    text.append(name, sub);
    const size = document.createElement("span");
    size.className = "row-size";
    size.textContent = r.sizeText ?? formatSize(r.allocated);
    const share = document.createElement("span");
    share.className = "row-share";
    share.textContent = r.sizeText ? "" : formatPercent(r.allocated, whole);
    const bar = document.createElement("span");
    bar.className = "row-bar";
    const fill = document.createElement("span");
    fill.style.width = `${whole > 0 ? Math.min(100, (r.allocated * 100) / whole) : 0}%`;
    fill.style.background = r.color;
    bar.append(fill);
    // No icon: the text spans the icon column too.
    text.style.gridColumn = "2 / 4";
    row.append(swatch, text, size, share, bar);
    if (r.title) row.title = r.title;
    if (r.onClick) {
      row.classList.add("clickable");
      row.addEventListener("click", r.onClick);
    }
    return row;
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
