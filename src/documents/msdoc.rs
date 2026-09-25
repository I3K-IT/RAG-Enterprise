//! Text of a Word binary document (.doc), Word 6.0 through 2003, read the
//! way the format describes itself — no converter, no external process.
//!
//! A .doc is an OLE compound file. Its `WordDocument` stream opens with the
//! File Information Block (FIB), whose version decides the rest:
//!
//! - **Word 97–2003** ([MS-DOC]): the FIB says which table stream (`0Table`
//!   or `1Table`) holds the piece table, and where. The piece table maps
//!   runs of character positions to where their text sits in
//!   `WordDocument`: 8-bit Windows-1252 for "compressed" pieces, UTF-16LE
//!   otherwise.
//! - **Word 6.0/95**: no Unicode and no separate table stream. The text is
//!   one 8-bit run between `fcMin` and `fcMac` — or, in a fast-saved file,
//!   pieces listed by a piece table inside `WordDocument` itself — in the
//!   code page of the document's fonts.
//!
//! Read in order, the pieces give the whole text — body first, then
//! footnotes, headers, comments and text boxes — with Word's structure
//! still inline as control characters, which [`clean`] turns into plain
//! text.
//!
//! Every offset below comes from the file, so every read is bounds-checked
//! and returns an error rather than panicking. The output is capped as
//! well: nothing stops two pieces from pointing at the same bytes, so a
//! small file could otherwise expand without limit — the same threat the
//! ZIP decompression-bomb check covers for .docx and .xlsx.

use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

/// `wIdent` values: Word 97–2003, then the two Word 6.0/95 wrote.
const WORD_IDENTS: [u16; 3] = [0xA5EC, 0xA5DC, 0xA699];
/// `nFib` of Word 97; Word 6.0 and 95 wrote 101–105, with an older FIB.
const NFIB_WORD97: u16 = 0x00C1;
const NFIB_WORD6: u16 = 0x0065;
/// `FibBase.flags` bits, at the same place in both FIBs.
const F_COMPLEX: u16 = 0x0004;
const F_ENCRYPTED: u16 = 0x0100;
const F_WHICH_TBL_STM: u16 = 0x0200;
/// Index of the (`fcClx`, `lcbClx`) pair within `FibRgFcLcb97`.
const CLX_PAIR: usize = 33;
/// Nesting deeper than this is not something Word writes; past it, fields
/// are still paired but no longer tracked one by one.
const MAX_FIELD_DEPTH: usize = 64;

/// The text of the .doc at `path`, refused if it would exceed `max_bytes`.
pub fn extract_text(path: &Path, max_bytes: u64) -> Result<String> {
    let file =
        std::fs::File::open(path).with_context(|| format!("opening doc {}", path.display()))?;
    // No stream in the container can legitimately be longer than the file.
    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut cfb = cfb::CompoundFile::open(file).with_context(|| {
        format!(
            "doc is not a readable OLE compound file: {}",
            path.display()
        )
    })?;

    let word = read_stream(&mut cfb, "WordDocument", file_len)?;
    if !WORD_IDENTS.contains(&u16_at(&word, 0)?) {
        bail!("doc: not a Word document (wrong FIB signature)");
    }
    let nfib = u16_at(&word, 2)?;
    let flags = u16_at(&word, 0x0A)?;
    if flags & F_ENCRYPTED != 0 {
        bail!("this .doc is password-protected, so its text cannot be read");
    }
    let raw = if nfib >= NFIB_WORD97 {
        let fib = Fib::parse(&word, flags)?;
        let table = read_stream(
            &mut cfb,
            if fib.table_1 { "1Table" } else { "0Table" },
            file_len,
        )?;
        let clx = slice(&table, fib.fc_clx as usize, fib.lcb_clx as usize)
            .context("doc: the piece table lies outside its table stream")?;
        text_from_pieces(&word, piece_table(clx)?, max_bytes)?
    } else if nfib >= NFIB_WORD6 {
        word6_text(&word, flags, max_bytes)?
    } else {
        bail!("this .doc was saved by a Word older than 6.0, which is not supported — save it as .docx");
    };
    Ok(clean(&raw))
}

fn read_stream<F: Read + std::io::Seek>(
    cfb: &mut cfb::CompoundFile<F>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    let stream = cfb
        .open_stream(name)
        .with_context(|| format!("doc has no {name} stream — not a Word document"))?;
    let mut buf = Vec::new();
    stream
        .take(limit)
        .read_to_end(&mut buf)
        .with_context(|| format!("reading the {name} stream"))?;
    Ok(buf)
}

/// The parts of a Word 97–2003 File Information Block this reader needs.
struct Fib {
    table_1: bool,
    fc_clx: u32,
    lcb_clx: u32,
}

impl Fib {
    fn parse(word: &[u8], flags: u16) -> Result<Self> {
        // FibBase is 32 bytes. After it, each variable part is preceded by
        // its own length: csw (u16 count) + fibRgW, cslw (u32 count) +
        // fibRgLw, cbRgFcLcb (pair count) + fibRgFcLcbBlob. Walking the
        // lengths rather than assuming Word 97's sizes keeps a file that
        // lies about them from sending a read somewhere else.
        let csw = u16_at(word, 0x20)? as usize;
        let cslw_at = 0x22 + csw * 2;
        let cslw = u16_at(word, cslw_at)? as usize;
        let pairs_at = cslw_at + 2 + cslw * 4;
        let pairs = u16_at(word, pairs_at)? as usize;
        if pairs <= CLX_PAIR {
            bail!("doc: the FIB is too short to locate the piece table");
        }
        let clx_at = pairs_at + 2 + CLX_PAIR * 8;
        Ok(Self {
            table_1: flags & F_WHICH_TBL_STM != 0,
            fc_clx: u32_at(word, clx_at)?,
            lcb_clx: u32_at(word, clx_at + 4)?,
        })
    }
}

/// The `PlcPcd` inside a `Clx`: zero or more `Prc` blocks (formatting,
/// skipped) followed by the one `Pcdt` that holds the piece table.
fn piece_table(clx: &[u8]) -> Result<&[u8]> {
    let mut at = 0usize;
    loop {
        match clx.get(at) {
            Some(0x01) => {
                let size = i16::from_le_bytes(array_at(clx, at + 1)?);
                if size < 0 {
                    bail!("doc: corrupt formatting block in the piece table");
                }
                at += 3 + size as usize;
            }
            Some(0x02) => {
                let len = u32_at(clx, at + 1)? as usize;
                return slice(clx, at + 5, len).context("doc: truncated piece table");
            }
            Some(other) => bail!("doc: unexpected block {other:#04x} in the piece table"),
            None => bail!("doc: no piece table"),
        }
    }
}

/// How many pieces a `PlcPcd` describes: n+1 u32 character positions
/// followed by n 8-byte descriptors, 12n + 4 bytes in all.
fn piece_count(plcpcd: &[u8]) -> Result<usize> {
    match plcpcd.len().checked_sub(4) {
        Some(rest) if rest.is_multiple_of(12) => Ok(rest / 12),
        _ => bail!("doc: malformed piece table"),
    }
}

/// Concatenates the text of every piece, in order. A `PlcPcd` is n+1
/// character positions (u32) followed by n 8-byte piece descriptors; bytes
/// 2..6 of each descriptor locate its text and say whether it is 8-bit.
fn text_from_pieces(word: &[u8], plcpcd: &[u8], max_bytes: u64) -> Result<String> {
    let n = piece_count(plcpcd)?;
    let mut out = String::new();
    for i in 0..n {
        let start = u32_at(plcpcd, i * 4)?;
        let end = u32_at(plcpcd, (i + 1) * 4)?;
        let chars = end
            .checked_sub(start)
            .context("doc: piece table out of order")? as usize;
        // Worst case for UTF-8 output is three bytes per UTF-16 unit or per
        // Windows-1252 byte, so this bound is checked before decoding.
        let ceiling = out.len() as u64 + chars as u64 * 3;
        if ceiling > max_bytes {
            bail!("doc refused: its text would pass the {max_bytes} byte limit");
        }
        let fc_compressed = u32_at(plcpcd, (n + 1) * 4 + i * 8 + 2)?;
        let fc = (fc_compressed & 0x3FFF_FFFF) as usize;
        if fc_compressed & 0x4000_0000 != 0 {
            let bytes = slice(word, fc / 2, chars)
                .context("doc: a text piece lies outside the document")?;
            let (text, _) = encoding_rs::WINDOWS_1252.decode_without_bom_handling(bytes);
            out.push_str(&text);
        } else {
            let bytes = slice(word, fc, chars * 2)
                .context("doc: a text piece lies outside the document")?;
            let units = bytes
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]));
            out.extend(char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')));
        }
    }
    Ok(out)
}

/// Word 6.0/95. The FIB fields used here sit at fixed offsets: `fcMin` and
/// `fcMac` bound the text of a normally saved file; a fast-saved
/// ("complex") one instead lists pieces in a piece table at `fcClx`, inside
/// `WordDocument` — there is no table stream yet — whose file positions are
/// plain byte offsets, one byte per character.
fn word6_text(word: &[u8], flags: u16, max_bytes: u64) -> Result<String> {
    let bytes: Vec<u8> = if flags & F_COMPLEX != 0 {
        let clx = slice(
            word,
            u32_at(word, 0x160)? as usize,
            u32_at(word, 0x164)? as usize,
        )
        .context("doc: the piece table lies outside the document")?;
        let plcpcd = piece_table(clx)?;
        let n = piece_count(plcpcd)?;
        let mut out = Vec::new();
        for i in 0..n {
            let chars = u32_at(plcpcd, (i + 1) * 4)?
                .checked_sub(u32_at(plcpcd, i * 4)?)
                .context("doc: piece table out of order")? as usize;
            if (out.len() + chars) as u64 * 3 > max_bytes {
                bail!("doc refused: its text would pass the {max_bytes} byte limit");
            }
            let fc = u32_at(plcpcd, (n + 1) * 4 + i * 8 + 2)? as usize;
            out.extend_from_slice(
                slice(word, fc, chars).context("doc: a text piece lies outside the document")?,
            );
        }
        out
    } else {
        let (fc_min, fc_mac) = (u32_at(word, 0x18)? as usize, u32_at(word, 0x1C)? as usize);
        let len = fc_mac
            .checked_sub(fc_min)
            .context("doc: text bounds out of order")?;
        if len as u64 * 3 > max_bytes {
            bail!("doc refused: its text would pass the {max_bytes} byte limit");
        }
        slice(word, fc_min, len)
            .context("doc: the text lies outside the document")?
            .to_vec()
    };
    let (text, _) = word6_code_page(word).decode_without_bom_handling(&bytes);
    Ok(text.into_owned())
}

/// Word 6.0/95 stores no Unicode, so the bytes are in whatever code page
/// the document was written in, and nothing says which directly. The font
/// table does: each font names its Windows character set. As Apache POI
/// does, the first font that is not Western, default or symbol decides;
/// with none, the text is Windows-1252. The table is a u16 length followed
/// by entries whose first byte is their own length minus one and whose
/// fifth is the character set.
fn word6_code_page(word: &[u8]) -> &'static encoding_rs::Encoding {
    use encoding_rs::WINDOWS_1252;
    let table = u32_at(word, 0xD0)
        .and_then(|at| Ok((at as usize, u32_at(word, 0xD4)? as usize)))
        .ok()
        .and_then(|(at, len)| slice(word, at, len));
    let Some(table) = table else {
        return WINDOWS_1252;
    };
    let mut at = 2;
    while let (Some(&len_m1), Some(&charset)) = (table.get(at), table.get(at + 4)) {
        if let Some(encoding) = super::codepage::by_charset(charset) {
            return encoding;
        }
        at += len_m1 as usize + 1;
    }
    WINDOWS_1252
}

/// Word keeps its structure inline, as control characters. This keeps what
/// a reader of the document sees and drops the rest.
///
/// Fields (hyperlinks, page numbers, cross-references…) are
/// `0x13 code 0x14 result 0x15`, possibly nested: the result is visible
/// text, the code is an instruction like `HYPERLINK "https://…"`.
fn clean(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    // One entry per open field: true while its code is being read, false
    // once its separator has shown up and its result follows.
    let mut fields: Vec<bool> = Vec::new();
    let mut in_code = 0usize;
    let mut untracked = 0usize;
    for c in raw.chars() {
        match c {
            '\u{13}' => {
                if fields.len() < MAX_FIELD_DEPTH {
                    fields.push(true);
                    in_code += 1;
                } else {
                    untracked += 1;
                }
                continue;
            }
            '\u{14}' => {
                if untracked == 0 {
                    if let Some(top) = fields.last_mut() {
                        if *top {
                            *top = false;
                            in_code -= 1;
                        }
                    }
                }
                continue;
            }
            '\u{15}' => {
                if untracked > 0 {
                    untracked -= 1;
                } else if fields.pop() == Some(true) {
                    in_code -= 1;
                }
                continue;
            }
            _ => {}
        }
        if in_code > 0 || untracked > 0 {
            continue;
        }
        match c {
            // paragraph end, line break, page or section break
            '\r' | '\u{0B}' | '\u{0C}' => out.push('\n'),
            // end of a table cell or row
            '\u{07}' => out.push('\t'),
            '\t' | '\n' => out.push(c),
            // non-breaking hyphen, non-breaking space
            '\u{1E}' => out.push('-'),
            '\u{A0}' => out.push(' '),
            // picture and object anchors, footnote and comment marks,
            // optional hyphens: nothing a reader sees
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

fn slice(buf: &[u8], at: usize, len: usize) -> Option<&[u8]> {
    buf.get(at..at.checked_add(len)?)
}

fn array_at<const N: usize>(buf: &[u8], at: usize) -> Result<[u8; N]> {
    slice(buf, at, N)
        .and_then(|b| b.try_into().ok())
        .context("doc: truncated structure")
}

fn u16_at(buf: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_le_bytes(array_at(buf, at)?))
}

fn u32_at(buf: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_le_bytes(array_at(buf, at)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Seek, SeekFrom, Write};

    /// A real Word 97 document, saved by LibreOffice 24.2 from HTML written
    /// for this test: accents, typographic symbols, text outside
    /// Windows-1252, a hyperlink (a field) and a table.
    const WORD97_DOC: &[u8] = include_bytes!("testdata/word97.doc");
    const LIMIT: u64 = 1 << 20;

    fn doc_file(bytes: &[u8]) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.doc");
        std::fs::write(&path, bytes).unwrap();
        (dir, path)
    }

    /// The fixture with its `WordDocument` stream edited in place.
    fn patched(edit: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
        let mut cfb = cfb::CompoundFile::open(Cursor::new(WORD97_DOC.to_vec())).unwrap();
        let mut word = Vec::new();
        cfb.open_stream("WordDocument")
            .unwrap()
            .read_to_end(&mut word)
            .unwrap();
        edit(&mut word);
        let mut stream = cfb.open_stream("WordDocument").unwrap();
        stream.seek(SeekFrom::Start(0)).unwrap();
        stream.write_all(&word).unwrap();
        drop(stream);
        cfb.flush().unwrap();
        cfb.into_inner().into_inner()
    }

    fn put_u16(buf: &mut [u8], at: usize, v: u16) {
        buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
    }

    fn put_u32(buf: &mut [u8], at: usize, v: u32) {
        buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    #[test]
    fn reads_a_real_word97_document() {
        let (_dir, path) = doc_file(WORD97_DOC);
        let text = extract_text(&path, LIMIT).unwrap();
        for expected in [
            "Legacy Word document",
            "città, perché, naïve — and symbols € ™ “quotes”",
            "Łódź, αβγ, Ω",
            "A link to the published specification inside a sentence.",
            "Quarter\tAmount",
            "Closing paragraph after the table.",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in {text:?}");
        }
        assert!(!text.contains("HYPERLINK"), "a field code leaked: {text:?}");
        assert!(
            !text.contains("example.com"),
            "a link target leaked: {text:?}"
        );
    }

    #[test]
    fn a_password_protected_document_is_refused_clearly() {
        let bytes = patched(|w| {
            let flags = u16::from_le_bytes([w[0x0A], w[0x0B]]);
            put_u16(w, 0x0A, flags | F_ENCRYPTED);
        });
        let (_dir, path) = doc_file(&bytes);
        let e = extract_text(&path, LIMIT).unwrap_err();
        assert!(e.to_string().contains("password-protected"), "{e:#}");
    }

    #[test]
    fn a_document_older_than_word_6_is_refused_clearly() {
        let bytes = patched(|w| put_u16(w, 2, 0x0030));
        let (_dir, path) = doc_file(&bytes);
        let e = extract_text(&path, LIMIT).unwrap_err();
        assert!(e.to_string().contains("older than 6.0"), "{e:#}");
    }

    #[test]
    fn a_stream_that_is_not_word_is_refused() {
        let bytes = patched(|w| put_u16(w, 0, 0x1234));
        let (_dir, path) = doc_file(&bytes);
        let e = extract_text(&path, LIMIT).unwrap_err();
        assert!(e.to_string().contains("not a Word document"), "{e:#}");
    }

    /// Every offset comes from the file: one that points nowhere must be an
    /// error, never an out-of-bounds panic.
    #[test]
    fn a_piece_table_pointing_outside_its_stream_is_an_error() {
        // fcClx/lcbClx of the fixture, at their Word 97 position.
        let bytes = patched(|w| put_u32(w, 0x01A6, u32::MAX));
        let (_dir, path) = doc_file(&bytes);
        let e = extract_text(&path, LIMIT).unwrap_err();
        assert!(e.to_string().contains("outside"), "{e:#}");
    }

    // ── fields and Word's control characters ───────────────────────────────

    #[test]
    fn a_field_shows_its_result_not_its_code() {
        let raw = "see \u{13} HYPERLINK \"https://example.com\" \u{14}the spec\u{15} now";
        assert_eq!(clean(raw), "see the spec now");
    }

    #[test]
    fn a_field_nested_in_another_field_s_code_stays_hidden() {
        // IF { PAGE } > 1 "yes": the inner result belongs to the outer code.
        let raw = "\u{13} IF \u{13} PAGE \u{14}3\u{15} > 1 \u{14}yes\u{15}";
        assert_eq!(clean(raw), "yes");
    }

    #[test]
    fn unbalanced_field_marks_do_not_derail_the_text() {
        assert_eq!(clean("a\u{14}b\u{15}c"), "abc");
        assert_eq!(clean("kept \u{13} never closed"), "kept ");
    }

    #[test]
    fn nesting_past_the_tracked_depth_still_pairs_up() {
        let deep = MAX_FIELD_DEPTH + 40;
        let raw = format!("{}{}visible", "\u{13}".repeat(deep), "\u{15}".repeat(deep));
        assert_eq!(clean(&raw), "visible");
    }

    #[test]
    fn word_marks_become_plain_text() {
        let raw = "a\rb\u{0B}c\u{0C}d\u{07}e\u{1E}f\u{A0}g\u{01}h\u{1F}i";
        assert_eq!(clean(raw), "a\nb\nc\nd\te-f ghi");
    }

    // ── the piece table ────────────────────────────────────────────────────

    /// A `PlcPcd` from (first character, last character + 1, FcCompressed).
    fn plcpcd(pieces: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (start, _, _) in pieces {
            out.extend_from_slice(&start.to_le_bytes());
        }
        out.extend_from_slice(&pieces.last().map_or(0, |p| p.1).to_le_bytes());
        for (_, _, fc) in pieces {
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(&fc.to_le_bytes());
            out.extend_from_slice(&[0, 0]);
        }
        out
    }

    #[test]
    fn compressed_and_unicode_pieces_are_both_decoded() {
        let mut word = vec![0u8; 0x300];
        word[0x100..0x106].copy_from_slice(b"Hi \x93q\x94"); // Windows-1252 curly quotes
        let omega: Vec<u8> = "Ω!".encode_utf16().flat_map(u16::to_le_bytes).collect();
        word[0x200..0x204].copy_from_slice(&omega);
        let table = plcpcd(&[(0, 6, (0x100 * 2) | 0x4000_0000), (6, 8, 0x200)]);
        assert_eq!(text_from_pieces(&word, &table, LIMIT).unwrap(), "Hi “q”Ω!");
    }

    #[test]
    fn a_piece_outside_the_document_is_an_error() {
        let word = vec![0u8; 0x100];
        let table = plcpcd(&[(0, 10, 0x0FFF_FFFF)]);
        assert!(text_from_pieces(&word, &table, LIMIT).is_err());
    }

    #[test]
    fn pieces_out_of_order_are_an_error() {
        let word = vec![0u8; 0x100];
        let mut table = plcpcd(&[(5, 9, 0)]);
        table[4..8].copy_from_slice(&2u32.to_le_bytes()); // ends before it starts
        assert!(text_from_pieces(&word, &table, LIMIT).is_err());
    }

    /// Pieces may point at the same bytes; a small file must not be able to
    /// ask for more text than the limit that way.
    #[test]
    fn overlapping_pieces_cannot_expand_past_the_limit() {
        let word = vec![b'x'; 1000];
        let pieces: Vec<_> = (0..500u32)
            .map(|i| (i * 1000, (i + 1) * 1000, 0x4000_0000))
            .collect();
        let e = text_from_pieces(&word, &plcpcd(&pieces), 100_000).unwrap_err();
        assert!(e.to_string().contains("byte limit"), "{e:#}");
    }

    #[test]
    fn formatting_blocks_before_the_piece_table_are_skipped() {
        let mut clx = vec![0x01, 0x02, 0x00, 0xAA, 0xBB, 0x02];
        clx.extend_from_slice(&3u32.to_le_bytes());
        clx.extend_from_slice(b"pcd");
        assert_eq!(piece_table(&clx).unwrap(), b"pcd");
        assert!(piece_table(&[0x01, 0x00, 0x00]).is_err(), "no Pcdt at all");
        assert!(piece_table(&[0x07]).is_err(), "unknown block");
    }

    // ── Word 6.0/95 ────────────────────────────────────────────────────────
    // LibreOffice can no longer write this format, so these build the few
    // FIB fields the reader uses by hand. The reader itself was checked
    // against the 14 Word 6.0/95 files in Apache POI's test corpus —
    // Cyrillic, Czech and fast-saved ones included — all read.

    fn word6(text_at: usize, text: &[u8]) -> Vec<u8> {
        let mut w = vec![0u8; 0x400];
        put_u32(&mut w, 0x18, text_at as u32);
        put_u32(&mut w, 0x1C, (text_at + text.len()) as u32);
        w[text_at..text_at + text.len()].copy_from_slice(text);
        w
    }

    #[test]
    fn word6_text_is_the_run_between_fc_min_and_fc_mac() {
        let w = word6(0x300, b"Hello Word 6\r");
        assert_eq!(clean(&word6_text(&w, 0, LIMIT).unwrap()), "Hello Word 6\n");
    }

    #[test]
    fn word6_code_page_comes_from_the_font_table() {
        // "Привет" in Windows-1251, and a font table whose one font declares
        // the Cyrillic character set (204).
        let mut w = word6(0x300, &[0xCF, 0xF0, 0xE8, 0xE2, 0xE5, 0xF2]);
        let font_table = [0x0A, 0x00, 0x06, 0, 0, 0, 204, 0, b'A', 0];
        w[0x200..0x20A].copy_from_slice(&font_table);
        put_u32(&mut w, 0xD0, 0x200);
        put_u32(&mut w, 0xD4, font_table.len() as u32);
        assert_eq!(word6_text(&w, 0, LIMIT).unwrap(), "Привет");
    }

    #[test]
    fn a_fast_saved_word6_file_is_read_through_its_piece_table() {
        let mut w = word6(0x300, b"second first ");
        let mut clx = vec![0x02];
        let table = plcpcd(&[(0, 6, 0x307), (6, 13, 0x300)]);
        clx.extend_from_slice(&(table.len() as u32).to_le_bytes());
        clx.extend_from_slice(&table);
        w[0x200..0x200 + clx.len()].copy_from_slice(&clx);
        put_u32(&mut w, 0x160, 0x200);
        put_u32(&mut w, 0x164, clx.len() as u32);
        assert_eq!(word6_text(&w, F_COMPLEX, LIMIT).unwrap(), "first second ");
    }
}
