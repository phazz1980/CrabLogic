//! The module images of the `crabmod` partition (`specs/wasm-kernel`, stage 3).
//!
//! The partition is cut into areas of variable length; an area starts at a flash sector and is
//! a header of [`HEADER`] bytes followed by the image (`.wasm` for the interpreter or an XIP
//! `.aot`, which runs straight from the area). A new version is written into free sectors
//! while the old one keeps running; only a complete, checked image gets a header, so a broken
//! upload leaves nothing. Of the areas with one name the highest generation wins.

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

use crate::wamr::ExecutionMode;

/// Flash erase unit; areas start on its boundary.
pub const SECTOR: usize = 4096;
/// Bytes of an area before the image. A multiple of 16: AOT text must be aligned.
pub const HEADER: usize = 256;
/// Longest module name (as the editor and the C++ master have it).
pub const MAX_NAME: usize = 15;
/// Longest list of hardware rights, comma separated.
pub const MAX_ALLOW: usize = 199;

/// "CRBM", as in the C++ master; the layout differs, hence the version.
const MAGIC: u32 = 0x4D42_5243;
const VERSION: u16 = 2;
const FLAG_AOT: u16 = 1;
// Offsets in the header (little-endian fields).
const AT_VERSION: usize = 4;
const AT_FLAGS: usize = 6;
const AT_LEN: usize = 8;
const AT_CRC: usize = 12;
const AT_GEN: usize = 16;
const AT_MEMORY: usize = 20;
const AT_NAME: usize = 24;
const AT_ALLOW: usize = AT_NAME + MAX_NAME + 1;
const AT_HCRC: usize = HEADER - 4;
const _: () = assert!(AT_ALLOW + MAX_ALLOW < AT_HCRC);

/// CRC-32 (IEEE 802.3) continued from `crc` (0 to start), as `esp_crc32_le` counts it.
pub fn crc32_update(crc: u32, bytes: &[u8]) -> u32 {
    let mut crc = !crc;
    for &byte in bytes {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

pub fn crc32(bytes: &[u8]) -> u32 {
    crc32_update(0, bytes)
}

/// A NOR flash partition: writing only clears bits, erasing sets a whole sector.
/// The methods return `false` on a failure of the chip or a range outside the partition.
pub trait Flash {
    fn size(&self) -> usize;
    fn read(&self, at: usize, buf: &mut [u8]) -> bool;
    fn write(&mut self, at: usize, data: &[u8]) -> bool;
    /// `at` and `len` are multiples of [`SECTOR`].
    fn erase(&mut self, at: usize, len: usize) -> bool;
}

/// Why the storage refused; `Display` is the reason of the `err` answer of the protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageError {
    /// No run of free sectors is long enough.
    NoSpace,
    /// The flash failed, or what was written does not read back.
    Flash,
    /// The image does not match the length and CRC of its `put`.
    Crc,
}

impl fmt::Display for StorageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoSpace => "no space",
            Self::Flash => "flash",
            Self::Crc => "crc/len",
        })
    }
}

/// What is stored with an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    pub name: String,
    pub len: usize,
    pub crc: u32,
    pub mode: ExecutionMode,
    /// Bytes of linear memory (`crab.mem`); 0 keeps what the module declares.
    pub memory: u32,
    /// Hardware rights, comma separated.
    pub allow: String,
}

/// A stored image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Area {
    /// Offset of the header in the partition.
    pub at: usize,
    pub gen: u32,
    pub meta: Meta,
}

impl Area {
    /// Offset of the image in the partition.
    pub fn data_at(&self) -> usize {
        self.at + HEADER
    }

    /// Bytes of the partition the area takes (whole sectors).
    pub fn size(&self) -> usize {
        area_size(self.meta.len)
    }

    pub fn allow_list(&self) -> Vec<String> {
        self.meta
            .allow
            .split(',')
            .filter(|a| !a.is_empty())
            .map(String::from)
            .collect()
    }
}

fn area_size(len: usize) -> usize {
    (HEADER + len).div_ceil(SECTOR) * SECTOR
}

fn u32_at(h: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([h[at], h[at + 1], h[at + 2], h[at + 3]])
}

fn text_at(h: &[u8], at: usize, max: usize) -> Option<String> {
    let field = &h[at..at + max + 1];
    let len = field.iter().position(|&b| b == 0)?;
    core::str::from_utf8(&field[..len]).ok().map(String::from)
}

fn encode(meta: &Meta, gen: u32) -> [u8; HEADER] {
    let mut h = [0u8; HEADER];
    h[..4].copy_from_slice(&MAGIC.to_le_bytes());
    h[AT_VERSION..AT_VERSION + 2].copy_from_slice(&VERSION.to_le_bytes());
    let flags = if meta.mode == ExecutionMode::AotXip {
        FLAG_AOT
    } else {
        0
    };
    h[AT_FLAGS..AT_FLAGS + 2].copy_from_slice(&flags.to_le_bytes());
    h[AT_LEN..AT_LEN + 4].copy_from_slice(&(meta.len as u32).to_le_bytes());
    h[AT_CRC..AT_CRC + 4].copy_from_slice(&meta.crc.to_le_bytes());
    h[AT_GEN..AT_GEN + 4].copy_from_slice(&gen.to_le_bytes());
    h[AT_MEMORY..AT_MEMORY + 4].copy_from_slice(&meta.memory.to_le_bytes());
    h[AT_NAME..AT_NAME + meta.name.len()].copy_from_slice(meta.name.as_bytes());
    h[AT_ALLOW..AT_ALLOW + meta.allow.len()].copy_from_slice(meta.allow.as_bytes());
    let hcrc = crc32(&h[..AT_HCRC]);
    h[AT_HCRC..].copy_from_slice(&hcrc.to_le_bytes());
    h
}

fn decode(h: &[u8; HEADER]) -> Option<(Meta, u32)> {
    if u32_at(h, 0) != MAGIC
        || u16::from_le_bytes([h[AT_VERSION], h[AT_VERSION + 1]]) != VERSION
        || u32_at(h, AT_HCRC) != crc32(&h[..AT_HCRC])
    {
        return None;
    }
    let flags = u16::from_le_bytes([h[AT_FLAGS], h[AT_FLAGS + 1]]);
    let meta = Meta {
        name: text_at(h, AT_NAME, MAX_NAME)?,
        len: u32_at(h, AT_LEN) as usize,
        crc: u32_at(h, AT_CRC),
        mode: if flags & FLAG_AOT != 0 {
            ExecutionMode::AotXip
        } else {
            ExecutionMode::Interpreter
        },
        memory: u32_at(h, AT_MEMORY),
        allow: text_at(h, AT_ALLOW, MAX_ALLOW)?,
    };
    (meta.len > 0 && !meta.name.is_empty()).then_some((meta, u32_at(h, AT_GEN)))
}

/// The areas of a partition.
#[derive(Debug)]
pub struct Storage<F> {
    flash: F,
    /// Sorted by offset.
    areas: Vec<Area>,
    next_gen: u32,
    /// Where the search for free sectors starts: after the newest area, so the writes go round.
    cursor: usize,
}

impl<F: Flash> Storage<F> {
    /// Reads the partition: areas that do not check are forgotten, and so are older versions.
    pub fn open(flash: F) -> Self {
        let mut s = Self {
            flash,
            areas: Vec::new(),
            next_gen: 1,
            cursor: 0,
        };
        let mut at = 0;
        while at + SECTOR <= s.flash.size() {
            let mut h = [0u8; HEADER];
            let found = s
                .flash
                .read(at, &mut h)
                .then(|| decode(&h))
                .flatten()
                .filter(|(meta, _)| at + area_size(meta.len) <= s.flash.size());
            match found {
                Some((meta, gen)) if s.image_crc(at, meta.len) == Some(meta.crc) => {
                    let area = Area { at, gen, meta };
                    at += area.size();
                    s.areas.push(area);
                }
                // A header over a broken image (power lost while it was being removed).
                Some(_) => {
                    s.forget(at);
                    at += SECTOR;
                }
                None => at += SECTOR,
            }
        }
        let stale: Vec<usize> = s
            .areas
            .iter()
            .filter(|a| {
                s.areas
                    .iter()
                    .any(|b| b.meta.name == a.meta.name && b.gen > a.gen)
            })
            .map(|a| a.at)
            .collect();
        for at in stale {
            s.remove(at);
        }
        if let Some(newest) = s.areas.iter().max_by_key(|a| a.gen) {
            s.next_gen = newest.gen.wrapping_add(1).max(1);
            s.cursor = newest.at + newest.size();
        }
        s
    }

    pub fn flash(&self) -> &F {
        &self.flash
    }

    /// The stored images in the order of their place in the partition.
    pub fn areas(&self) -> &[Area] {
        &self.areas
    }

    pub fn area(&self, at: usize) -> Option<&Area> {
        self.areas.iter().find(|a| a.at == at)
    }

    /// The newest image with this name.
    pub fn find(&self, name: &str) -> Option<&Area> {
        self.areas
            .iter()
            .filter(|a| a.meta.name == name)
            .max_by_key(|a| a.gen)
    }

    pub fn capacity(&self) -> usize {
        self.flash.size() / SECTOR * SECTOR
    }

    /// Runs of free sectors as (offset, bytes).
    fn gaps(&self) -> Vec<(usize, usize)> {
        let mut gaps = Vec::new();
        let mut at = 0;
        for area in &self.areas {
            if area.at > at {
                gaps.push((at, area.at - at));
            }
            at = area.at + area.size();
        }
        if self.capacity() > at {
            gaps.push((at, self.capacity() - at));
        }
        gaps
    }

    /// Bytes in free sectors.
    pub fn free(&self) -> usize {
        self.gaps().iter().map(|g| g.1).sum()
    }

    /// The longest image that fits now.
    pub fn largest(&self) -> usize {
        let run = self.gaps().iter().map(|g| g.1).max().unwrap_or(0);
        run.saturating_sub(HEADER)
    }

    /// Finds room for an image of `len` bytes; returns the offset of its area. Nothing is
    /// written: the caller erases the sectors ([`Storage::erase_sector`]), writes the image
    /// ([`Storage::write`]) and commits it ([`Storage::commit`]).
    pub fn reserve(&self, len: usize) -> Result<usize, StorageError> {
        let need = area_size(len);
        let mut first = None;
        for (at, size) in self.gaps() {
            // The part of the gap at or after the cursor is tried first.
            let from = at.max(self.cursor.min(at + size));
            if at + size - from >= need {
                return Ok(from);
            }
            if size >= need && first.is_none() {
                first = Some(at);
            }
        }
        first.ok_or(StorageError::NoSpace)
    }

    /// Bytes to erase before an image of `len` bytes is written.
    pub fn area_size(len: usize) -> usize {
        area_size(len)
    }

    /// Erases one sector of an area being written.
    pub fn erase_sector(&mut self, at: usize) -> Result<(), StorageError> {
        if self
            .areas
            .iter()
            .any(|a| at >= a.at && at < a.at + a.size())
        {
            return Err(StorageError::Flash);
        }
        self.flash
            .erase(at, SECTOR)
            .then_some(())
            .ok_or(StorageError::Flash)
    }

    /// Writes image bytes at `offset` of the image of the area at `at`.
    pub fn write(&mut self, at: usize, offset: usize, data: &[u8]) -> Result<(), StorageError> {
        self.flash
            .write(at + HEADER + offset, data)
            .then_some(())
            .ok_or(StorageError::Flash)
    }

    fn image_crc(&self, at: usize, len: usize) -> Option<u32> {
        let mut buf = [0u8; 256];
        let (mut crc, mut done) = (0, 0);
        while done < len {
            let n = (len - done).min(buf.len());
            if !self.flash.read(at + HEADER + done, &mut buf[..n]) {
                return None;
            }
            crc = crc32_update(crc, &buf[..n]);
            done += n;
        }
        Some(crc)
    }

    /// Checks the image written at `at` against `meta` and gives it a header. An older version
    /// with the same name stays until [`Storage::remove`]d.
    pub fn commit(&mut self, at: usize, meta: Meta) -> Result<&Area, StorageError> {
        if meta.name.is_empty()
            || meta.name.len() > MAX_NAME
            || meta.allow.len() > MAX_ALLOW
            || meta.len == 0
        {
            return Err(StorageError::Crc);
        }
        if self.image_crc(at, meta.len) != Some(meta.crc) {
            return Err(StorageError::Flash);
        }
        let gen = self.next_gen;
        if !self.flash.write(at, &encode(&meta, gen)) {
            return Err(StorageError::Flash);
        }
        self.next_gen = gen.wrapping_add(1).max(1);
        let area = Area { at, gen, meta };
        self.cursor = at + area.size();
        let pos = self.areas.partition_point(|a| a.at < at);
        self.areas.insert(pos, area);
        Ok(&self.areas[pos])
    }

    /// Zero bits need no erase: the header stops being one.
    fn forget(&mut self, at: usize) -> bool {
        self.flash.write(at, &[0u8; 4])
    }

    /// Forgets the area at `at`; its sectors are free again. Returns whether there was one.
    pub fn remove(&mut self, at: usize) -> bool {
        let Some(pos) = self.areas.iter().position(|a| a.at == at) else {
            return false;
        };
        self.forget(at);
        self.areas.remove(pos);
        true
    }
}

/// A partition in RAM that behaves like NOR flash, for tests on the desktop.
#[derive(Debug, Clone)]
pub struct RamFlash {
    pub bytes: Vec<u8>,
}

impl RamFlash {
    pub fn new(size: usize) -> Self {
        Self {
            bytes: alloc::vec![0xFF; size],
        }
    }
}

impl Flash for RamFlash {
    fn size(&self) -> usize {
        self.bytes.len()
    }

    fn read(&self, at: usize, buf: &mut [u8]) -> bool {
        match at
            .checked_add(buf.len())
            .and_then(|end| self.bytes.get(at..end))
        {
            Some(src) => {
                buf.copy_from_slice(src);
                true
            }
            None => false,
        }
    }

    fn write(&mut self, at: usize, data: &[u8]) -> bool {
        match at
            .checked_add(data.len())
            .and_then(|end| self.bytes.get_mut(at..end))
        {
            Some(dst) => {
                dst.iter_mut().zip(data).for_each(|(d, s)| *d &= s);
                true
            }
            None => false,
        }
    }

    fn erase(&mut self, at: usize, len: usize) -> bool {
        if !at.is_multiple_of(SECTOR) || !len.is_multiple_of(SECTOR) {
            return false;
        }
        match at
            .checked_add(len)
            .and_then(|end| self.bytes.get_mut(at..end))
        {
            Some(dst) => {
                dst.fill(0xFF);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str, image: &[u8]) -> Meta {
        Meta {
            name: String::from(name),
            len: image.len(),
            crc: crc32(image),
            mode: ExecutionMode::AotXip,
            memory: 4096,
            allow: String::from("gpio_write,millis"),
        }
    }

    /// The whole upload of one image; returns the offset of its area.
    fn put(s: &mut Storage<RamFlash>, name: &str, image: &[u8]) -> Result<usize, StorageError> {
        let at = s.reserve(image.len())?;
        for sector in (at..at + area_size(image.len())).step_by(SECTOR) {
            s.erase_sector(sector)?;
        }
        for (i, chunk) in image.chunks(1000).enumerate() {
            s.write(at, i * 1000, chunk)?;
        }
        s.commit(at, meta(name, image)).map(|a| a.at)
    }

    fn image(seed: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| (i as u8).wrapping_mul(7) ^ seed).collect()
    }

    fn read(s: &Storage<RamFlash>, area: &Area) -> Vec<u8> {
        let mut data = alloc::vec![0; area.meta.len];
        assert!(s.flash().read(area.data_at(), &mut data));
        data
    }

    #[test]
    fn crc32_matches_the_editor() {
        assert_eq!(crc32(b"hello"), 0x3610_a686);
        assert_eq!(crc32_update(crc32(b"he"), b"llo"), 0x3610_a686);
    }

    #[test]
    fn images_survive_a_restart_with_what_is_stored_about_them() {
        let mut s = Storage::open(RamFlash::new(16 * SECTOR));
        assert_eq!(s.free(), 16 * SECTOR);
        let a = image(1, 5000);
        let b = image(2, 100);
        let at_a = put(&mut s, "pump", &a).unwrap();
        let at_b = put(&mut s, "fan", &b).unwrap();
        assert_eq!((at_a, at_b), (0, 2 * SECTOR));
        assert_eq!(s.free(), 13 * SECTOR);

        let s = Storage::open(s.flash().clone());
        assert_eq!(s.areas().len(), 2);
        let pump = s.find("pump").unwrap();
        assert_eq!(pump.meta, meta("pump", &a));
        assert_eq!(pump.allow_list(), ["gpio_write", "millis"]);
        assert_eq!(read(&s, pump), a);
        assert_eq!(read(&s, s.find("fan").unwrap()), b);
        // The image starts aligned for AOT text.
        assert_eq!(pump.data_at() % 16, 0);
        assert!(s.find("valve").is_none());
    }

    #[test]
    fn a_new_version_lives_beside_the_old_one_until_it_is_removed() {
        let mut s = Storage::open(RamFlash::new(8 * SECTOR));
        let old = put(&mut s, "pump", &image(1, 3000)).unwrap();
        let new = put(&mut s, "pump", &image(2, 6000)).unwrap();
        assert_ne!(old, new);
        assert_eq!(s.find("pump").unwrap().at, new);
        assert_eq!(read(&s, s.area(old).unwrap()), image(1, 3000));

        // Power lost before the old one was removed: the newer wins.
        let reopened = Storage::open(s.flash().clone());
        assert_eq!(reopened.areas().len(), 1);
        assert_eq!(reopened.find("pump").unwrap().at, new);

        assert!(s.remove(old));
        assert!(!s.remove(old));
        assert_eq!(s.free(), 6 * SECTOR);
    }

    #[test]
    fn space_is_reused_and_overflow_is_refused() {
        let mut s = Storage::open(RamFlash::new(4 * SECTOR));
        let a = put(&mut s, "a", &image(1, SECTOR)).unwrap();
        let b = put(&mut s, "b", &image(2, SECTOR)).unwrap();
        assert_eq!(s.free(), 0);
        assert_eq!(s.largest(), 0);
        assert_eq!(put(&mut s, "c", &[1]), Err(StorageError::NoSpace));
        assert!(s.remove(a));
        assert_eq!(s.largest(), 2 * SECTOR - HEADER);
        // Too long for the gap, although exactly what `largest` says fits.
        assert_eq!(
            put(&mut s, "c", &image(3, 2 * SECTOR)),
            Err(StorageError::NoSpace)
        );
        let c = put(&mut s, "c", &image(3, 2 * SECTOR - HEADER)).unwrap();
        assert_eq!(c, a);
        assert_eq!(read(&s, s.area(b).unwrap()), image(2, SECTOR));
        assert_eq!(read(&s, s.area(c).unwrap()), image(3, 2 * SECTOR - HEADER));
    }

    #[test]
    fn writes_go_round_the_partition() {
        let mut s = Storage::open(RamFlash::new(4 * SECTOR));
        let first = put(&mut s, "a", &[1; 10]).unwrap();
        s.remove(first);
        // The freed sector is not taken again at once.
        let second = put(&mut s, "a", &[2; 10]).unwrap();
        assert_eq!((first, second), (0, SECTOR));
        let mut s = Storage::open(s.flash().clone());
        assert_eq!(put(&mut s, "b", &image(3, SECTOR)), Ok(2 * SECTOR));
        // Nothing is left after the newest area: back to the start.
        assert_eq!(s.reserve(10), Ok(0));
    }

    #[test]
    fn a_broken_upload_leaves_nothing() {
        let mut s = Storage::open(RamFlash::new(4 * SECTOR));
        let data = image(1, 3000);
        let at = s.reserve(data.len()).unwrap();
        s.erase_sector(at).unwrap();
        s.write(at, 0, &data[..2000]).unwrap();
        // The last chunk never came.
        assert_eq!(s.commit(at, meta("pump", &data)), Err(StorageError::Flash));
        assert!(s.areas().is_empty());
        assert!(Storage::open(s.flash().clone()).areas().is_empty());
        // A stored image is not erased under a running module.
        let at = put(&mut s, "pump", &data).unwrap();
        assert_eq!(s.erase_sector(at), Err(StorageError::Flash));
    }

    #[test]
    fn an_image_damaged_in_flash_is_forgotten() {
        let mut s = Storage::open(RamFlash::new(4 * SECTOR));
        let at = put(&mut s, "pump", &image(1, 3000)).unwrap();
        put(&mut s, "fan", &image(2, 10)).unwrap();
        let mut flash = s.flash().clone();
        flash.bytes[at + HEADER + 7] ^= 0x10;
        let s = Storage::open(flash);
        assert_eq!(s.areas().len(), 1);
        assert_eq!(s.areas()[0].meta.name, "fan");
        assert_eq!(s.free(), 3 * SECTOR);
        // Forgotten for good, not only skipped.
        assert_eq!(Storage::open(s.flash().clone()).free(), 3 * SECTOR);
    }
}
