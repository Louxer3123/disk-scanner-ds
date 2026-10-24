// The Disk health screen: S.M.A.R.T. of every physical disk, in the spirit of
// CrystalDiskInfo — a list of disks on the left, the chosen one on the right.
// Names and values are inserted as text, never as HTML.

import type { DiskHealth, HealthReport } from "./api";
import { formatCount, formatSize } from "./format";
import { S } from "./strings";

export interface HealthHandlers {
  refresh(): void;
  close(): void;
  /** Restart as administrator to read S.M.A.R.T. of SATA disks. */
  elevate(): void;
}

/** Temperatures from which a disk is shown as warm and as hot, °C. */
function temperatureLevel(d: DiskHealth): "" | "warm" | "hot" {
  if (d.temperature === null) return "";
  const [warm, hot] = d.rotational ? [50, 55] : [60, 70];
  return d.temperature >= hot ? "hot" : d.temperature >= warm ? "warm" : "";
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text) e.textContent = text;
  return e;
}

export class HealthView {
  private report: HealthReport | null = null;
  private selected = 0;

  constructor(
    private root: HTMLElement,
    private handlers: HealthHandlers,
  ) {}

  showLoading() {
    this.root.replaceChildren(this.header(), el("div", "health-note", S.healthLoading));
  }

  showError(text: string) {
    this.root.replaceChildren(this.header(), el("div", "health-note", text));
  }

  show(report: HealthReport) {
    this.report = report;
    if (!report.disks.some((d) => d.number === this.selected)) this.selected = report.disks[0]?.number ?? 0;
    this.render();
  }

  private render() {
    const report = this.report!;
    const body = el("div", "health-body");
    const list = el("div", "health-list");
    for (const d of report.disks) list.append(this.diskButton(d));
    const disk = report.disks.find((d) => d.number === this.selected);
    body.append(list, disk ? this.details(disk, report.elevated) : el("div", "health-note", S.healthNoDisks));
    this.root.replaceChildren(this.header(), body);
  }

  private header(): HTMLElement {
    const head = el("div", "health-head");
    head.append(el("h2", "", S.healthTitle));
    const refresh = el("button", "btn small", S.healthRefresh);
    refresh.type = "button";
    refresh.addEventListener("click", () => this.handlers.refresh());
    const close = el("button", "btn small", S.close);
    close.type = "button";
    close.addEventListener("click", () => this.handlers.close());
    head.append(el("span", "spacer"), refresh, close);
    return head;
  }

  private diskButton(d: DiskHealth): HTMLElement {
    const b = el("button", `health-disk ${d.verdict}`);
    b.type = "button";
    b.classList.toggle("selected", d.number === this.selected);
    const badge = el("span", `verdict ${d.verdict}`, S.verdicts[d.verdict] ?? d.verdict);
    const text = el("span", "health-disk-text");
    text.append(el("b", "", d.model || `PhysicalDrive${d.number}`));
    const sub = [d.letters.join(" "), formatSize(d.size)].filter(Boolean).join(" · ");
    text.append(el("span", "", sub));
    const temp = el("span", `temp ${temperatureLevel(d)}`, d.temperature === null ? "" : `${d.temperature} °C`);
    b.append(badge, text, temp);
    b.addEventListener("click", () => {
      this.selected = d.number;
      this.render();
    });
    return b;
  }

  private details(d: DiskHealth, elevated: boolean): HTMLElement {
    const box = el("div", "health-details");
    box.append(el("h3", "", d.model || `PhysicalDrive${d.number}`));

    // The verdict and the temperature, large, as CrystalDiskInfo puts them.
    const summary = el("div", "health-summary");
    const verdict = el("div", `big verdict ${d.verdict}`);
    verdict.append(el("small", "", S.healthStatus), el("b", "", S.verdicts[d.verdict] ?? d.verdict));
    if (d.lifeLeft !== null) verdict.append(el("span", "", `${d.lifeLeft} %`));
    const temp = el("div", `big temp ${temperatureLevel(d)}`);
    temp.append(el("small", "", S.healthTemperature), el("b", "", d.temperature === null ? "—" : `${d.temperature} °C`));
    summary.append(verdict, temp);
    box.append(summary);

    if (d.reasons.length) {
      const reasons = el("ul", "health-reasons");
      for (const r of d.reasons) reasons.append(el("li", "", r));
      box.append(reasons);
    }
    if (d.smartError) box.append(this.smartNote(d, elevated));

    const facts: [string, string | null][] = [
      [S.healthLetters, d.letters.join(" ") || null],
      [S.healthInterface, d.bus],
      [S.healthType, d.rotational === null ? null : d.rotational ? "HDD" : "SSD"],
      [S.healthSize, formatSize(d.size)],
      [S.healthFirmware, d.firmware || null],
      [S.healthSerial, d.serial || null],
      [S.healthPowerOn, d.powerOnHours === null ? null : S.healthHours(d.powerOnHours)],
      [S.healthPowerCycles, d.powerCycles === null ? null : formatCount(d.powerCycles)],
      [S.healthUnsafeShutdowns, d.unsafeShutdowns === null ? null : formatCount(d.unsafeShutdowns)],
      [S.healthWritten, d.bytesWritten === null ? null : formatSize(d.bytesWritten)],
      [S.healthRead, d.bytesRead === null ? null : formatSize(d.bytesRead)],
      [S.healthLife, d.lifeLeft === null ? null : `${d.lifeLeft} %`],
      [S.healthSpare, d.availableSpare === null ? null : `${d.availableSpare} % (${S.healthSpareThreshold} ${d.spareThreshold} %)`],
      [S.healthMediaErrors, d.mediaErrors === null ? null : formatCount(d.mediaErrors)],
      [S.healthReallocated, d.reallocatedSectors === null ? null : formatCount(d.reallocatedSectors)],
      [S.healthPending, d.pendingSectors === null ? null : formatCount(d.pendingSectors)],
      [S.healthUncorrectable, d.uncorrectableSectors === null ? null : formatCount(d.uncorrectableSectors)],
    ];
    const grid = el("dl", "health-facts");
    for (const [name, value] of facts) {
      if (value === null) continue;
      grid.append(el("dt", "", name), el("dd", "", value));
    }
    box.append(grid);

    if (d.attributes.length) box.append(this.attributeTable(d));
    return box;
  }

  private smartNote(d: DiskHealth, elevated: boolean): HTMLElement {
    const note = el("div", "health-smart-note");
    if (d.smartError === "needs_admin" && !elevated) {
      note.append(el("p", "", S.healthNeedsAdmin));
      const go = el("button", "btn small primary", S.healthElevate);
      go.type = "button";
      go.addEventListener("click", () => this.handlers.elevate());
      note.append(go);
    } else {
      note.append(el("p", "", S.healthUnsupported));
    }
    return note;
  }

  private attributeTable(d: DiskHealth): HTMLElement {
    const table = el("table", "health-attrs");
    const head = el("tr");
    for (const h of ["ID", S.healthAttrName, S.healthAttrCurrent, S.healthAttrWorst, S.healthAttrThreshold, S.healthAttrRaw]) {
      head.append(el("th", "", h));
    }
    table.append(head);
    for (const a of d.attributes) {
      const tr = el("tr");
      const failing = a.threshold > 0 && a.threshold < 0xfe && a.current > 0 && a.current <= a.threshold;
      const watched = [0x05, 0xc5, 0xc6].includes(a.id) && a.raw > 0;
      if (failing) tr.className = "bad";
      else if (watched) tr.className = "caution";
      const raw = `${a.raw.toString(16).toUpperCase().padStart(12, "0")} (${formatCount(a.raw)})`;
      for (const v of [a.id.toString(16).toUpperCase().padStart(2, "0"), a.name, a.current, a.worst, a.threshold, raw]) {
        tr.append(el("td", "", String(v)));
      }
      table.append(tr);
    }
    const wrap = el("div", "health-attrs-wrap");
    wrap.append(el("h4", "", S.healthAttributes), table);
    return wrap;
  }
}
