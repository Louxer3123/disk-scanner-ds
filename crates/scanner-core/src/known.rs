//! Well-known system files and folders: page and hibernation files, update
//! caches, temp folders, package caches, system folders. For each, whether it
//! may be deleted by hand, freed only through a system tool, or not touched.
//!
//! The UI holds the texts (what it is, how to free the space) under each rule's
//! id; this module only says where the places are and how safe they are.

use crate::tree::{NodeId, NodeKind, Tree};

/// What may be done with a known place.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Safety {
    /// Its contents can be deleted by hand: temp files, caches, crash dumps.
    Safe,
    /// The space can be freed, but through a system tool or setting, not by
    /// deleting files (page file, WinSxS, Windows.old, package caches).
    Tool,
    /// The system or programs need it; deleting it breaks them.
    Keep,
}

impl Safety {
    pub fn as_str(self) -> &'static str {
        match self {
            Safety::Safe => "safe",
            Safety::Tool => "tool",
            Safety::Keep => "keep",
        }
    }
}

/// One known place: `pattern` is a path below the file system root with `/`
/// between names, where `*` stands for any one name (a user's folder).
#[derive(Clone, Copy, Debug)]
pub struct Rule {
    pub id: &'static str,
    pub pattern: &'static str,
    pub safety: Safety,
}

/// The rules of one system and how it compares names.
pub struct Catalogue {
    pub rules: &'static [Rule],
    pub case_insensitive: bool,
}

const fn rule(id: &'static str, pattern: &'static str, safety: Safety) -> Rule {
    Rule { id, pattern, safety }
}

use Safety::{Keep, Safe, Tool};

/// Windows, relative to a drive root (`C:\`).
pub static WINDOWS: Catalogue = Catalogue {
    case_insensitive: true,
    rules: &[
        rule("win.pagefile", "pagefile.sys", Tool),
        rule("win.hiberfil", "hiberfil.sys", Tool),
        rule("win.swapfile", "swapfile.sys", Keep),
        rule("win.dumpstack", "DumpStack.log.tmp", Keep),
        rule("win.recycle", "$Recycle.Bin", Tool),
        rule("win.sysvolinfo", "System Volume Information", Tool),
        rule("win.recovery", "Recovery", Keep),
        rule("win.windows", "Windows", Keep),
        rule("win.winsxs", "Windows/WinSxS", Tool),
        rule("win.update", "Windows/SoftwareDistribution/Download", Tool),
        rule("win.installer", "Windows/Installer", Keep),
        rule("win.driverstore", "Windows/System32/DriverStore", Keep),
        rule("win.temp", "Windows/Temp", Safe),
        rule("win.dumps", "Windows/MEMORY.DMP", Safe),
        rule("win.dumps", "Windows/Minidump", Safe),
        rule("win.dumps", "Windows/LiveKernelReports", Safe),
        rule("win.logs", "Windows/Logs", Safe),
        rule("win.old", "Windows.old", Tool),
        rule("win.upgrade", "$WINDOWS.~BT", Tool),
        rule("win.upgrade", "$Windows.~WS", Tool),
        rule("win.programfiles", "Program Files", Keep),
        rule("win.programfiles", "Program Files (x86)", Keep),
        rule("win.programdata", "ProgramData", Keep),
        rule("win.pkgcache", "ProgramData/Package Cache", Keep),
        rule("win.wer", "ProgramData/Microsoft/Windows/WER", Safe),
        rule("win.appdata", "Users/*/AppData", Keep),
        rule("win.temp", "Users/*/AppData/Local/Temp", Safe),
        rule("win.dumps", "Users/*/AppData/Local/CrashDumps", Safe),
        rule("win.inetcache", "Users/*/AppData/Local/Microsoft/Windows/INetCache", Safe),
    ],
};

/// Linux, relative to `/`.
pub static LINUX: Catalogue = Catalogue {
    case_insensitive: false,
    rules: &[
        rule("lin.swapfile", "swapfile", Tool),
        rule("lin.swapfile", "swap.img", Tool),
        rule("lin.system", "usr", Keep),
        rule("lin.system", "etc", Keep),
        rule("lin.system", "bin", Keep),
        rule("lin.system", "sbin", Keep),
        rule("lin.system", "lib", Keep),
        rule("lin.system", "lib64", Keep),
        rule("lin.system", "var", Keep),
        rule("lin.boot", "boot", Keep),
        rule("lin.lostfound", "lost+found", Keep),
        rule("lin.journal", "var/log/journal", Tool),
        rule("lin.apt", "var/cache/apt", Tool),
        rule("lin.dnf", "var/cache/dnf", Tool),
        rule("lin.pacman", "var/cache/pacman/pkg", Tool),
        rule("lin.snap", "var/lib/snapd", Tool),
        rule("lin.flatpak", "var/lib/flatpak", Tool),
        rule("lin.docker", "var/lib/docker", Tool),
        rule("lin.tmp", "tmp", Safe),
        rule("lin.tmp", "var/tmp", Safe),
        rule("lin.usercache", "home/*/.cache", Safe),
        rule("lin.usercache", "root/.cache", Safe),
        rule("lin.trash", "home/*/.local/share/Trash", Tool),
    ],
};

/// macOS, relative to `/` (paths on the Data volume are taken as from `/`).
pub static MACOS: Catalogue = Catalogue {
    case_insensitive: true,
    rules: &[
        rule("mac.system", "System", Keep),
        rule("mac.system", "Library", Keep),
        rule("mac.system", "usr", Keep),
        rule("mac.vm", "private/var/vm", Keep),
        rule("mac.usercache", "Library/Caches", Safe),
        rule("mac.usercache", "Users/*/Library/Caches", Safe),
        rule("mac.trash", "Users/*/.Trash", Tool),
        rule("mac.derived", "Users/*/Library/Developer/Xcode/DerivedData", Safe),
        rule("mac.simulators", "Users/*/Library/Developer/CoreSimulator", Tool),
    ],
};

/// The catalogue of the system the program runs on.
pub fn catalogue() -> &'static Catalogue {
    if cfg!(windows) {
        &WINDOWS
    } else if cfg!(target_os = "macos") {
        &MACOS
    } else {
        &LINUX
    }
}

/// A place found: its rule, and whether the path is inside it rather than it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Known {
    pub id: &'static str,
    pub safety: Safety,
    /// The path lies below the place the rule names.
    pub inside: bool,
}

/// Names of `path` below the file system root: after the drive on Windows
/// (`C:\Windows\Temp` → `Windows`, `Temp`), after `/` elsewhere, with macOS's
/// `/System/Volumes/Data` taken as `/`.
pub fn components(path: &str) -> Vec<&str> {
    let rest = if cfg!(windows) {
        let rest = path.strip_prefix(r"\\?\").unwrap_or(path);
        match rest.as_bytes() {
            [_, b':', ..] => &rest[2..],
            _ => rest,
        }
    } else if cfg!(target_os = "macos") {
        path.strip_prefix("/System/Volumes/Data").filter(|r| r.is_empty() || r.starts_with('/')).unwrap_or(path)
    } else {
        path
    };
    rest.split(['/', std::path::MAIN_SEPARATOR]).filter(|c| !c.is_empty()).collect()
}

impl Catalogue {
    fn same(&self, pattern: &str, name: &str) -> bool {
        pattern == "*" || pattern == name || self.case_insensitive && pattern.eq_ignore_ascii_case(name)
    }

    /// The most specific rule for `path` (names from [`components`]): the
    /// place itself, or the nearest one it lies in.
    pub fn identify(&self, path: &[&str]) -> Option<Known> {
        self.rules
            .iter()
            .filter_map(|r| {
                let pattern: Vec<&str> = r.pattern.split('/').collect();
                let matches = pattern.len() <= path.len() && pattern.iter().zip(path).all(|(p, n)| self.same(p, n));
                matches.then_some((pattern.len(), r))
            })
            .max_by_key(|&(len, _)| len)
            .map(|(len, r)| Known { id: r.id, safety: r.safety, inside: len < path.len() })
    }

    /// Every known place present in `tree`, whose root is at `root` (names
    /// from [`components`]: empty for a whole drive, `["home"]` for `/home`).
    pub fn find_all(&self, tree: &Tree, root: &[&str]) -> Vec<(Rule, NodeId)> {
        let mut found = Vec::new();
        for r in self.rules {
            let pattern: Vec<&str> = r.pattern.split('/').collect();
            // The part of the pattern above the tree must be the tree's root.
            if pattern.len() <= root.len() || !pattern.iter().zip(root).all(|(p, n)| self.same(p, n)) {
                continue;
            }
            let mut level = vec![NodeId::ROOT];
            for name in &pattern[root.len()..] {
                level = level
                    .iter()
                    .flat_map(|&n| tree.children(n))
                    .filter(|&c| !matches!(tree.kind(c), NodeKind::FreeSpace | NodeKind::Unscanned) && self.same(name, tree.name(c)))
                    .collect();
            }
            found.extend(level.into_iter().map(|n| (*r, n)));
        }
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(c: &Catalogue, path: &str) -> Option<(&'static str, Safety, bool)> {
        let names: Vec<&str> = path.split('/').filter(|n| !n.is_empty()).collect();
        c.identify(&names).map(|k| (k.id, k.safety, k.inside))
    }

    #[test]
    fn windows_places() {
        assert_eq!(id(&WINDOWS, "pagefile.sys"), Some(("win.pagefile", Tool, false)));
        assert_eq!(id(&WINDOWS, "HIBERFIL.SYS"), Some(("win.hiberfil", Tool, false)), "names are case-insensitive");
        assert_eq!(id(&WINDOWS, "Windows"), Some(("win.windows", Keep, false)));
        assert_eq!(id(&WINDOWS, "Windows/System32/kernel32.dll"), Some(("win.windows", Keep, true)));
        assert_eq!(id(&WINDOWS, "Windows/WinSxS/amd64_x/f.dll"), Some(("win.winsxs", Tool, true)));
        // A safe folder inside a protected one wins: it is the closer rule.
        assert_eq!(id(&WINDOWS, "Windows/Temp/old.log"), Some(("win.temp", Safe, true)));
        assert_eq!(id(&WINDOWS, "Users/me/AppData/Local/Temp"), Some(("win.temp", Safe, false)));
        assert_eq!(id(&WINDOWS, "Users/me/AppData/Roaming/app"), Some(("win.appdata", Keep, true)));
        assert_eq!(id(&WINDOWS, "Users/me/Documents"), None);
        assert_eq!(id(&WINDOWS, "Games/pagefile.sys"), None, "only at the drive root");
    }

    #[test]
    fn linux_and_macos_places() {
        assert_eq!(id(&LINUX, "var/log/journal"), Some(("lin.journal", Tool, false)));
        assert_eq!(id(&LINUX, "var/log/syslog"), Some(("lin.system", Keep, true)));
        assert_eq!(id(&LINUX, "home/me/.cache/thumbnails"), Some(("lin.usercache", Safe, true)));
        assert_eq!(id(&LINUX, "home/me/.Cache"), None, "Linux names are case-sensitive");
        assert_eq!(id(&MACOS, "Users/me/Library/Caches/com.app"), Some(("mac.usercache", Safe, true)));
        assert_eq!(id(&MACOS, "private/var/vm/sleepimage"), Some(("mac.vm", Keep, true)));
    }

    #[test]
    fn components_of_paths() {
        if cfg!(windows) {
            assert_eq!(components(r"C:\Windows\Temp"), ["Windows", "Temp"]);
            assert_eq!(components(r"\\?\D:\pagefile.sys"), ["pagefile.sys"]);
            assert!(components(r"C:\").is_empty());
        } else {
            assert_eq!(components("/var/log"), ["var", "log"]);
            assert!(components("/").is_empty());
        }
    }

    #[test]
    fn finds_places_in_a_tree() {
        let mut t = Tree::new("/home");
        let me = t.add_dir(NodeId::ROOT, "me");
        let cache = t.add_dir(me, ".cache");
        t.add_file(cache, "x", 10, 4096);
        let you = t.add_dir(NodeId::ROOT, "you");
        let local = t.add_dir(you, ".local");
        let share = t.add_dir(local, "share");
        let trash = t.add_dir(share, "Trash");
        t.add_dir(NodeId::ROOT, "docs");

        let mut found: Vec<(&str, NodeId)> = LINUX.find_all(&t, &["home"]).into_iter().map(|(r, n)| (r.id, n)).collect();
        found.sort();
        assert_eq!(found, [("lin.trash", trash), ("lin.usercache", cache)]);
        // From the whole file system the same places are under "home".
        assert!(LINUX.find_all(&t, &["srv"]).is_empty());

        let mut w = Tree::new("C:\\");
        let win = w.add_dir(NodeId::ROOT, "WINDOWS");
        let temp = w.add_dir(win, "Temp");
        let page = w.add_file(NodeId::ROOT, "pagefile.sys", 1, 1 << 30);
        let mut found: Vec<(&str, NodeId)> = WINDOWS.find_all(&w, &[]).into_iter().map(|(r, n)| (r.id, n)).collect();
        found.sort();
        assert_eq!(found, [("win.pagefile", page), ("win.temp", temp), ("win.windows", win)]);
    }
}
