// Hints about well-known system places: what they are, whether they may be
// deleted, and how to free their space. The places are found by the backend
// (scanner_core::known); the texts live in i18n/known.*.ts under the same ids.

import type { Known, KnownSafety } from "./api";
import { S } from "./strings";

/** System tools a hint can open (see `open_tool` in the backend). */
export type Tool = "cleanmgr" | "storage" | "restore" | "performance" | "apps";

/** A command to copy and run, and a tool to open, for each place that has them. */
const ACTIONS: Record<string, { cmd?: string; tool?: Tool }> = {
  "win.pagefile": { tool: "performance" },
  "win.hiberfil": { cmd: "powercfg /hibernate off" },
  "win.sysvolinfo": { tool: "restore" },
  "win.windows": { tool: "cleanmgr" },
  "win.winsxs": { cmd: "DISM /Online /Cleanup-Image /StartComponentCleanup" },
  "win.update": { tool: "cleanmgr" },
  "win.driverstore": { tool: "cleanmgr" },
  "win.temp": { tool: "storage" },
  "win.logs": { tool: "cleanmgr" },
  "win.old": { tool: "storage" },
  "win.upgrade": { tool: "cleanmgr" },
  "win.programfiles": { tool: "apps" },
  "win.wer": { tool: "cleanmgr" },
  "win.inetcache": { tool: "cleanmgr" },
  "lin.system": { cmd: "sudo apt autoremove" },
  "lin.boot": { cmd: "sudo apt autoremove --purge" },
  "lin.journal": { cmd: "sudo journalctl --vacuum-size=200M" },
  "lin.apt": { cmd: "sudo apt clean" },
  "lin.dnf": { cmd: "sudo dnf clean all" },
  "lin.pacman": { cmd: "sudo paccache -rk1" },
  "lin.snap": { cmd: "sudo snap set system refresh.retain=2" },
  "lin.flatpak": { cmd: "flatpak uninstall --unused" },
  "lin.docker": { cmd: "docker system prune" },
  "lin.trash": { cmd: "gio trash --empty" },
  "mac.simulators": { cmd: "xcrun simctl delete unavailable" },
};

export interface KnownHandlers {
  openTool(tool: Tool): void;
  copy(text: string): void;
}

export function knownTitle(id: string): string {
  return S.known[id]?.title ?? id;
}

/** Whether deleting this deserves a warning: anything but a safe place's contents. */
export function needsWarning(k: Known | null | undefined): k is Known {
  return !!k && k.safety !== "safe";
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, className = "", text = ""): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (className) e.className = className;
  if (text) e.textContent = text;
  return e;
}

/** A small label with the safety of a place. */
export function safetyBadge(safety: KnownSafety, short = false): HTMLElement {
  // `known-safe` and not `safe`: plain names like `tool` are taken by other styles.
  return el("span", `known-badge known-${safety}`, short ? S.safetyShort[safety] : S.safety[safety]);
}

/** The explanation of a place, with its command and tool button, for dialogs. */
export function knownCard(k: Known, handlers: KnownHandlers): HTMLElement {
  const text = S.known[k.id];
  const card = el("div", "known-card");
  const head = el("div", "known-head");
  head.append(el("b", "", text?.title ?? k.id), safetyBadge(k.safety));
  card.append(head);
  if (k.inside) card.append(el("p", "note", S.knownInside(text?.title ?? k.id)));
  if (text) card.append(el("p", "", text.what), el("p", "", text.advice));
  const action = ACTIONS[k.id];
  if (action?.cmd) {
    const cmd = action.cmd;
    const line = el("div", "known-cmd");
    const copy = el("button", "btn small", S.copy);
    copy.type = "button";
    copy.addEventListener("click", () => handlers.copy(cmd));
    line.append(el("code", "", cmd), copy);
    card.append(line);
  }
  if (action?.tool) {
    const tool = action.tool;
    const open = el("button", "btn small", S.tools[tool]);
    open.type = "button";
    open.addEventListener("click", () => handlers.openTool(tool));
    card.append(open);
  }
  return card;
}
