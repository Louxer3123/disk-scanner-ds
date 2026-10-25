//! Disk health from S.M.A.R.T.: what CrystalDiskInfo shows. For every physical
//! disk: model, serial and firmware, then for SATA disks the attribute table
//! with thresholds, for NVMe disks the health log; from either, an overall
//! verdict (good, caution, bad), temperature, power-on hours, power cycles,
//! amount written and, for SSDs, remaining life.
//!
//! The description of a disk can be read by anyone; S.M.A.R.T. data needs
//! administrator rights (the disk must be opened for reading and writing).
//! Without them `smart_error` says so and only the description is filled in.

/// Overall verdict, as CrystalDiskInfo gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    /// No S.M.A.R.T. data (no rights, unsupported disk or bridge).
    Unknown,
    Good,
    /// Something to keep an eye on: reallocated or pending sectors, media
    /// errors, little life left, a temperature warning.
    Caution,
    /// An attribute below its threshold, or the drive reports a critical state.
    Bad,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Unknown => "unknown",
            Verdict::Good => "good",
            Verdict::Caution => "caution",
            Verdict::Bad => "bad",
        }
    }
}

/// One row of a SATA disk's attribute table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub id: u8,
    pub name: &'static str,
    /// Normalized value, 1..=253 (higher is better), and the worst it has been.
    pub current: u8,
    pub worst: u8,
    /// The vendor's failure threshold for `current`; 0 when there is none.
    pub threshold: u8,
    /// The raw value, 48 bits; its meaning depends on the attribute and vendor.
    pub raw: u64,
    /// Bit 0 of the flags: a pre-failure attribute (versus an age counter).
    pub prefailure: bool,
}

/// What a drive reports about itself, in units people read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HealthFacts {
    pub temperature: Option<i32>,
    pub power_on_hours: Option<u64>,
    pub power_cycles: Option<u64>,
    pub unsafe_shutdowns: Option<u64>,
    pub bytes_written: Option<u64>,
    pub bytes_read: Option<u64>,
    /// SSD life left, percent.
    pub life_left: Option<u8>,
    /// NVMe: spare capacity left and the level the vendor warns at, percent.
    pub available_spare: Option<u8>,
    pub spare_threshold: Option<u8>,
    pub media_errors: Option<u64>,
    pub reallocated_sectors: Option<u64>,
    pub pending_sectors: Option<u64>,
    pub uncorrectable_sectors: Option<u64>,
    /// NVMe critical warning bits, as reported.
    pub critical_warning: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskHealth {
    /// N in `\\.\PhysicalDriveN`; matches [`crate::DriveInfo::physical_disk`].
    pub number: u32,
    pub model: String,
    pub serial: String,
    pub firmware: String,
    /// `NVMe`, `SATA`, `USB`, ... as Windows names the bus.
    pub bus: &'static str,
    pub size: u64,
    /// `Some(true)` for a spinning disk, `Some(false)` for an SSD.
    pub rotational: Option<bool>,
    pub verdict: Verdict,
    /// Why the verdict is what it is, in short English phrases, e.g.
    /// `"reallocated sectors: 8"`; empty when all is well.
    pub reasons: Vec<String>,
    pub facts: HealthFacts,
    /// SATA disks: the attribute table. NVMe disks: empty.
    pub attributes: Vec<Attribute>,
    /// Why there is no S.M.A.R.T. data, if there is none.
    pub smart_error: Option<SmartError>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartError {
    /// Opening the disk for S.M.A.R.T. needs administrator rights.
    NeedsAdmin,
    /// The disk or its controller (often a USB bridge) does not pass S.M.A.R.T. through.
    Unsupported,
}

// ---------- ATA (SATA) ----------

/// Names of the common ATA attributes, as CrystalDiskInfo and smartctl call them.
pub fn attribute_name(id: u8) -> &'static str {
    match id {
        0x01 => "Read Error Rate",
        0x02 => "Throughput Performance",
        0x03 => "Spin-Up Time",
        0x04 => "Start/Stop Count",
        0x05 => "Reallocated Sectors Count",
        0x07 => "Seek Error Rate",
        0x08 => "Seek Time Performance",
        0x09 => "Power-On Hours",
        0x0A => "Spin Retry Count",
        0x0B => "Recalibration Retries",
        0x0C => "Power Cycle Count",
        0xA9 => "Remaining Life",
        0xAA => "Available Reserved Space",
        0xAB => "Program Fail Count",
        0xAC => "Erase Fail Count",
        0xAD => "Wear Leveling Count",
        0xAE => "Unexpected Power Loss Count",
        0xB1 => "Wear Leveling Count",
        0xB3 => "Used Reserved Block Count",
        0xB4 => "Unused Reserved Block Count",
        0xB5 => "Program Fail Count",
        0xB6 => "Erase Fail Count",
        0xB7 => "SATA Downshift Error Count",
        0xB8 => "End-to-End Error",
        0xBB => "Reported Uncorrectable Errors",
        0xBC => "Command Timeout",
        0xBD => "High Fly Writes",
        0xBE => "Airflow Temperature",
        0xBF => "G-Sense Error Rate",
        0xC0 => "Power-off Retract Count",
        0xC1 => "Load/Unload Cycle Count",
        0xC2 => "Temperature",
        0xC3 => "Hardware ECC Recovered",
        0xC4 => "Reallocation Event Count",
        0xC5 => "Current Pending Sector Count",
        0xC6 => "Uncorrectable Sector Count",
        0xC7 => "UltraDMA CRC Error Count",
        0xC8 => "Write Error Rate",
        0xCA => "Data Address Mark Errors",
        0xE7 => "SSD Life Left",
        0xE8 => "Available Reserved Space",
        0xE9 => "Media Wearout Indicator",
        0xF1 => "Total Host Writes",
        0xF2 => "Total Host Reads",
        _ => "Vendor Specific",
    }
}

/// Parses the 512-byte answers to SMART READ DATA (attributes) and SMART READ
/// THRESHOLDS: up to 30 entries of 12 bytes after a 2-byte version.
pub fn parse_ata_attributes(data: &[u8; 512], thresholds: Option<&[u8; 512]>) -> Vec<Attribute> {
    let threshold_of = |id: u8| -> u8 {
        let Some(t) = thresholds else { return 0 };
        (0..30).map(|i| 2 + i * 12).find(|&o| t[o] == id).map_or(0, |o| t[o + 1])
    };
    (0..30)
        .map(|i| &data[2 + i * 12..2 + (i + 1) * 12])
        .filter(|e| e[0] != 0)
        .map(|e| Attribute {
            id: e[0],
            name: attribute_name(e[0]),
            current: e[3],
            worst: e[4],
            threshold: threshold_of(e[0]),
            raw: e[5..11].iter().rev().fold(0u64, |acc, &b| (acc << 8) | u64::from(b)),
            prefailure: e[1] & 1 != 0,
        })
        .collect()
}

/// Facts and verdict from an attribute table. Follows CrystalDiskInfo: an
/// attribute at or below a non-zero threshold is bad; reallocated, pending or
/// uncorrectable sectors are a caution.
pub fn evaluate_ata(attrs: &[Attribute], rotational: Option<bool>) -> (HealthFacts, Verdict, Vec<String>) {
    let get = |id: u8| attrs.iter().find(|a| a.id == id);
    let raw = |id: u8| get(id).map(|a| a.raw);
    let mut f = HealthFacts {
        // Temperature: the low byte of the raw value; the others hold min/max on some drives.
        temperature: raw(0xC2).or(raw(0xBE)).map(|r| (r & 0xFF) as i32).filter(|&t| t > 0 && t < 150),
        // Some drives count minutes or half-minutes in the upper bytes; the low 32 bits are hours.
        power_on_hours: raw(0x09).map(|r| r & 0xFFFF_FFFF),
        power_cycles: raw(0x0C),
        unsafe_shutdowns: raw(0xC0).or(raw(0xAE)),
        reallocated_sectors: raw(0x05).map(|r| r & 0xFFFF_FFFF),
        pending_sectors: raw(0xC5).map(|r| r & 0xFFFF_FFFF),
        uncorrectable_sectors: raw(0xC6).map(|r| r & 0xFFFF_FFFF),
        // Total host writes are counted in 512-byte LBAs on most SSDs.
        bytes_written: raw(0xF1).map(|r| r.saturating_mul(512)),
        bytes_read: raw(0xF2).map(|r| r.saturating_mul(512)),
        ..HealthFacts::default()
    };
    if rotational != Some(true) {
        f.life_left = [0xE7, 0xA9, 0xB1, 0xAD, 0xE9]
            .iter()
            .find_map(|&id| get(id))
            .map(|a| a.current.min(100));
    }

    let mut reasons = Vec::new();
    let mut verdict = Verdict::Good;
    for a in attrs {
        // 0 means "no threshold"; current 0 means the field is not used.
        if a.threshold > 0 && a.threshold < 0xFE && a.current > 0 && a.current <= a.threshold {
            verdict = Verdict::Bad;
            reasons.push(format!("{} below threshold ({} ≤ {})", a.name, a.current, a.threshold));
        }
    }
    for (count, what) in [
        (f.reallocated_sectors, "reallocated sectors"),
        (f.pending_sectors, "pending sectors"),
        (f.uncorrectable_sectors, "uncorrectable sectors"),
    ] {
        if let Some(n) = count.filter(|&n| n > 0) {
            verdict = verdict.max(Verdict::Caution);
            reasons.push(format!("{what}: {n}"));
        }
    }
    (f, verdict, reasons)
}

// ---------- NVMe ----------

fn le(bytes: &[u8]) -> u64 {
    // The log's 128-bit counters fit in 64 bits for any real drive; saturate otherwise.
    if bytes.len() > 8 && bytes[8..].iter().any(|&b| b != 0) {
        return u64::MAX;
    }
    bytes.iter().take(8).rev().fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
}

/// Facts and verdict from the 512-byte NVMe SMART / Health Information log
/// page (log identifier 02h).
pub fn evaluate_nvme(log: &[u8; 512]) -> (HealthFacts, Verdict, Vec<String>) {
    let critical = log[0];
    let kelvin = u16::from_le_bytes([log[1], log[2]]);
    let f = HealthFacts {
        critical_warning: Some(critical),
        temperature: (kelvin > 0).then(|| i32::from(kelvin) - 273),
        available_spare: Some(log[3]),
        spare_threshold: Some(log[4]),
        // "Percentage used" may exceed 100 on a worn drive.
        life_left: Some(100u8.saturating_sub(log[5])),
        // Data units are thousands of 512-byte units.
        bytes_read: Some(le(&log[32..48]).saturating_mul(512_000)),
        bytes_written: Some(le(&log[48..64]).saturating_mul(512_000)),
        power_cycles: Some(le(&log[112..128])),
        power_on_hours: Some(le(&log[128..144])),
        unsafe_shutdowns: Some(le(&log[144..160])),
        media_errors: Some(le(&log[160..176])),
        ..HealthFacts::default()
    };

    let mut reasons = Vec::new();
    let mut verdict = Verdict::Good;
    for (bit, what, bad) in [
        (0, "available spare below threshold", true),
        (1, "temperature warning", false),
        (2, "reliability degraded", true),
        (3, "read-only mode", true),
        (4, "volatile memory backup failed", true),
    ] {
        if critical & (1 << bit) != 0 {
            verdict = verdict.max(if bad { Verdict::Bad } else { Verdict::Caution });
            reasons.push(what.to_owned());
        }
    }
    if let Some(n) = f.media_errors.filter(|&n| n > 0) {
        verdict = verdict.max(Verdict::Caution);
        reasons.push(format!("media errors: {n}"));
    }
    if f.life_left.is_some_and(|l| l <= 10) {
        verdict = verdict.max(Verdict::Caution);
        reasons.push(format!("life left: {}%", f.life_left.unwrap_or(0)));
    }
    (f, verdict, reasons)
}

#[cfg_attr(not(windows), allow(dead_code))]
fn bus_name(bus: u32) -> &'static str {
    match bus {
        1 => "SCSI",
        2 => "ATAPI",
        3 => "ATA",
        4 => "1394",
        7 => "USB",
        8 => "RAID",
        9 => "iSCSI",
        10 => "SAS",
        11 => "SATA",
        12 => "SD",
        13 => "MMC",
        14 | 15 => "Virtual",
        16 => "Storage Spaces",
        17 => "NVMe",
        18 => "SCM",
        19 => "UFS",
        _ => "Unknown",
    }
}

pub use imp::list_disks;

#[cfg(windows)]
mod imp {
    use super::*;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::IO::DeviceIoControl;
    use windows_sys::Win32::System::Ioctl::{
        IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_STORAGE_QUERY_PROPERTY, SMART_RCV_DRIVE_DATA,
    };

    const MAX_DISKS: u32 = 32;
    const ERROR_ACCESS_DENIED: u32 = 5;

    struct Handle(HANDLE);

    impl Drop for Handle {
        fn drop(&mut self) {
            // SAFETY: opened by `open` and closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Opens `\\.\PhysicalDriveN`. Access 0 is enough for queries about the
    /// device; S.M.A.R.T. needs read and write, which needs administrator rights.
    fn open(number: u32, access: u32) -> Result<Handle, u32> {
        let path: Vec<u16> = format!(r"\\.\PhysicalDrive{number}").encode_utf16().chain([0]).collect();
        // SAFETY: NUL-terminated path; the handle is owned by `Handle`.
        let h = unsafe {
            CreateFileW(path.as_ptr(), access, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut())
        };
        if h == INVALID_HANDLE_VALUE {
            // SAFETY: read right after the failed call.
            return Err(unsafe { GetLastError() });
        }
        Ok(Handle(h))
    }

    /// One DeviceIoControl round trip; `None` on failure.
    fn ioctl(h: &Handle, code: u32, input: &[u8], output: &mut [u8]) -> Option<usize> {
        let mut returned = 0u32;
        // SAFETY: both buffers are valid for the lengths passed.
        let ok = unsafe {
            DeviceIoControl(
                h.0,
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                output.as_mut_ptr().cast(),
                output.len() as u32,
                &mut returned,
                null_mut(),
            ) != 0
        };
        ok.then_some(returned as usize)
    }

    fn u32_at(b: &[u8], off: usize) -> u32 {
        b.get(off..off + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().expect("4 bytes")))
    }

    /// A NUL-terminated ASCII string at `offset` in `buf`, trimmed.
    fn string_at(buf: &[u8], offset: u32) -> String {
        let Some(rest) = buf.get(offset as usize..).filter(|_| offset != 0) else { return String::new() };
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        String::from_utf8_lossy(&rest[..end]).trim().to_owned()
    }

    fn property_query(property: u32) -> [u8; 12] {
        let mut q = [0u8; 12];
        q[0..4].copy_from_slice(&property.to_le_bytes());
        q
    }

    /// Model, serial, firmware, bus, size and rotation of a disk.
    fn describe(number: u32, h: &Handle) -> DiskHealth {
        let mut desc = vec![0u8; 1024];
        let n = ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, &property_query(0), &mut desc).unwrap_or(0);
        desc.truncate(n);
        let vendor = string_at(&desc, u32_at(&desc, 12));
        let product = string_at(&desc, u32_at(&desc, 16));
        let model = if vendor.is_empty() || product.starts_with(&vendor) { product } else { format!("{vendor} {product}") };

        let mut geometry = [0u8; 256];
        let size = ioctl(h, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut geometry)
            .filter(|&n| n >= 32)
            .map_or(0, |_| u64::from_le_bytes(geometry[24..32].try_into().expect("8 bytes")));

        // StorageDeviceSeekPenaltyProperty: whether seeks cost time, i.e. a spinning disk.
        let mut seek = [0u8; 12];
        let rotational = ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, &property_query(7), &mut seek)
            .filter(|&n| n >= 9)
            .map(|_| seek[8] != 0);

        DiskHealth {
            number,
            model,
            serial: string_at(&desc, u32_at(&desc, 24)),
            firmware: string_at(&desc, u32_at(&desc, 20)),
            bus: bus_name(u32_at(&desc, 28)),
            size,
            rotational,
            verdict: Verdict::Unknown,
            reasons: Vec::new(),
            facts: HealthFacts::default(),
            attributes: Vec::new(),
            smart_error: None,
        }
    }

    /// The NVMe health log page through the storage driver's protocol query.
    fn nvme_health_log(h: &Handle) -> Option<[u8; 512]> {
        // STORAGE_PROPERTY_QUERY (8 bytes up to AdditionalParameters), then
        // STORAGE_PROTOCOL_SPECIFIC_DATA (40 bytes), then room for the data.
        const SPECIFIC: usize = 40;
        let mut buf = vec![0u8; 8 + SPECIFIC + 512];
        buf[0..4].copy_from_slice(&50u32.to_le_bytes()); // StorageDeviceProtocolSpecificProperty
        buf[4..8].copy_from_slice(&0u32.to_le_bytes()); // PropertyStandardQuery
        let fields = [3u32, 2, 2, 0, SPECIFIC as u32, 512]; // NVMe, log page, health log, -, offset, length
        for (i, v) in fields.iter().enumerate() {
            buf[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
        }
        let input = buf.clone();
        let n = ioctl(h, IOCTL_STORAGE_QUERY_PROPERTY, &input, &mut buf)?;
        // STORAGE_PROTOCOL_DATA_DESCRIPTOR: Version, Size, then the specific data,
        // whose ProtocolDataOffset counts from its own start.
        let offset = 8 + u32_at(&buf, 8 + 16) as usize;
        let length = u32_at(&buf, 8 + 20) as usize;
        if length < 512 || offset + 512 > n.min(buf.len()) {
            return None;
        }
        buf[offset..offset + 512].try_into().ok()
    }

    /// SMART READ DATA (0xD0) or READ THRESHOLDS (0xD1) through SMART_RCV_DRIVE_DATA.
    fn ata_smart(h: &Handle, feature: u8) -> Option<[u8; 512]> {
        // SENDCMDINPARAMS: cBufferSize, IDEREGS (8), bDriveNumber, 3 + 16 reserved, buffer.
        let mut input = [0u8; 32 + 1];
        input[0..4].copy_from_slice(&512u32.to_le_bytes());
        input[4..12].copy_from_slice(&[feature, 1, 1, 0x4F, 0xC2, 0xA0, 0xB0, 0]);
        // SENDCMDOUTPARAMS: cBufferSize, DRIVERSTATUS (12), then the 512 bytes.
        let mut output = [0u8; 16 + 512];
        let n = ioctl(h, SMART_RCV_DRIVE_DATA, &input, &mut output)?;
        if n < 16 + 512 || output[4] != 0 {
            return None;
        }
        output[16..].try_into().ok()
    }

    /// Every physical disk that can be opened, with its health where readable.
    pub fn list_disks() -> Vec<DiskHealth> {
        (0..MAX_DISKS)
            .filter_map(|n| {
                let query = open(n, 0).ok()?;
                let mut disk = describe(n, &query);
                // The NVMe health log is a query too: some drivers answer it
                // without administrator rights.
                if disk.bus == "NVMe"
                    && let Some(log) = nvme_health_log(&query)
                {
                    (disk.facts, disk.verdict, disk.reasons) = evaluate_nvme(&log);
                    return Some(disk);
                }
                match open(n, GENERIC_READ | GENERIC_WRITE) {
                    Err(ERROR_ACCESS_DENIED) => disk.smart_error = Some(SmartError::NeedsAdmin),
                    Err(_) => disk.smart_error = Some(SmartError::Unsupported),
                    Ok(h) => {
                        let evaluated = if disk.bus == "NVMe" {
                            nvme_health_log(&h).map(|log| evaluate_nvme(&log))
                        } else {
                            ata_smart(&h, 0xD0).map(|data| {
                                let thresholds = ata_smart(&h, 0xD1);
                                disk.attributes = parse_ata_attributes(&data, thresholds.as_ref());
                                evaluate_ata(&disk.attributes, disk.rotational)
                            })
                        };
                        match evaluated {
                            Some((facts, verdict, reasons)) => {
                                disk.facts = facts;
                                disk.verdict = verdict;
                                disk.reasons = reasons;
                            }
                            None => disk.smart_error = Some(SmartError::Unsupported),
                        }
                    }
                }
                Some(disk)
            })
            .collect()
    }
}

#[cfg(not(windows))]
mod imp {
    use super::DiskHealth;

    pub fn list_disks() -> Vec<DiskHealth> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An attribute table with `(id, flags, current, worst, raw)` rows.
    fn table(rows: &[(u8, u16, u8, u8, u64)]) -> [u8; 512] {
        let mut b = [0u8; 512];
        for (i, &(id, flags, cur, worst, raw)) in rows.iter().enumerate() {
            let o = 2 + i * 12;
            b[o] = id;
            b[o + 1..o + 3].copy_from_slice(&flags.to_le_bytes());
            b[o + 3] = cur;
            b[o + 4] = worst;
            b[o + 5..o + 11].copy_from_slice(&raw.to_le_bytes()[..6]);
        }
        b
    }

    fn thresholds(rows: &[(u8, u8)]) -> [u8; 512] {
        let mut b = [0u8; 512];
        for (i, &(id, t)) in rows.iter().enumerate() {
            b[2 + i * 12] = id;
            b[3 + i * 12] = t;
        }
        b
    }

    #[test]
    fn healthy_hdd_attributes() {
        let data = table(&[
            (0x01, 0x2F, 200, 200, 0),
            (0x05, 0x33, 200, 200, 0),
            (0x09, 0x32, 91, 91, 6789),
            (0x0C, 0x32, 100, 100, 1234),
            // 36 °C now; min 20 and max 45 in the upper bytes.
            (0xC2, 0x22, 114, 98, 36 | (20 << 16) | (45 << 32)),
            (0xC5, 0x32, 200, 200, 0),
        ]);
        let t = thresholds(&[(0x01, 51), (0x05, 140), (0x09, 0)]);
        let attrs = parse_ata_attributes(&data, Some(&t));
        assert_eq!(attrs.len(), 6);
        assert_eq!(attrs[1], Attribute {
            id: 5,
            name: "Reallocated Sectors Count",
            current: 200,
            worst: 200,
            threshold: 140,
            raw: 0,
            prefailure: true,
        });
        let (f, verdict, reasons) = evaluate_ata(&attrs, Some(true));
        assert_eq!(verdict, Verdict::Good, "{reasons:?}");
        assert_eq!((f.temperature, f.power_on_hours, f.power_cycles), (Some(36), Some(6789), Some(1234)));
        assert_eq!(f.life_left, None, "no life left on a spinning disk");
    }

    #[test]
    fn reallocated_sectors_are_a_caution_and_a_crossed_threshold_is_bad() {
        let data = table(&[(0x05, 0x33, 199, 199, 8), (0x09, 0x32, 90, 90, 100)]);
        let t = thresholds(&[(0x05, 140)]);
        let (_, v, reasons) = evaluate_ata(&parse_ata_attributes(&data, Some(&t)), Some(true));
        assert_eq!(v, Verdict::Caution);
        assert_eq!(reasons, ["reallocated sectors: 8"]);

        let data = table(&[(0x05, 0x33, 100, 100, 3000)]);
        let (_, v, reasons) = evaluate_ata(&parse_ata_attributes(&data, Some(&t)), Some(true));
        assert_eq!(v, Verdict::Bad);
        assert!(reasons[0].contains("below threshold"), "{reasons:?}");
    }

    #[test]
    fn ssd_life_and_writes() {
        let data = table(&[(0xB1, 0x13, 97, 97, 42), (0xF1, 0x32, 99, 99, 2_000_000_000)]);
        let (f, v, _) = evaluate_ata(&parse_ata_attributes(&data, None), Some(false));
        assert_eq!(v, Verdict::Good);
        assert_eq!(f.life_left, Some(97));
        assert_eq!(f.bytes_written, Some(1_024_000_000_000));
    }

    fn nvme_log(critical: u8, kelvin: u16, spare: u8, used: u8, written_units: u64, hours: u64, media_errors: u64) -> [u8; 512] {
        let mut b = [0u8; 512];
        b[0] = critical;
        b[1..3].copy_from_slice(&kelvin.to_le_bytes());
        b[3] = spare;
        b[4] = 10;
        b[5] = used;
        b[48..56].copy_from_slice(&written_units.to_le_bytes());
        b[112..120].copy_from_slice(&321u64.to_le_bytes());
        b[128..136].copy_from_slice(&hours.to_le_bytes());
        b[160..168].copy_from_slice(&media_errors.to_le_bytes());
        b
    }

    #[test]
    fn nvme_health_log() {
        let (f, v, reasons) = evaluate_nvme(&nvme_log(0, 311, 100, 3, 40_000_000, 5000, 0));
        assert_eq!(v, Verdict::Good, "{reasons:?}");
        assert_eq!(f.temperature, Some(38));
        assert_eq!(f.life_left, Some(97));
        assert_eq!(f.bytes_written, Some(40_000_000 * 512_000));
        assert_eq!((f.power_on_hours, f.power_cycles, f.media_errors), (Some(5000), Some(321), Some(0)));

        let (_, v, _) = evaluate_nvme(&nvme_log(0, 311, 100, 95, 0, 1, 0));
        assert_eq!(v, Verdict::Caution, "5% life left");
        let (_, v, _) = evaluate_nvme(&nvme_log(0b10, 360, 100, 3, 0, 1, 0));
        assert_eq!(v, Verdict::Caution, "too hot");
        let (_, v, reasons) = evaluate_nvme(&nvme_log(0b1000, 311, 100, 3, 0, 1, 2));
        assert_eq!(v, Verdict::Bad);
        assert_eq!(reasons, ["read-only mode", "media errors: 2"]);
        let (f, _, _) = evaluate_nvme(&nvme_log(0, 311, 100, 255, 0, 1, 0));
        assert_eq!(f.life_left, Some(0), "worn beyond 100%");
    }

    #[test]
    fn huge_counters_saturate() {
        let mut log = nvme_log(0, 300, 100, 0, 0, 0, 0);
        log[48 + 9] = 1; // bit 72 of data units written
        let (f, _, _) = evaluate_nvme(&log);
        assert_eq!(f.bytes_written, Some(u64::MAX));
    }
}
