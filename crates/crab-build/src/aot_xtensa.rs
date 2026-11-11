//! Pre-linking of XIP AOT images for classic ESP32 (Xtensa).
//!
//! wamrc leaves the `L32R` instructions of an Xtensa image unresolved: every one of them has an
//! `R_XTENSA_SLOT0_OP` relocation against `.literal` in the `.rela.text` group. WAMR refuses any text
//! relocation in an `--xip` image on a microcontroller (the code runs straight from flash and is
//! never written), so the master could not load such a file. The distance from an `L32R` to its
//! literal does not depend on where the image lands, only on the address modulo 4, and the master
//! keeps images 4-aligned in flash: so the editor writes the offsets into the code here and drops
//! the `.rela.text` group. The result loads like a RISC-V image, with no text relocations.

/// Section type of the code (`u32 literal_size, literal, code`).
const SECTION_TEXT: u32 = 2;
/// Section type of the relocations.
const SECTION_RELOCATION: u32 = 5;
/// `R_XTENSA_SLOT0_OP`: the only relocation `L32R` gets.
const R_XTENSA_SLOT0_OP: u32 = 20;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PrelinkError {
    #[error("AOT-образ для ESP32 повреждён: {0}")]
    Malformed(&'static str),
    #[error(
        "AOT-образ для ESP32 содержит перемещение, которое редактор не умеет вписать \
         (тип {kind}, символ «{symbol}»)"
    )]
    Unsupported { kind: u32, symbol: String },
    #[error(
        "AOT-образ для ESP32 содержит перемещения кода «{0}», которые редактор не умеет вписать"
    )]
    UnsupportedGroup(String),
    #[error("литерал слишком далеко от команды L32R (смещение {0}); модуль слишком большой")]
    OutOfRange(i64),
}

fn malformed(what: &'static str) -> PrelinkError {
    PrelinkError::Malformed(what)
}

fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, PrelinkError> {
    b.get(at..at + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
        .ok_or(malformed("файл обрывается"))
}

/// One relocation group as found in the file.
struct Group {
    /// File offset of the group (its `name_index`), and of the end of its last entry.
    start: usize,
    end: usize,
    name: String,
    entries: usize,
}

/// Symbol names of the relocation section, by index.
fn symbols(b: &[u8], body: usize) -> Result<(Vec<String>, usize), PrelinkError> {
    let count = u32_at(b, body)? as usize;
    let mut at = body + 4;
    let offsets_at = at;
    at = at
        .checked_add(count.checked_mul(4).ok_or(malformed("число символов"))?)
        .ok_or(malformed("число символов"))?;
    let strings_len = u32_at(b, at)? as usize;
    at += 4;
    let strings = b
        .get(at..at + strings_len)
        .ok_or(malformed("таблица имён обрывается"))?;
    let mut names = Vec::with_capacity(count);
    for i in 0..count {
        let off = u32_at(b, offsets_at + 4 * i)? as usize;
        let len = strings
            .get(off..off + 2)
            .map(|s| u16::from_le_bytes([s[0], s[1]]) as usize)
            .ok_or(malformed("имя символа вне таблицы"))?;
        let raw = strings
            .get(off + 2..off + 2 + len)
            .ok_or(malformed("имя символа вне таблицы"))?;
        let raw = raw.strip_suffix(&[0]).unwrap_or(raw);
        names.push(String::from_utf8_lossy(raw).into_owned());
    }
    Ok((names, at + strings_len))
}

/// Pre-links `image` (see the module docs). An image without a `.rela.text` group is returned as is.
pub fn prelink(image: &[u8]) -> Result<Vec<u8>, PrelinkError> {
    if !image.starts_with(b"\0aot") {
        return Err(malformed("нет заголовка \\0aot"));
    }
    let mut out = image.to_vec();

    // Find the text and relocation sections.
    let mut text: Option<(usize, usize)> = None; // (body, size)
    let mut reloc: Option<(usize, usize)> = None; // (section header, size)
    let mut at = 8;
    while align4(at) < image.len() {
        at = align4(at);
        let kind = u32_at(image, at)?;
        let size = u32_at(image, at + 4)? as usize;
        let end = (at + 8)
            .checked_add(size)
            .filter(|&e| e <= image.len())
            .ok_or(malformed("секция выходит за конец файла"))?;
        match kind {
            SECTION_TEXT => text = Some((at + 8, size)),
            SECTION_RELOCATION => reloc = Some((at, size)),
            _ => {}
        }
        at = end;
    }
    let Some((reloc_header, reloc_size)) = reloc else {
        return Ok(out);
    };
    let reloc_body = reloc_header + 8;
    let reloc_end = reloc_body + reloc_size;

    let (names, after_strings) = symbols(image, reloc_body)?;
    let groups_at = align4(after_strings);
    let group_count = u32_at(image, groups_at)? as usize;
    let mut groups = Vec::with_capacity(group_count);
    let mut at = groups_at + 4;
    for _ in 0..group_count {
        at = align4(at);
        let name_index = u32_at(image, at)? as usize;
        let entries = u32_at(image, at + 4)? as usize;
        let end = entries
            .checked_mul(16)
            .and_then(|n| n.checked_add(at + 8))
            .filter(|&e| e <= reloc_end)
            .ok_or(malformed("группа перемещений выходит за секцию"))?;
        let name = names
            .get(name_index)
            .ok_or(malformed("неизвестное имя группы"))?
            .clone();
        groups.push(Group {
            start: at,
            end,
            name,
            entries,
        });
        at = end;
    }

    // Other groups WAMR treats as text relocations (and refuses in an XIP image).
    if let Some(g) = groups.iter().find(|g| {
        [".rel.text", ".rel.ltext", ".rela.ltext", ".rela.literal"].contains(&g.name.as_str())
    }) {
        return Err(PrelinkError::UnsupportedGroup(g.name.clone()));
    }
    let Some(pos) = groups.iter().position(|g| g.name == ".rela.text") else {
        return Ok(out);
    };
    let group = &groups[pos];
    let (text_body, text_size) = text.ok_or(malformed("нет секции кода"))?;
    let literal = text_body + 4;
    let literal_size = u32_at(image, text_body)? as usize;
    let code = literal + literal_size;
    let code_end = text_body + text_size;
    if code > code_end {
        return Err(malformed("размер литералов больше секции кода"));
    }

    for i in 0..group.entries {
        let e = group.start + 8 + 16 * i;
        let offset = u32_at(image, e)? as usize;
        let addend = u32_at(image, e + 4)? as i32;
        let kind = u32_at(image, e + 8)?;
        let symbol = names
            .get(u32_at(image, e + 12)? as usize)
            .ok_or(malformed("неизвестный символ перемещения"))?;
        if kind != R_XTENSA_SLOT0_OP || symbol != ".literal" {
            return Err(PrelinkError::Unsupported {
                kind,
                symbol: symbol.clone(),
            });
        }
        let insn = code + offset;
        if insn + 3 > code_end {
            return Err(malformed("перемещение вне кода"));
        }
        // L32R: op0 (low nibble of the first byte) is 1.
        if image[insn] & 0x0f != 0x01 {
            return Err(malformed("перемещение SLOT0_OP указывает не на L32R"));
        }
        let target = literal as i64 + i64::from(addend);
        if target & 3 != 0 {
            return Err(malformed("литерал не выровнен"));
        }
        let relative = target - ((insn as i64 + 3) & !3);
        if !(-256 * 1024..=-4).contains(&relative) {
            return Err(PrelinkError::OutOfRange(relative));
        }
        let imm16 = ((relative >> 2) as i16).to_le_bytes();
        out[insn + 1] = imm16[0];
        out[insn + 2] = imm16[1];
    }

    // Drop the group: everything after it moves by a multiple of 4, so alignment stays.
    let removed = group.end - group.start;
    out.drain(group.start..group.end);
    out[groups_at..groups_at + 4].copy_from_slice(&(group_count as u32 - 1).to_le_bytes());
    let new_size = (reloc_size - removed) as u32;
    out[reloc_header + 4..reloc_header + 8].copy_from_slice(&new_size.to_le_bytes());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(v: &mut Vec<u8>, x: u32) {
        v.extend_from_slice(&x.to_le_bytes());
    }

    fn pad(v: &mut Vec<u8>) {
        while !v.len().is_multiple_of(4) {
            v.push(0);
        }
    }

    /// One relocation: (offset, addend, type, symbol index).
    type Entry = (u32, i32, u32, u32);

    /// Relocation section body with `groups` of (name index, entries).
    fn reloc_body(names: &[&str], groups: &[(u32, Vec<Entry>)]) -> Vec<u8> {
        let mut strings = Vec::new();
        let mut offsets = Vec::new();
        for n in names {
            offsets.push(strings.len() as u32);
            strings.extend_from_slice(&((n.len() + 1) as u16).to_le_bytes());
            strings.extend_from_slice(n.as_bytes());
            strings.push(0);
        }
        let mut b = Vec::new();
        put(&mut b, names.len() as u32);
        for o in offsets {
            put(&mut b, o);
        }
        put(&mut b, strings.len() as u32);
        b.extend_from_slice(&strings);
        pad(&mut b);
        put(&mut b, groups.len() as u32);
        for (name, entries) in groups {
            pad(&mut b);
            put(&mut b, *name);
            put(&mut b, entries.len() as u32);
            for &(off, add, ty, sym) in entries {
                put(&mut b, off);
                put(&mut b, add as u32);
                put(&mut b, ty);
                put(&mut b, sym);
            }
        }
        b
    }

    fn section(v: &mut Vec<u8>, kind: u32, body: &[u8]) {
        pad(v);
        put(v, kind);
        put(v, body.len() as u32);
        v.extend_from_slice(body);
    }

    /// Image: 8 bytes of literals, code with two L32R (at code offsets 1 and 5), relocation section
    /// with `.rela.text` (both L32R) and `.rela.data` (kept), then a custom section after it.
    fn sample(rela_text_type: u32) -> Vec<u8> {
        let mut v = b"\0aot".to_vec();
        put(&mut v, 5);
        section(&mut v, 0, &[1, 2, 3, 4]);
        let mut text = Vec::new();
        put(&mut text, 8);
        text.extend_from_slice(&[0xaa; 8]);
        text.extend_from_slice(&[0x00, 0x21, 0x00, 0x00, 0x00, 0x31, 0x00, 0x00, 0x00, 0x00]);
        section(&mut v, 2, &text);
        let body = reloc_body(
            &[".rela.text", ".literal", ".rela.data", "func0"],
            &[
                (
                    0,
                    vec![(1, 0, rela_text_type, 1), (5, 4, rela_text_type, 1)],
                ),
                (2, vec![(0, 0, 1, 3)]),
            ],
        );
        section(&mut v, 5, &body);
        section(&mut v, 100, b"tail");
        v
    }

    /// Sections of an image as (type, body).
    fn sections(b: &[u8]) -> Vec<(u32, Vec<u8>)> {
        let mut out = Vec::new();
        let mut at = 8;
        while align4(at) < b.len() {
            at = align4(at);
            let kind = u32_at(b, at).unwrap();
            let size = u32_at(b, at + 4).unwrap() as usize;
            out.push((kind, b[at + 8..at + 8 + size].to_vec()));
            at += 8 + size;
        }
        out
    }

    #[test]
    fn prelinks_l32r_and_drops_group() {
        let input = sample(R_XTENSA_SLOT0_OP);
        let out = prelink(&input).unwrap();
        let secs = sections(&out);
        assert_eq!(secs.iter().map(|s| s.0).collect::<Vec<_>>(), [0, 2, 5, 100]);
        assert_eq!(secs[3].1, b"tail");

        // Text body starts at file offset 8 + 8 + 4 + 8 = 28: literal at 32, code at 40.
        let text = &secs[1].1;
        let code = &text[12..];
        // L32R at file 41 -> pc base (44) & !3 = 44, target 32: -12 -> imm16 -3.
        assert_eq!(&code[1..4], &[0x21, 0xfd, 0xff]);
        // L32R at file 45 -> base 48, target 36: -12 -> -3.
        assert_eq!(&code[5..8], &[0x31, 0xfd, 0xff]);

        let reloc = &secs[2].1;
        let expected = reloc_body(
            &[".rela.text", ".literal", ".rela.data", "func0"],
            &[(2, vec![(0, 0, 1, 3)])],
        );
        assert_eq!(reloc, &expected);
        // Prelinking again changes nothing.
        assert_eq!(prelink(&out).unwrap(), out);
    }

    #[test]
    fn unknown_text_relocation_is_refused() {
        let err = prelink(&sample(1)).unwrap_err();
        assert_eq!(
            err,
            PrelinkError::Unsupported {
                kind: 1,
                symbol: ".literal".into()
            }
        );
    }

    #[test]
    fn image_without_relocations_is_unchanged() {
        let mut v = b"\0aot".to_vec();
        put(&mut v, 5);
        section(&mut v, 0, &[1, 2, 3]);
        section(&mut v, 2, &[0, 0, 0, 0, 1, 2]);
        assert_eq!(prelink(&v).unwrap(), v);
    }

    #[test]
    fn truncated_image_is_reported() {
        let input = sample(R_XTENSA_SLOT0_OP);
        assert!(matches!(
            prelink(&input[..input.len() - 30]),
            Err(PrelinkError::Malformed(_))
        ));
        assert!(prelink(b"junk").is_err());
    }

    /// Real wamrc output kept by the developer (scratchpad copy), when present.
    #[test]
    fn real_image_when_present() {
        let Some(path) = std::env::var_os("CRAB_XTENSA_AOT_SAMPLE") else {
            return;
        };
        let image = std::fs::read(path).unwrap();
        let out = prelink(&image).unwrap();
        assert!(out.len() < image.len() || out == image);
        assert_eq!(prelink(&out).unwrap(), out);
    }
}
