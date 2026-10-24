//! Core of the disk usage scanner: walks a directory and builds a compact tree of
//! the space taken by every file and folder below it.
//!
//! ```no_run
//! use scanner_core::{CancelToken, NodeId, format_size, scan};
//!
//! let result = scan("C:\\".as_ref(), &CancelToken::new(), |_| {}).unwrap();
//! let tree = &result.tree;
//! for child in tree.children_by_size(NodeId::ROOT).into_iter().take(10) {
//!     println!("{:>10}  {}", format_size(tree.allocated(child)), tree.name(child));
//! }
//! ```

mod drives;
pub mod filetype;
mod format;
pub mod layout;
mod mft;
mod platform;
pub mod search;
mod tree;
mod walk;

pub use drives::{DriveInfo, DriveKind, drive_info, list_drives, present_roots};
pub use format::format_size;
pub use mft::{is_elevated, scan_mft};
pub use layout::{Hit, LayoutParams, Segment, SunburstLayout};
pub use platform::{display_path, is_plain_win32_name};
pub use tree::{Children, NodeFlags, NodeId, NodeKind, Tree};
pub use walk::{CancelToken, Progress, Scan, ScanOptions, ScanStats, default_threads, scan, scan_with};
