//! Parsing NTFS file records (the entries of the Master File Table).
//!
//! Only what a disk usage scan needs is read: names with their parent folder,
//! sizes of the data streams, the directory flag and the reparse tag. All reads
//! are bounds-checked, so a damaged record yields `None` instead of a panic.

/// `$FILE_NAME` namespace of a short 8.3 alias that duplicates a long name.
pub const NAMESPACE_DOS: u8 = 2;
/// Reparse tags with this bit (symlinks, junctions, mount points) point
/// elsewhere; others (e.g. OneDrive placeholders) are ordinary files or folders.
pub const TAG_NAME_SURROGATE: u32 = 0x2000_0000;

const ATTR_FILE_NAME: u32 = 0x30;
const ATTR_DATA: u32 = 0x80;
const ATTR_REPARSE_POINT: u32 = 0xC0;
const ATTR_END: u32 = 0xFFFF_FFFF;
const RECORD_IN_USE: u16 = 0x0001;
const RECORD_IS_DIR: u16 = 0x0002;
/// Low 48 bits of a file reference are the record number, the top 16 the sequence.
const REF_MASK: u64 = 0x0000_FFFF_FFFF_FFFF;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileName {
    /// Record number of the folder holding this name.
    pub parent: u64,
    /// Sequence number the parent record must have; a mismatch means the parent
    /// was deleted and its record reused.
    pub parent_seq: u16,
    pub name: String,
    /// The name had unpaired UTF-16 surrogates and was stored lossily.
    pub lossy: bool,
    pub namespace: u8,
}

/// One run of a non-resident attribute: `clusters` clusters starting at `lcn`,
/// or a sparse hole when `lcn` is `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub lcn: Option<u64>,
    pub clusters: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub sequence: u16,
    /// For an extension record, the base record it belongs to.
    pub base: Option<u64>,
    pub is_dir: bool,
    pub names: Vec<FileName>,
    /// Length of the unnamed data stream, if this record holds its first part.
    pub logical: Option<u64>,
    /// Clusters allocated to the parts of all data streams held in this record,
    /// counted from their runs: holes in sparse and compressed streams take none.
    pub clusters: u64,
    pub reparse_tag: Option<u32>,
    /// Runs of the unnamed data stream; only decoded when asked for (for `$MFT`).
    pub data_runs: Vec<Run>,
}

impl Record {
    /// A symlink, junction or mount point: recorded, never followed.
    pub fn is_link(&self) -> bool {
        self.reparse_tag.is_some_and(|t| t & TAG_NAME_SURROGATE != 0)
    }
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(off..off + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(off..off + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(off..off + 8)?.try_into().ok()?))
}

/// Undoes the "update sequence" protection: NTFS stores a check value in the last
/// two bytes of every sector of a record and keeps the real bytes in an array in
/// the header. Returns `false` for a record that is not a valid `FILE` record or
/// was torn (a sector does not carry the check value).
pub fn apply_fixups(buf: &mut [u8], sector_size: usize) -> bool {
    if buf.get(0..4) != Some(b"FILE") || sector_size < 2 {
        return false;
    }
    let (Some(usa_off), Some(usa_count)) = (u16_at(buf, 4), u16_at(buf, 6)) else { return false };
    let (usa_off, usa_count) = (usa_off as usize, usa_count as usize);
    if usa_count < 2 || usa_off + usa_count * 2 > buf.len() || (usa_count - 1) * sector_size > buf.len() {
        return false;
    }
    let check = [buf[usa_off], buf[usa_off + 1]];
    for i in 1..usa_count {
        let end = i * sector_size;
        if buf[end - 2..end] != check {
            return false;
        }
        let fix = usa_off + i * 2;
        let (a, b) = (buf[fix], buf[fix + 1]);
        buf[end - 2] = a;
        buf[end - 1] = b;
    }
    true
}

/// Parses a record after [`apply_fixups`]. `None` for a record not in use or
/// damaged. `want_runs` also decodes the runs of the unnamed data stream.
pub fn parse(buf: &[u8], want_runs: bool) -> Option<Record> {
    if buf.get(0..4) != Some(b"FILE") {
        return None;
    }
    let flags = u16_at(buf, 0x16)?;
    if flags & RECORD_IN_USE == 0 {
        return None;
    }
    let used = (u32_at(buf, 0x18)? as usize).min(buf.len());
    let base = u64_at(buf, 0x20)? & REF_MASK;
    let mut rec = Record {
        sequence: u16_at(buf, 0x10)?,
        base: (base != 0).then_some(base),
        is_dir: flags & RECORD_IS_DIR != 0,
        ..Record::default()
    };

    let mut off = u16_at(buf, 0x14)? as usize;
    while off + 16 <= used {
        let kind = u32_at(buf, off)?;
        if kind == ATTR_END {
            break;
        }
        let len = u32_at(buf, off + 4)? as usize;
        if len < 16 || off + len > used {
            break;
        }
        let attr = &buf[off..off + len];
        let non_resident = attr[8] != 0;
        let named = attr[9] != 0;

        if non_resident {
            if kind == ATTR_DATA {
                // The length is only valid in the attribute's first part.
                if !named && u64_at(attr, 0x10)? == 0 {
                    rec.logical = Some(u64_at(attr, 0x30)?);
                }
                // The allocated size in the header is not used: a sparse stream
                // need not carry the sparse flag (`$BadClus:$Bad` spans the whole
                // volume as one hole), so only the runs tell what is on disk.
                let runs_off = u16_at(attr, 0x20)? as usize;
                let runs = attr.get(runs_off..).and_then(decode_runs).unwrap_or_default();
                rec.clusters += runs.iter().filter(|r| r.lcn.is_some()).map(|r| r.clusters).sum::<u64>();
                if !named && want_runs {
                    rec.data_runs.extend(runs);
                }
            }
            if kind == ATTR_REPARSE_POINT {
                // The tag lives in the (non-resident) value; treat it as an ordinary
                // reparse point, so a folder is still descended into.
                rec.reparse_tag.get_or_insert(0);
            }
        } else {
            let value_len = u32_at(attr, 0x10)? as usize;
            let value_off = u16_at(attr, 0x14)? as usize;
            let value = attr.get(value_off..value_off + value_len)?;
            match kind {
                ATTR_FILE_NAME => rec.names.push(parse_file_name(value)?),
                ATTR_DATA if !named => rec.logical = Some(value_len as u64),
                ATTR_REPARSE_POINT => rec.reparse_tag = Some(u32_at(value, 0)?),
                _ => {}
            }
        }
        off += len;
    }
    Some(rec)
}

fn parse_file_name(value: &[u8]) -> Option<FileName> {
    let parent = u64_at(value, 0)?;
    let len = *value.get(0x40)? as usize;
    let namespace = *value.get(0x41)?;
    let units: Vec<u16> = value
        .get(0x42..0x42 + len * 2)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_le_bytes(c))
        .collect();
    let (name, lossy) = match String::from_utf16(&units) {
        Ok(s) => (s, false),
        Err(_) => (String::from_utf16_lossy(&units), true),
    };
    Some(FileName { parent: parent & REF_MASK, parent_seq: (parent >> 48) as u16, name, lossy, namespace })
}

/// Decodes a run list: each run starts with a header byte whose low nibble is the
/// size of the length field and whose high nibble is the size of the (signed,
/// relative to the previous run) cluster offset; an offset size of 0 is a sparse
/// hole; a 0 header byte ends the list.
pub fn decode_runs(data: &[u8]) -> Option<Vec<Run>> {
    let mut runs = Vec::new();
    let mut pos = 0;
    let mut lcn: i64 = 0;
    loop {
        let header = *data.get(pos)?;
        if header == 0 {
            return Some(runs);
        }
        let (len_size, off_size) = ((header & 0x0F) as usize, (header >> 4) as usize);
        if len_size == 0 || len_size > 8 || off_size > 8 {
            return None;
        }
        pos += 1;
        let clusters = le_unsigned(data.get(pos..pos + len_size)?);
        pos += len_size;
        let run_lcn = if off_size == 0 {
            None
        } else {
            lcn = lcn.checked_add(le_signed(data.get(pos..pos + off_size)?))?;
            pos += off_size;
            Some(u64::try_from(lcn).ok()?)
        };
        runs.push(Run { lcn: run_lcn, clusters });
    }
}

fn le_unsigned(bytes: &[u8]) -> u64 {
    bytes.iter().rev().fold(0, |acc, &b| (acc << 8) | u64::from(b))
}

fn le_signed(bytes: &[u8]) -> i64 {
    let unsigned = le_unsigned(bytes);
    let bits = bytes.len() * 8;
    if bits < 64 && unsigned & (1 << (bits - 1)) != 0 {
        (unsigned | (!0u64 << bits)) as i64
    } else {
        unsigned as i64
    }
}

/// Builders for synthetic records, shared with the tree-building tests.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    pub const RECORD_SIZE: usize = 1024;
    pub const SECTOR: usize = 512;
    const FIRST_ATTR: usize = 0x38;

    pub struct RecordBuilder {
        flags: u16,
        sequence: u16,
        base: u64,
        attrs: Vec<Vec<u8>>,
    }

    impl RecordBuilder {
        pub fn new(is_dir: bool) -> Self {
            RecordBuilder { flags: RECORD_IN_USE | if is_dir { RECORD_IS_DIR } else { 0 }, sequence: 1, base: 0, attrs: Vec::new() }
        }

        pub fn not_in_use(mut self) -> Self {
            self.flags &= !RECORD_IN_USE;
            self
        }

        pub fn sequence(mut self, seq: u16) -> Self {
            self.sequence = seq;
            self
        }

        pub fn extension_of(mut self, base: u64) -> Self {
            self.base = base;
            self
        }

        fn resident(&mut self, kind: u32, name: &str, value: &[u8]) {
            let name16: Vec<u8> = name.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let name_off = 0x18;
            let value_off = (name_off + name16.len()).next_multiple_of(8);
            let len = (value_off + value.len()).next_multiple_of(8);
            let mut a = vec![0u8; len];
            a[0..4].copy_from_slice(&kind.to_le_bytes());
            a[4..8].copy_from_slice(&(len as u32).to_le_bytes());
            a[9] = (name16.len() / 2) as u8;
            a[0x0A..0x0C].copy_from_slice(&(name_off as u16).to_le_bytes());
            a[0x10..0x14].copy_from_slice(&(value.len() as u32).to_le_bytes());
            a[0x14..0x16].copy_from_slice(&(value_off as u16).to_le_bytes());
            a[name_off..name_off + name16.len()].copy_from_slice(&name16);
            a[value_off..value_off + value.len()].copy_from_slice(value);
            self.attrs.push(a);
        }

        pub fn file_name(mut self, parent: u64, parent_seq: u16, name: &str, namespace: u8) -> Self {
            let units: Vec<u16> = name.encode_utf16().collect();
            let mut v = vec![0u8; 0x42 + units.len() * 2];
            v[0..8].copy_from_slice(&(parent | (u64::from(parent_seq) << 48)).to_le_bytes());
            v[0x40] = units.len() as u8;
            v[0x41] = namespace;
            for (i, u) in units.iter().enumerate() {
                v[0x42 + i * 2..0x44 + i * 2].copy_from_slice(&u.to_le_bytes());
            }
            self.resident(ATTR_FILE_NAME, "", &v);
            self
        }

        pub fn resident_data(mut self, stream: &str, len: usize) -> Self {
            self.resident(ATTR_DATA, stream, &vec![0xAB; len]);
            self
        }

        pub fn reparse(mut self, tag: u32) -> Self {
            self.resident(ATTR_REPARSE_POINT, "", &tag.to_le_bytes());
            self
        }

        /// A non-resident data stream. `allocated` goes into the header only; the
        /// parser counts the clusters of `runs`.
        pub fn data(mut self, stream: &str, lowest_vcn: u64, allocated: u64, size: u64, runs: &[u8]) -> Self {
            let name16: Vec<u8> = stream.encode_utf16().flat_map(u16::to_le_bytes).collect();
            let name_off = 0x48;
            let runs_off = (name_off + name16.len()).next_multiple_of(8);
            let len = (runs_off + runs.len() + 1).next_multiple_of(8);
            let mut a = vec![0u8; len];
            a[0..4].copy_from_slice(&ATTR_DATA.to_le_bytes());
            a[4..8].copy_from_slice(&(len as u32).to_le_bytes());
            a[8] = 1;
            a[9] = (name16.len() / 2) as u8;
            a[0x0A..0x0C].copy_from_slice(&(name_off as u16).to_le_bytes());
            a[0x10..0x18].copy_from_slice(&lowest_vcn.to_le_bytes());
            a[0x20..0x22].copy_from_slice(&(runs_off as u16).to_le_bytes());
            a[0x28..0x30].copy_from_slice(&allocated.to_le_bytes());
            a[0x30..0x38].copy_from_slice(&size.to_le_bytes());
            a[0x38..0x40].copy_from_slice(&size.to_le_bytes());
            a[name_off..name_off + name16.len()].copy_from_slice(&name16);
            a[runs_off..runs_off + runs.len()].copy_from_slice(runs);
            self.attrs.push(a);
            self
        }

        /// The record as stored on disk, with fixups applied.
        pub fn build(self) -> Vec<u8> {
            let mut b = vec![0u8; RECORD_SIZE];
            b[0..4].copy_from_slice(b"FILE");
            let usa_off = 0x30u16;
            let usa_count = (RECORD_SIZE / SECTOR + 1) as u16;
            b[4..6].copy_from_slice(&usa_off.to_le_bytes());
            b[6..8].copy_from_slice(&usa_count.to_le_bytes());
            b[0x10..0x12].copy_from_slice(&self.sequence.to_le_bytes());
            b[0x14..0x16].copy_from_slice(&(FIRST_ATTR as u16).to_le_bytes());
            b[0x16..0x18].copy_from_slice(&self.flags.to_le_bytes());
            b[0x20..0x28].copy_from_slice(&self.base.to_le_bytes());
            let mut off = FIRST_ATTR;
            for a in &self.attrs {
                b[off..off + a.len()].copy_from_slice(a);
                off += a.len();
            }
            b[off..off + 4].copy_from_slice(&ATTR_END.to_le_bytes());
            b[0x18..0x1C].copy_from_slice(&((off + 8) as u32).to_le_bytes());
            b[0x1C..0x20].copy_from_slice(&(RECORD_SIZE as u32).to_le_bytes());
            // Protect each sector: move its last two bytes into the array, put the
            // check value in their place.
            let check = [0x42u8, 0x00];
            b[usa_off as usize..usa_off as usize + 2].copy_from_slice(&check);
            for i in 1..usa_count as usize {
                let end = i * SECTOR;
                let fix = usa_off as usize + i * 2;
                b[fix] = b[end - 2];
                b[fix + 1] = b[end - 1];
                b[end - 2..end].copy_from_slice(&check);
            }
            b
        }
    }

    /// Fixes up and parses a built record.
    pub fn parsed(mut bytes: Vec<u8>) -> Option<Record> {
        assert!(apply_fixups(&mut bytes, SECTOR), "fixups failed");
        parse(&bytes, true)
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    #[test]
    fn fixups_restore_sector_ends_and_detect_torn_records() {
        // Data reaching across the first sector boundary.
        let bytes = RecordBuilder::new(false).file_name(5, 5, &"x".repeat(250), 1).build();
        let mut ok = bytes.clone();
        assert!(apply_fixups(&mut ok, SECTOR));
        assert_eq!(parse(&ok, false).unwrap().names[0].name, "x".repeat(250));

        let mut torn = bytes.clone();
        torn[SECTOR - 1] ^= 0xFF;
        assert!(!apply_fixups(&mut torn, SECTOR));

        let mut not_file = bytes;
        not_file[0] = b'B';
        assert!(!apply_fixups(&mut not_file, SECTOR));
    }

    #[test]
    fn parses_names_and_resident_data() {
        let r = parsed(
            RecordBuilder::new(false)
                .sequence(7)
                .file_name(42, 3, "Отчёт за год.docx", 1)
                .file_name(42, 3, "OTCHET~1.DOC", NAMESPACE_DOS)
                .resident_data("", 300)
                .build(),
        )
        .unwrap();
        assert_eq!(r.sequence, 7);
        assert!(!r.is_dir && r.base.is_none());
        assert_eq!(r.names.len(), 2);
        assert_eq!(r.names[0], FileName { parent: 42, parent_seq: 3, name: "Отчёт за год.docx".into(), lossy: false, namespace: 1 });
        assert_eq!(r.logical, Some(300));
        assert_eq!(r.clusters, 0, "resident data lives inside the record");
    }

    #[test]
    fn parses_non_resident_and_named_streams() {
        let r = parsed(
            RecordBuilder::new(false)
                .file_name(5, 5, "movie.mkv", 1)
                .data("", 0, 8192, 5000, &[0x11, 0x02, 0x10, 0x00])
                .data("Zone.Identifier", 0, 4096, 26, &[0x11, 0x01, 0x20, 0x00])
                .build(),
        )
        .unwrap();
        assert_eq!(r.logical, Some(5000), "the named stream does not change the length");
        assert_eq!(r.clusters, 2 + 1, "but its clusters count");
        assert_eq!(r.data_runs, vec![Run { lcn: Some(0x10), clusters: 2 }]);
    }

    #[test]
    fn holes_take_no_space_whatever_the_header_says() {
        // 16 real clusters, then a hole of 240: a compressed or sparse file.
        let r = parsed(
            RecordBuilder::new(false)
                .file_name(5, 5, "zeros.bin", 1)
                .data("", 0, 256 * 4096, 256 * 4096, &[0x11, 0x10, 0x20, 0x01, 0xF0, 0x00])
                .build(),
        )
        .unwrap();
        assert_eq!(r.clusters, 16);

        // `$BadClus:$Bad`: one hole the size of the volume, no sparse flag, and an
        // allocated size in the header equal to the whole volume.
        let bad = parsed(
            RecordBuilder::new(false)
                .file_name(5, 5, "$BadClus", 1)
                .data("$Bad", 0, 500 << 30, 500 << 30, &[0x04, 0x00, 0x00, 0x00, 0x08, 0x00])
                .build(),
        )
        .unwrap();
        assert_eq!(bad.clusters, 0);
    }

    #[test]
    fn later_parts_add_clusters_but_no_length() {
        // A continuation (lowest VCN > 0) in an extension record.
        let ext = parsed(RecordBuilder::new(false).extension_of(77).data("", 512, 999, 999, &[0x11, 0x04, 0x30, 0x00]).build()).unwrap();
        assert_eq!(ext.base, Some(77));
        assert_eq!((ext.logical, ext.clusters), (None, 4));
    }

    #[test]
    fn reparse_tags_tell_links_from_placeholders() {
        let junction = parsed(RecordBuilder::new(true).file_name(5, 5, "link", 1).reparse(0xA000_0003).build()).unwrap();
        assert!(junction.is_dir && junction.is_link());
        let onedrive = parsed(RecordBuilder::new(true).file_name(5, 5, "OneDrive", 1).reparse(0x9000_601A).build()).unwrap();
        assert!(!onedrive.is_link());
    }

    #[test]
    fn records_not_in_use_are_skipped() {
        assert!(parsed(RecordBuilder::new(false).not_in_use().file_name(5, 5, "gone", 1).build()).is_none());
    }

    #[test]
    fn unpaired_surrogates_are_flagged() {
        let mut bytes = RecordBuilder::new(false).file_name(5, 5, "ab", 1).build();
        // Overwrite the name's first UTF-16 unit with a lone high surrogate.
        let pos = bytes.windows(4).position(|w| w == [b'a', 0, b'b', 0]).unwrap();
        bytes[pos..pos + 2].copy_from_slice(&0xD800u16.to_le_bytes());
        let r = parsed(bytes).unwrap();
        assert!(r.names[0].lossy);
        assert_eq!(r.names[0].name, "\u{FFFD}b");
    }

    #[test]
    fn decodes_run_lists() {
        // 0x21: 1-byte length, 2-byte offset. Second run goes back (negative delta),
        // the third is sparse.
        let runs = decode_runs(&[0x21, 0x10, 0x00, 0x01, 0x11, 0x08, 0xF0, 0x01, 0x04, 0x00]).unwrap();
        assert_eq!(
            runs,
            vec![
                Run { lcn: Some(0x100), clusters: 0x10 },
                Run { lcn: Some(0x100 - 0x10), clusters: 8 },
                Run { lcn: None, clusters: 4 },
            ]
        );
        assert_eq!(decode_runs(&[0]).unwrap(), vec![]);
        assert!(decode_runs(&[0x21, 0x10]).is_none(), "truncated");
        assert!(decode_runs(&[0x11, 0x01, 0x80, 0x00]).is_none(), "negative cluster number");
    }

    #[test]
    fn damaged_records_do_not_panic() {
        let good = RecordBuilder::new(false).file_name(5, 5, "f", 1).resident_data("", 10).build();
        for cut in [0, 3, 0x20, 0x40, 0x60, 0x100] {
            let mut b = good.clone();
            apply_fixups(&mut b, SECTOR);
            let _ = parse(&b[..cut], true);
        }
        let mut b = good;
        apply_fixups(&mut b, SECTOR);
        // An attribute claiming to be longer than the record.
        b[0x38 + 4..0x38 + 8].copy_from_slice(&0xFFFFu32.to_le_bytes());
        let r = parse(&b, true).unwrap();
        assert!(r.names.is_empty());
    }
}
