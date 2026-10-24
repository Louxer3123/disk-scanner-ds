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
mod format;
pub mod layout;
mod platform;
mod tree;
mod walk;

pub use drives::{DriveInfo, DriveKind, list_drives};
pub use format::format_size;
pub use layout::{Hit, LayoutParams, Segment, SunburstLayout};
pub use platform::display_path;
pub use tree::{Children, NodeFlags, NodeId, NodeKind, Tree};
pub use walk::{CancelToken, Progress, Scan, ScanStats, scan};
