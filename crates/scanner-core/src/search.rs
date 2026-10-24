//! Finding nodes by name: a substring, or a mask with `*` and `?`.

use crate::tree::{NodeId, NodeKind, Tree};

/// No match in the subtree.
pub const MARK_NONE: u8 = 0;
/// Something below the node matches.
pub const MARK_CONTAINS: u8 = 1;
/// The node itself matches.
pub const MARK_MATCH: u8 = 2;

/// A parsed search query. Matching ignores case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    /// Plain text: the name contains it anywhere.
    Substring(String),
    /// A mask with `*` (any run of characters) and `?` (one character) that must
    /// cover the whole name, e.g. `*.mp4` or `IMG_????.jpg`. Kept lower-case, as
    /// characters, and as bytes when ASCII for the allocation-free fast path.
    Mask { chars: Vec<char>, ascii: Option<Vec<u8>> },
}

impl Query {
    /// `None` for an empty or blank query.
    pub fn parse(text: &str) -> Option<Query> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let lower = text.to_lowercase();
        Some(if lower.contains(['*', '?']) {
            Query::Mask { chars: lower.chars().collect(), ascii: lower.is_ascii().then(|| lower.clone().into_bytes()) }
        } else {
            Query::Substring(lower)
        })
    }

    pub fn matches(&self, name: &str) -> bool {
        match self {
            Query::Substring(needle) => {
                if name.is_ascii() && needle.is_ascii() {
                    // Fast path without allocating, for the vast majority of names.
                    let n = needle.as_bytes();
                    n.len() <= name.len() && name.as_bytes().windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
                } else {
                    name.to_lowercase().contains(needle.as_str())
                }
            }
            Query::Mask { ascii: Some(mask), .. } if name.is_ascii() => {
                wildcard(mask, name.as_bytes(), b'*', b'?', |m, n| m == n.to_ascii_lowercase())
            }
            Query::Mask { chars, .. } => {
                let name: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
                wildcard(chars, &name, '*', '?', |m, n| m == n)
            }
        }
    }
}

/// Wildcard matching with backtracking to the last `*`: linear in practice.
/// `eq(mask item, name item)` compares one position.
fn wildcard<T: Copy + PartialEq>(mask: &[T], name: &[T], star_item: T, any: T, eq: impl Fn(T, T) -> bool) -> bool {
    let (mut m, mut n) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if m < mask.len() && mask[m] != star_item && (mask[m] == any || eq(mask[m], name[n])) {
            m += 1;
            n += 1;
        } else if m < mask.len() && mask[m] == star_item {
            star = Some((m, n));
            m += 1;
        } else if let Some((sm, sn)) = star {
            // Let the last `*` swallow one more character and retry.
            m = sm + 1;
            n = sn + 1;
            star = Some((sm, sn + 1));
        } else {
            return false;
        }
    }
    mask[m..].iter().all(|&c| c == star_item)
}

/// Marks every node of `tree`: [`MARK_MATCH`] if its name matches, else
/// [`MARK_CONTAINS`] if something below it does, else [`MARK_NONE`]. Indexed by
/// [`NodeId::index`]. Space blocks never match.
pub fn mark_matches(tree: &Tree, query: &Query) -> Vec<u8> {
    let n = tree.node_count();
    let mut marks = vec![MARK_NONE; n];
    for i in 1..n {
        let id = tree.node_id(i).expect("in range");
        if !matches!(tree.kind(id), NodeKind::Dir | NodeKind::File | NodeKind::Link) || !query.matches(tree.name(id)) {
            continue;
        }
        marks[i] = MARK_MATCH;
        let mut cur = tree.parent(id);
        while let Some(p) = cur {
            // Already marked: its ancestors are, or will be when it is processed.
            if marks[p.index()] != MARK_NONE {
                break;
            }
            marks[p.index()] = MARK_CONTAINS;
            cur = tree.parent(p);
        }
    }
    marks
}

/// Matches found under `root` (not counting `root` itself).
#[derive(Debug, Default)]
pub struct Found {
    /// Every matching node.
    pub nodes: Vec<NodeId>,
    /// Space taken by the matches, counting a match inside a matching folder once.
    pub allocated: u64,
}

/// Collects the matches below `root`, visiting only branches that contain some.
pub fn find_marked(tree: &Tree, root: NodeId, marks: &[u8]) -> Found {
    let mut found = Found::default();
    // (node, whether an ancestor below `root` already matched)
    let mut stack: Vec<(NodeId, bool)> = tree.children(root).map(|c| (c, false)).collect();
    while let Some((id, inside_match)) = stack.pop() {
        let mark = marks.get(id.index()).copied().unwrap_or(MARK_NONE);
        if mark == MARK_NONE {
            continue;
        }
        let is_match = mark == MARK_MATCH;
        if is_match {
            found.nodes.push(id);
            if !inside_match {
                found.allocated += tree.allocated(id);
            }
        }
        stack.extend(tree.children(id).map(|c| (c, inside_match || is_match)));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> Query {
        Query::parse(s).unwrap()
    }

    #[test]
    fn parses_text_and_masks() {
        assert_eq!(Query::parse("  "), None);
        assert_eq!(q(" Foto "), Query::Substring("foto".into()));
        assert!(matches!(q("*.MP4"), Query::Mask { ascii: Some(_), .. }));
        assert!(matches!(q("*.фото"), Query::Mask { ascii: None, .. }));
    }

    #[test]
    fn ascii_and_unicode_mask_paths_agree() {
        for (mask, name) in [("*.jpg", "A.JPG"), ("*.jpg", "Фото.JPG"), ("a?c", "ABC"), ("a?c", "AБC"), ("*x*", "yyy")] {
            let fast = q(mask).matches(name);
            let slow = match q(mask) {
                Query::Mask { chars, .. } => {
                    let lowered: Vec<char> = name.chars().flat_map(char::to_lowercase).collect();
                    wildcard(&chars, &lowered, '*', '?', |m, n| m == n)
                }
                Query::Substring(_) => unreachable!(),
            };
            assert_eq!(fast, slow, "{mask} vs {name}");
        }
    }

    #[test]
    fn substring_ignores_case_in_any_script() {
        assert!(q("report").matches("Q3 Report.xlsx"));
        assert!(q("ОТЧЁТ").matches("годовой отчёт.docx"));
        assert!(q("ä").matches("BÄR"));
        assert!(!q("report").matches("rep"));
        assert!(!q("xyz").matches(""));
    }

    #[test]
    fn masks_cover_the_whole_name() {
        assert!(q("*.mp4").matches("Holiday.MP4"));
        assert!(!q("*.mp4").matches("holiday.mp4.part"));
        assert!(q("img_????.jpg").matches("IMG_0042.JPG"));
        assert!(!q("img_????.jpg").matches("IMG_42.JPG"));
        assert!(q("*").matches(""));
        assert!(q("a*b*c").matches("aXXbYYc"));
        assert!(!q("a*b*c").matches("aXXbYY"));
        assert!(q("*видео*").matches("Моё ВИДЕО 2024"));
        assert!(q("**x").matches("x"));
    }

    /// root/
    ///   photos/ (a.jpg 10, b.png 20, raw/ (c.jpg 5))
    ///   docs/   (cv.pdf 7)
    ///   d.jpg   3
    fn sample() -> Tree {
        let mut t = Tree::new("C:\\");
        let photos = t.add_dir(NodeId::ROOT, "photos");
        t.add_file(photos, "a.jpg", 10, 10);
        t.add_file(photos, "b.png", 20, 20);
        let raw = t.add_dir(photos, "raw");
        t.add_file(raw, "c.jpg", 5, 5);
        let docs = t.add_dir(NodeId::ROOT, "docs");
        t.add_file(docs, "cv.pdf", 7, 7);
        t.add_file(NodeId::ROOT, "d.jpg", 3, 3);
        t.add_space(NodeId::ROOT, "jpg free space", NodeKind::FreeSpace, 100);
        t
    }

    fn names(t: &Tree, ids: &[NodeId]) -> Vec<String> {
        let mut v: Vec<String> = ids.iter().map(|&i| t.name(i).to_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn marks_matches_and_their_ancestors() {
        let t = sample();
        let marks = mark_matches(&t, &q("*.jpg"));
        let mark = |path: &[&str]| marks[t.find_path(path).0.index()];
        assert_eq!(mark(&[]), MARK_CONTAINS);
        assert_eq!(mark(&["photos"]), MARK_CONTAINS);
        assert_eq!(mark(&["photos", "raw"]), MARK_CONTAINS);
        assert_eq!(mark(&["photos", "a.jpg"]), MARK_MATCH);
        assert_eq!(mark(&["photos", "b.png"]), MARK_NONE);
        assert_eq!(mark(&["docs"]), MARK_NONE);
        assert_eq!(mark(&["jpg free space"]), MARK_NONE, "space blocks never match");
    }

    #[test]
    fn finds_matches_under_a_root_without_double_counting() {
        let t = sample();
        let marks = mark_matches(&t, &q("*.jpg"));
        let all = find_marked(&t, NodeId::ROOT, &marks);
        assert_eq!(names(&t, &all.nodes), ["a.jpg", "c.jpg", "d.jpg"]);
        assert_eq!(all.allocated, 18);

        let photos = t.find_path(&["photos"]).0;
        assert_eq!(names(&t, &find_marked(&t, photos, &marks).nodes), ["a.jpg", "c.jpg"]);

        let marks = mark_matches(&t, &q("r"));
        let found = find_marked(&t, NodeId::ROOT, &marks);
        assert_eq!(names(&t, &found.nodes), ["raw"]);
        assert_eq!(found.allocated, 5);
    }

    #[test]
    fn a_match_inside_a_matching_folder_is_counted_once() {
        let mut t = Tree::new("C:\\");
        let music = t.add_dir(NodeId::ROOT, "Music");
        t.add_file(music, "music.mp3", 10, 10);
        t.add_file(music, "cover.jpg", 2, 2);
        let found = find_marked(&t, NodeId::ROOT, &mark_matches(&t, &q("music")));
        assert_eq!(names(&t, &found.nodes), ["Music", "music.mp3"]);
        assert_eq!(found.allocated, 12);
    }
}
