//! File types: a category for every file by its extension, the dominant category
//! of every folder, and space per extension.

use std::collections::HashMap;

use crate::tree::{NodeId, NodeKind, Tree};

/// Broad kind of a file, by extension. The numbers are stable: the UI keys its
/// colours and names by them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum Category {
    Other = 0,
    Video = 1,
    Audio = 2,
    Image = 3,
    Document = 4,
    Archive = 5,
    Program = 6,
    Code = 7,
    DiskImage = 8,
    Data = 9,
}

impl Category {
    pub const COUNT: usize = 10;
    pub const ALL: [Category; Self::COUNT] = [
        Category::Other,
        Category::Video,
        Category::Audio,
        Category::Image,
        Category::Document,
        Category::Archive,
        Category::Program,
        Category::Code,
        Category::DiskImage,
        Category::Data,
    ];

    /// Category of a lower-case extension without the dot.
    pub fn of_extension(ext: &str) -> Category {
        match ext {
            "mp4" | "mkv" | "avi" | "mov" | "wmv" | "webm" | "m4v" | "mpg" | "mpeg" | "m2ts" | "mts" | "ts" | "vob"
            | "flv" | "3gp" | "ogv" | "rmvb" => Category::Video,
            "mp3" | "flac" | "wav" | "aac" | "m4a" | "ogg" | "opus" | "wma" | "aiff" | "ape" | "alac" | "mid"
            | "midi" => Category::Audio,
            "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "tif" | "tiff" | "heic" | "heif" | "avif" | "svg"
            | "ico" | "psd" | "raw" | "cr2" | "cr3" | "nef" | "arw" | "dng" | "orf" | "rw2" | "xcf" => Category::Image,
            "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "rtf" | "txt" | "md"
            | "epub" | "fb2" | "djvu" | "mobi" | "chm" => Category::Document,
            "zip" | "rar" | "7z" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "tar" | "cab" | "lz" | "lzma" | "z"
            | "pak" => Category::Archive,
            "exe" | "dll" | "sys" | "msi" | "msp" | "msu" | "drv" | "ocx" | "cpl" | "scr" | "appx" | "msix"
            | "appxbundle" | "msixbundle" | "so" | "dylib" | "apk" | "jar" | "mui" | "efi" | "node" | "pdb" => {
                Category::Program
            }
            "rs" | "c" | "h" | "cpp" | "cc" | "hpp" | "cs" | "java" | "kt" | "go" | "py" | "js" | "mjs" | "cjs"
            | "tsx" | "jsx" | "html" | "htm" | "css" | "scss" | "php" | "rb" | "swift" | "lua" | "sh"
            | "ps1" | "bat" | "cmd" | "pas" | "dpr" | "vue" | "sql" | "toml" | "ini" | "cfg" | "lock" | "map" => {
                Category::Code
            }
            "iso" | "img" | "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow2" | "wim" | "esd" | "swm" | "dmg" | "avhdx" => {
                Category::DiskImage
            }
            "db" | "sqlite" | "sqlite3" | "mdb" | "accdb" | "mdf" | "ldf" | "ndf" | "pst" | "ost" | "edb" | "csv"
            | "json" | "xml" | "yaml" | "yml" | "parquet" | "log" | "etl" | "evtx" | "bak" | "dat" | "bin" => {
                Category::Data
            }
            _ => Category::Other,
        }
    }

    /// Category of a file name.
    pub fn of_name(name: &str) -> Category {
        let mut buf = ExtBuf::default();
        buf.extension(name).map_or(Category::Other, Category::of_extension)
    }
}

/// The extension of `name` without the dot, as written: `None` for no dot, a
/// leading dot only (`.gitignore`) or a trailing dot.
pub fn extension(name: &str) -> Option<&str> {
    let dot = name.rfind('.')?;
    let ext = &name[dot + 1..];
    (dot > 0 && !ext.is_empty()).then_some(ext)
}

/// Longer "extensions" are parts of a name (`report.final-version-2`), not types.
const MAX_EXTENSION: usize = 16;

/// Reusable buffer for lower-casing extensions without an allocation per file.
#[derive(Default)]
struct ExtBuf(String);

impl ExtBuf {
    fn extension(&mut self, name: &str) -> Option<&str> {
        let ext = extension(name).filter(|e| e.len() <= MAX_EXTENSION && !e.contains(' '))?;
        self.0.clear();
        if ext.is_ascii() {
            self.0.extend(ext.chars().map(|c| c.to_ascii_lowercase()));
        } else {
            self.0.extend(ext.chars().flat_map(char::to_lowercase));
        }
        Some(&self.0)
    }
}

/// For every node of `tree`, indexed like the tree: a file's category, or for a
/// folder the category taking the most space below it. Links, free space and
/// folders with no files get [`Category::Other`].
pub fn dominant_categories(tree: &Tree) -> Vec<Category> {
    let mut out = vec![Category::Other; tree.node_count()];
    let mut buf = ExtBuf::default();
    // Post-order walk: each open folder keeps its space per category.
    let mut stack = vec![(NodeId::ROOT, tree.children(NodeId::ROOT), [0u64; Category::COUNT])];
    while let Some((_, children, _)) = stack.last_mut() {
        match children.next() {
            Some(child) => match tree.kind(child) {
                NodeKind::Dir => stack.push((child, tree.children(child), [0; Category::COUNT])),
                NodeKind::File => {
                    let cat = buf.extension(tree.name(child)).map_or(Category::Other, Category::of_extension);
                    out[child.index()] = cat;
                    let sums = &mut stack.last_mut().expect("just used").2;
                    sums[cat as usize] = sums[cat as usize].saturating_add(tree.allocated(child));
                }
                _ => {}
            },
            None => {
                let (dir, _, sums) = stack.pop().expect("just used");
                out[dir.index()] = dominant(&sums);
                if let Some(parent) = stack.last_mut() {
                    for (total, s) in parent.2.iter_mut().zip(sums) {
                        *total = total.saturating_add(s);
                    }
                }
            }
        }
    }
    out
}

/// The known category with the most space, unless files of unknown types take
/// more than twice as much (a game folder with a few videos stays "other").
fn dominant(sums: &[u64; Category::COUNT]) -> Category {
    let (best, size) = Category::ALL[1..].iter().map(|&c| (c, sums[c as usize])).max_by_key(|&(_, s)| s).expect("non-empty");
    if size > 0 && size >= sums[Category::Other as usize] / 2 { best } else { Category::Other }
}

/// Space taken by files of one extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionStat {
    /// Lower-case, without the dot; empty for files without an extension.
    pub extension: String,
    pub category: Category,
    pub files: u64,
    pub allocated: u64,
}

/// Space per extension and per category, gathered over one or more subtrees.
#[derive(Default)]
pub struct TypeStats {
    by_ext: HashMap<String, (u64, u64)>,
    buf: ExtBuf,
}

impl TypeStats {
    /// Adds every file below `root` (or `root` itself, if it is a file).
    pub fn add(&mut self, tree: &Tree, root: NodeId) {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            match tree.kind(id) {
                NodeKind::Dir => stack.extend(tree.children(id)),
                NodeKind::File => {
                    let ext = self.buf.extension(tree.name(id)).unwrap_or("");
                    let size = tree.allocated(id);
                    if let Some(e) = self.by_ext.get_mut(ext) {
                        e.0 += 1;
                        e.1 = e.1.saturating_add(size);
                    } else {
                        self.by_ext.insert(ext.to_owned(), (1, size));
                    }
                }
                _ => {}
            }
        }
    }

    /// Extensions, largest first.
    pub fn extensions(&self) -> Vec<ExtensionStat> {
        let mut v: Vec<ExtensionStat> = self
            .by_ext
            .iter()
            .map(|(ext, &(files, allocated))| ExtensionStat {
                extension: ext.clone(),
                category: Category::of_extension(ext),
                files,
                allocated,
            })
            .collect();
        v.sort_by(|a, b| b.allocated.cmp(&a.allocated).then_with(|| a.extension.cmp(&b.extension)));
        v
    }

    /// Files and space per category, indexed by `Category as usize`.
    pub fn categories(&self) -> [(u64, u64); Category::COUNT] {
        let mut out = [(0u64, 0u64); Category::COUNT];
        for (ext, &(files, allocated)) in &self.by_ext {
            let c = &mut out[Category::of_extension(ext) as usize];
            c.0 += files;
            c.1 = c.1.saturating_add(allocated);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_and_categories() {
        assert_eq!(extension("movie.MKV"), Some("MKV"));
        assert_eq!(extension("archive.tar.gz"), Some("gz"));
        assert_eq!(extension(".gitignore"), None);
        assert_eq!(extension("README"), None);
        assert_eq!(extension("name."), None);
        assert_eq!(Category::of_name("Отпуск.MP4"), Category::Video);
        assert_eq!(Category::of_name("pagefile.sys"), Category::Program);
        assert_eq!(Category::of_name("backup.7z"), Category::Archive);
        assert_eq!(Category::of_name("notes"), Category::Other);
        assert_eq!(Category::of_name("Windows10.ISO"), Category::DiskImage);
        assert_eq!(Category::of_name("report.final version"), Category::Other, "not an extension");
        for (i, c) in Category::ALL.iter().enumerate() {
            assert_eq!(*c as usize, i, "ALL is in numeric order");
        }
    }

    fn sample() -> Tree {
        let mut t = Tree::new("D:\\");
        let videos = t.add_dir(NodeId::ROOT, "Videos");
        t.add_file(videos, "a.mp4", 0, 9000);
        t.add_file(videos, "a.srt", 0, 100);
        t.add_file(videos, "cover.JPG", 0, 500);
        let mixed = t.add_dir(NodeId::ROOT, "Mixed");
        let inner = t.add_dir(mixed, "inner");
        t.add_file(inner, "big.zip", 0, 5000);
        t.add_file(mixed, "song.mp3", 0, 3000);
        t.add_file(mixed, "README", 0, 10);
        t.add_dir(NodeId::ROOT, "Empty");
        t.add_file(NodeId::ROOT, "photo.jpg", 0, 700);
        t
    }

    #[test]
    fn folders_take_the_category_with_the_most_space() {
        let t = sample();
        let cats = dominant_categories(&t);
        let at = |path: &[&str]| cats[t.find_path(path).0.index()];
        assert_eq!(at(&["Videos"]), Category::Video);
        assert_eq!(at(&["Videos", "cover.JPG"]), Category::Image);
        assert_eq!(at(&["Mixed", "inner"]), Category::Archive);
        assert_eq!(at(&["Mixed"]), Category::Archive, "zip below beats the mp3");
        assert_eq!(at(&["Empty"]), Category::Other);
        assert_eq!(cats[NodeId::ROOT.index()], Category::Video);
    }

    #[test]
    fn mostly_unknown_files_make_an_other_folder() {
        let mut t = Tree::new("D:\\");
        let d = t.add_dir(NodeId::ROOT, "game");
        t.add_file(d, "level1.xyz", 0, 10_000);
        t.add_file(d, "intro.mp4", 0, 1_000);
        assert_eq!(dominant_categories(&t)[d.index()], Category::Other);
    }

    #[test]
    fn stats_per_extension_and_category() {
        let t = sample();
        let mut s = TypeStats::default();
        s.add(&t, NodeId::ROOT);
        let exts = s.extensions();
        assert_eq!(exts[0], ExtensionStat { extension: "mp4".into(), category: Category::Video, files: 1, allocated: 9000 });
        let jpg = exts.iter().find(|e| e.extension == "jpg").unwrap();
        assert_eq!((jpg.files, jpg.allocated), (2, 1200), "case is folded");
        let none = exts.iter().find(|e| e.extension.is_empty()).unwrap();
        assert_eq!((none.files, none.category), (1, Category::Other));
        let cats = s.categories();
        assert_eq!(cats[Category::Image as usize], (2, 1200));
        assert_eq!(cats.iter().map(|c| c.1).sum::<u64>(), t.allocated(NodeId::ROOT));

        // Only one folder.
        let mut s = TypeStats::default();
        s.add(&t, t.find_path(&["Mixed"]).0);
        assert_eq!(s.extensions().len(), 3);
    }
}
