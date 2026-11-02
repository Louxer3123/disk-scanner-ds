// English texts for the known system places; the same keys as known.ru.ts.

import type { KnownText } from "./known.ru";

export const knownEn: Record<string, KnownText> = {
  "win.pagefile": {
    title: "Page file",
    what: "Windows virtual memory: the system moves data here from RAM when RAM runs short.",
    advice:
      "Do not delete it by hand: Windows will not let you, and without it programs may crash. To make it smaller: Performance Options → Advanced → Virtual memory.",
  },
  "win.hiberfil": {
    title: "Hibernation file",
    what: "The contents of RAM for hibernation and Fast Startup; usually 40–75% of the RAM size.",
    advice:
      "It cannot be deleted by hand. If you do not need hibernation, turn it off with the command in a terminal run as administrator; the file goes away (and so does Fast Startup).",
  },
  "win.swapfile": {
    title: "App swap file",
    what: "Swap for Microsoft Store apps.",
    advice: "Do not delete it: Windows manages it together with the page file, and it is usually small.",
  },
  "win.dumpstack": {
    title: "Crash dump log",
    what: "A system file for writing a memory dump on a crash.",
    advice: "Do not delete it: it is in use and small.",
  },
  "win.recycle": {
    title: "Recycle Bin",
    what: "Files deleted to the Recycle Bin by every user of this drive.",
    advice: "Empty the Recycle Bin with the bin button on the toolbar or in Explorer. Do not delete the folder itself.",
  },
  "win.sysvolinfo": {
    title: "Restore points",
    what: "System restore points, volume shadow copies and system data (indexing, File History).",
    advice: "Do not delete it by hand (there is no access anyway). To delete old points or limit their space: System Protection → Configure.",
  },
  "win.recovery": {
    title: "Recovery environment",
    what: "Files of the Windows Recovery Environment (WinRE).",
    advice: "Do not delete them: without them recovery cannot start when Windows fails to boot.",
  },
  "win.windows": {
    title: "Windows folder",
    what: "The operating system itself.",
    advice: "Do not delete anything here by hand. Disk Cleanup (Clean up system files) and Storage Sense free space safely.",
  },
  "win.winsxs": {
    title: "Component store (WinSxS)",
    what: "Windows components and their earlier versions kept after updates. Many files here are hard links to system files, so the folder really takes less than shown.",
    advice: "Do not delete files by hand: updates and the system itself break. Old component versions are removed with the command, run as administrator.",
  },
  "win.update": {
    title: "Windows Update downloads",
    what: "Downloaded Windows update files.",
    advice: "Cleaned up with Disk Cleanup → Clean up system files → Windows Update Cleanup.",
  },
  "win.installer": {
    title: "Windows Installer cache",
    what: "Copies of the MSI and MSP packages of installed programs.",
    advice: "Do not delete it: without these, programs cannot be updated, repaired or uninstalled.",
  },
  "win.driverstore": {
    title: "Driver store",
    what: "Packages of installed drivers.",
    advice: "Do not delete files by hand. Disk Cleanup removes old driver versions (Device driver packages).",
  },
  "win.temp": {
    title: "Temporary files",
    what: "Temporary files of programs and installers.",
    advice: "Safe to delete. Close programs first: files in use cannot be deleted and can be skipped. Easiest: Storage Sense → Temporary files.",
  },
  "win.dumps": {
    title: "Memory dumps",
    what: "Memory snapshots taken after system and program crashes, needed only to find the cause.",
    advice: "Safe to delete unless you are investigating a crash.",
  },
  "win.logs": {
    title: "Windows logs",
    what: "Setup and servicing logs (CBS, DISM and others).",
    advice: "Old logs can be deleted; current ones are in use. Disk Cleanup does it more easily.",
  },
  "win.old": {
    title: "Previous Windows installation",
    what: "A copy of the earlier Windows after a major update, kept for going back; removed by itself after about 10 days.",
    advice:
      "If you will not go back, delete it with Storage Sense → Temporary files → Previous Windows installation(s). Not by hand: you lack the rights and leftovers stay.",
  },
  "win.upgrade": {
    title: "Windows upgrade files",
    what: "Temporary files of a Windows version upgrade.",
    advice: "Removed with Disk Cleanup → Clean up system files → Temporary Windows installation files.",
  },
  "win.programfiles": {
    title: "Installed programs",
    what: "The files of installed programs.",
    advice: "Do not delete program folders by hand: uninstall programs through Apps in Settings.",
  },
  "win.programdata": {
    title: "Shared program data",
    what: "Settings and data of programs, shared by all users.",
    advice: "Do not delete it by hand, or programs stop working. The folder of a program you already uninstalled can go if you are sure it is not needed.",
  },
  "win.pkgcache": {
    title: "Installer cache",
    what: "Setup packages of Visual Studio, .NET and other programs, needed to update and uninstall them.",
    advice: "Do not delete it, or the program cannot be updated or uninstalled.",
  },
  "win.wer": {
    title: "Error reports",
    what: "Windows error reports about programs.",
    advice: "Safe to delete; Disk Cleanup does it too (Windows error reports).",
  },
  "win.appdata": {
    title: "User program data",
    what: "Settings, caches and data of this user's programs.",
    advice: "Do not delete it as a whole: program settings and data would be lost. Clear caches in the programs themselves (in the browser, for one).",
  },
  "win.inetcache": {
    title: "Internet cache",
    what: "The cache of the web components built into Windows.",
    advice: "Safe to delete; Disk Cleanup clears it too (Temporary Internet Files).",
  },
  "lin.swapfile": {
    title: "Swap file",
    what: "Swap space: the system moves memory here when RAM runs short.",
    advice: "Do not delete it while it is on. To make it smaller: turn it off (swapoff), create it again at the size you want and turn it on.",
  },
  "lin.system": {
    title: "System files",
    what: "Files of the operating system and installed packages.",
    advice: "Do not delete them by hand. Remove packages you do not need with the package manager (on Ubuntu and Debian, with the command).",
  },
  "lin.boot": {
    title: "Boot loader and kernels",
    what: "Linux kernels and boot loader files.",
    advice: "Do not delete them by hand, or the system may not boot. The package manager removes old kernels.",
  },
  "lin.lostfound": {
    title: "Recovered fragments",
    what: "Where the disk check (fsck) puts file fragments it finds.",
    advice: "Do not delete the folder itself. Look through its contents and delete them if nothing there is worth keeping.",
  },
  "lin.journal": {
    title: "systemd journal",
    what: "The journald system log.",
    advice: "Do not delete files by hand. The command trims the journal to the size you want.",
  },
  "lin.apt": {
    title: "APT package cache",
    what: "Downloaded .deb packages.",
    advice: "Safe to clear with the command; packages are downloaded again when needed.",
  },
  "lin.dnf": {
    title: "DNF package cache",
    what: "Downloaded packages and repository metadata.",
    advice: "Safe to clear with the command.",
  },
  "lin.pacman": {
    title: "pacman package cache",
    what: "Downloaded packages, older versions included.",
    advice: "The command (from pacman-contrib) keeps only the latest version of each package.",
  },
  "lin.snap": {
    title: "Snap packages",
    what: "Installed snap packages and their earlier versions.",
    advice: "Do not delete them by hand. Run the command to keep fewer old versions.",
  },
  "lin.flatpak": {
    title: "Flatpak packages",
    what: "Flatpak apps and runtimes.",
    advice: "Do not delete them by hand. The command removes runtimes nothing uses.",
  },
  "lin.docker": {
    title: "Docker data",
    what: "Docker images, containers, volumes and build cache.",
    advice: "Do not delete it by hand, or Docker breaks. The command removes unused data.",
  },
  "lin.tmp": {
    title: "Temporary files",
    what: "Temporary files; /tmp is usually emptied on reboot.",
    advice: "Old files can be deleted; better leave those of running programs alone.",
  },
  "lin.usercache": {
    title: "User cache",
    what: "Program caches: thumbnails, browsers, package managers.",
    advice: "Safe to delete; programs build their cache again. Best with the programs closed.",
  },
  "lin.trash": {
    title: "Trash",
    what: "Files deleted to the Trash.",
    advice: "Empty the Trash in the file manager or with the command.",
  },
  "mac.system": {
    title: "System files",
    what: "Files of macOS and shared components.",
    advice: "Do not delete them: macOS protects them itself, and deleting them can break the system.",
  },
  "mac.vm": {
    title: "Swap and sleep",
    what: "Swap files and the memory image for sleep (sleepimage).",
    advice: "Do not delete them: macOS manages them itself.",
  },
  "mac.usercache": {
    title: "Caches",
    what: "Program caches.",
    advice: "Safe to delete; programs build their cache again. Best with the programs closed.",
  },
  "mac.trash": {
    title: "Trash",
    what: "Files deleted to the Trash.",
    advice: "Empty the Trash in Finder.",
  },
  "mac.derived": {
    title: "Xcode builds",
    what: "Xcode's intermediate build files (DerivedData).",
    advice: "Safe to delete: Xcode builds the projects again.",
  },
  "mac.simulators": {
    title: "Xcode simulators",
    what: "iOS and other simulators and their data.",
    advice: "Do not delete them by hand. The command removes unavailable simulators.",
  },
};
