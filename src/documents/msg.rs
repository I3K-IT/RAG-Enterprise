//! Outlook messages (.msg), read the way [MS-OXMSG] describes them: an OLE
//! compound file with one stream per property, named after it —
//! `__substg1.0_0037001F` is the subject (property 0x0037) as a UTF-16
//! string (type 0x001F). Fixed-size properties, such as dates, share one
//! stream instead. Recipients and attachments are storages of the same
//! shape; a message forwarded as an attachment is a whole message inside
//! one, and is read in turn.
//!
//! The body is the plain-text one when there is one; otherwise the HTML
//! body, or the RTF body, which Outlook keeps compressed. Attached
//! documents are read as an .eml's are (see eml.rs).

use std::collections::HashMap;
use std::io::{Read, Seek};
use std::path::Path;

use anyhow::{bail, Context, Result};
use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};

use super::codepage;
use super::eml::{attachment_extension, mailbox, read_attachment, Budget, Letter, MAX_DEPTH};

// Property ids ([MS-OXPROPS]).
const SUBJECT: u16 = 0x0037;
const TRANSPORT_HEADERS: u16 = 0x007D;
const CLIENT_SUBMIT_TIME: u16 = 0x0039;
const SENT_REPRESENTING_NAME: u16 = 0x0042;
const SENT_REPRESENTING_EMAIL: u16 = 0x0065;
const SENDER_NAME: u16 = 0x0C1A;
const SENDER_EMAIL: u16 = 0x0C1F;
const RECIPIENT_TYPE: u16 = 0x0C15;
const DISPLAY_CC: u16 = 0x0E03;
const DISPLAY_TO: u16 = 0x0E04;
const MESSAGE_DELIVERY_TIME: u16 = 0x0E06;
const BODY: u16 = 0x1000;
const RTF_COMPRESSED: u16 = 0x1009;
const BODY_HTML: u16 = 0x1013;
const DISPLAY_NAME: u16 = 0x3001;
const EMAIL_ADDRESS: u16 = 0x3003;
const ATTACH_DATA: u16 = 0x3701;
const ATTACH_FILENAME: u16 = 0x3704;
const ATTACH_METHOD: u16 = 0x3705;
const ATTACH_LONG_FILENAME: u16 = 0x3707;
const ATTACH_MIME_TAG: u16 = 0x370E;
const SMTP_ADDRESS: u16 = 0x39FE;
const INTERNET_CPID: u16 = 0x3FDE;
const MESSAGE_LOCALE_ID: u16 = 0x3FF1;
const MESSAGE_CODEPAGE: u16 = 0x3FFD;
const SENDER_SMTP_ADDRESS: u16 = 0x5D01;

const PROPERTIES: &str = "__properties_version1.0";
/// The storage an attachment keeps a whole message in.
const EMBEDDED_MESSAGE: &str = "__substg1.0_3701000D";
/// `PR_ATTACH_METHOD` of an attachment that is a whole message.
const ATTACH_EMBEDDED_MSG: u32 = 5;
/// Where the fixed-size properties start in the properties stream: after
/// a header whose size depends on what the storage holds.
const TOP_LEVEL_HEADER: usize = 32;
const EMBEDDED_HEADER: usize = 24;
const CHILD_HEADER: usize = 8;

pub fn extract_text(path: &Path, data_dir: &Path) -> Result<String> {
    extract_at(path, data_dir, 0, &Budget::new())
}

/// `extract_text` for a message found `depth` messages deep, drawing on
/// the `budget` of the upload it came in.
pub(crate) fn extract_at(
    path: &Path,
    data_dir: &Path,
    depth: usize,
    budget: &Budget,
) -> Result<String> {
    let file =
        std::fs::File::open(path).with_context(|| format!("opening msg {}", path.display()))?;
    // No stream in the container can legitimately be longer than the file.
    let limit = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut ole = cfb::CompoundFile::open(file).with_context(|| {
        format!(
            "msg is not a readable OLE compound file: {}",
            path.display()
        )
    })?;
    if !ole.is_stream(Path::new("/").join(PROPERTIES)) {
        bail!("not an Outlook message: it has no {PROPERTIES} stream");
    }
    let mut out = Vec::new();
    Msg {
        ole: &mut ole,
        limit,
        data_dir,
        budget,
    }
    .read_message(Path::new("/"), TOP_LEVEL_HEADER, depth, &mut out)?;
    out.retain(|part| !part.is_empty());
    Ok(out.join("\n\n"))
}

struct Msg<'a, F> {
    ole: &'a mut cfb::CompoundFile<F>,
    limit: u64,
    /// Where attached documents are copied to be read.
    data_dir: &'a Path,
    /// Every stream read is drawn from it: streams can share their bytes.
    budget: &'a Budget,
}

impl<F: Read + Seek> Msg<'_, F> {
    fn read_message(
        &mut self,
        dir: &Path,
        header: usize,
        depth: usize,
        out: &mut Vec<String>,
    ) -> Result<()> {
        let fixed = self.fixed_properties(dir, header);
        let encoding = self.encoding(dir, &fixed);

        let mut letter = Letter {
            subject: self.string(dir, SUBJECT, encoding),
            date: [CLIENT_SUBMIT_TIME, MESSAGE_DELIVERY_TIME]
                .into_iter()
                .find_map(|id| fixed.get(&id).and_then(|value| filetime(u64_of(value)))),
            ..Default::default()
        };
        let name = self
            .string(dir, SENDER_NAME, encoding)
            .or_else(|| self.string(dir, SENT_REPRESENTING_NAME, encoding));
        let address = [SENDER_SMTP_ADDRESS, SENDER_EMAIL, SENT_REPRESENTING_EMAIL]
            .into_iter()
            .find_map(|id| self.string(dir, id, encoding).filter(|a| a.contains('@')));
        letter.from = mailbox(name.as_deref(), address.as_deref());
        // Before the attachments, which could spend the budget.
        letter.body = self.body(dir, &fixed, encoding)?;

        let mut forwarded = Vec::new();
        for (entry, is_storage) in self.entries(dir)? {
            if !is_storage {
                continue;
            }
            let child = dir.join(&entry);
            if entry.starts_with("__recip_version1.0_") {
                let kind = self
                    .fixed_properties(&child, CHILD_HEADER)
                    .get(&RECIPIENT_TYPE)
                    .map(u32_of);
                let name = self.string(&child, DISPLAY_NAME, encoding);
                let address = [SMTP_ADDRESS, EMAIL_ADDRESS].into_iter().find_map(|id| {
                    self.string(&child, id, encoding)
                        .filter(|a| a.contains('@'))
                });
                let who = mailbox(name.as_deref(), address.as_deref());
                match kind {
                    Some(1) => letter.to.extend(who),
                    Some(2) => letter.cc.extend(who),
                    _ => {}
                }
            } else if entry.starts_with("__attach_version1.0_") {
                let name = [ATTACH_LONG_FILENAME, ATTACH_FILENAME, DISPLAY_NAME]
                    .into_iter()
                    .find_map(|id| {
                        self.string(&child, id, encoding)
                            .filter(|n| !n.trim().is_empty())
                    });
                let method = self
                    .fixed_properties(&child, CHILD_HEADER)
                    .get(&ATTACH_METHOD)
                    .map(u32_of);
                let embedded = child.join(EMBEDDED_MESSAGE);
                // An embedded OLE object keeps its storage under the same
                // name, so the method decides when it says anything.
                if matches!(method, None | Some(ATTACH_EMBEDDED_MSG))
                    && self.ole.is_storage(&embedded)
                {
                    if depth < MAX_DEPTH {
                        forwarded.push(embedded);
                    }
                } else {
                    // Only the data of what will be read is.
                    let mime = self.string(&child, ATTACH_MIME_TAG, encoding);
                    let ext = attachment_extension(name.as_deref(), mime.as_deref());
                    if let Some((ext, bytes)) =
                        ext.and_then(|ext| Some((ext, self.binary(&child, ATTACH_DATA)?)))
                    {
                        let label = name.clone().unwrap_or_else(|| format!("attachment.{ext}"));
                        let text = if ext == "txt" {
                            // In whatever encoding its author's system used.
                            Some(decode_text(&bytes, encoding))
                        } else {
                            read_attachment(&label, &ext, &bytes, self.data_dir, depth, self.budget)
                        };
                        letter.attached_text.extend(text.map(|text| (label, text)));
                    }
                }
                letter.attachments.extend(name);
            }
        }
        // Without recipient storages, the display lists are all there is.
        let display = |list: Option<String>| -> Vec<String> {
            list.map(|l| {
                l.split(';')
                    .map(|n| n.trim().to_owned())
                    .filter(|n| !n.is_empty())
                    .collect()
            })
            .unwrap_or_default()
        };
        if letter.to.is_empty() {
            letter.to = display(self.string(dir, DISPLAY_TO, encoding));
        }
        if letter.cc.is_empty() {
            letter.cc = display(self.string(dir, DISPLAY_CC, encoding));
        }

        out.push(letter.render());
        for message in forwarded {
            self.read_message(&message, EMBEDDED_HEADER, depth + 1, out)?;
        }
        Ok(())
    }

    /// The code page of the message's 8-bit strings, found as Apache POI
    /// finds it: the message's code page, or failing that its language's,
    /// or the charset its Internet headers declare.
    fn encoding(&mut self, dir: &Path, fixed: &HashMap<u16, [u8; 8]>) -> &'static Encoding {
        if let Some(encoding) = fixed
            .get(&MESSAGE_CODEPAGE)
            .and_then(|value| codepage::by_number(u32_of(value)))
        {
            return encoding;
        }
        if let Some(lcid) = fixed.get(&MESSAGE_LOCALE_ID).map(u32_of) {
            return codepage::by_locale(lcid);
        }
        let headers = self
            .string(dir, TRANSPORT_HEADERS, WINDOWS_1252)
            .unwrap_or_default();
        headers
            .lines()
            .filter(|line| line.to_ascii_lowercase().starts_with("content-type"))
            .find_map(|line| {
                let at = line.to_ascii_lowercase().find("charset=")? + "charset=".len();
                let label = line[at..]
                    .trim_start_matches('"')
                    .split(['"', ';', ' '])
                    .next()?;
                Encoding::for_label(label.as_bytes())
            })
            .unwrap_or(WINDOWS_1252)
    }

    fn body(
        &mut self,
        dir: &Path,
        fixed: &HashMap<u16, [u8; 8]>,
        encoding: &'static Encoding,
    ) -> Result<String> {
        // The bodies are in the Internet code page, when there is one —
        // except that an 8-bit plain-text body never is in UTF-8, whatever
        // that says: Outlook writes it in the message's own code page.
        let internet = fixed
            .get(&INTERNET_CPID)
            .and_then(|value| codepage::by_number(u32_of(value)));
        let text_encoding = internet.filter(|&e| e != UTF_8).unwrap_or(encoding);
        if let Some(text) = self
            .string(dir, BODY, text_encoding)
            .filter(|t| !t.trim().is_empty())
        {
            return Ok(text);
        }
        let html = self.binary(dir, BODY_HTML).map(|bytes| {
            let charset = internet.unwrap_or(if std::str::from_utf8(&bytes).is_ok() {
                UTF_8
            } else {
                encoding
            });
            charset.decode_without_bom_handling(&bytes).0.into_owned()
        });
        if let Some(html) = html.or_else(|| self.string(dir, BODY_HTML, encoding)) {
            return Ok(super::html::to_text(&html));
        }
        if let Some(compressed) = self.binary(dir, RTF_COMPRESSED) {
            if let Some(rtf) = decompress_rtf(&compressed, self.budget)? {
                return super::rtf::text_of(&rtf);
            }
        }
        Ok(String::new())
    }

    /// The names of the entries in storage `dir`, and whether each is a
    /// storage, in name order.
    fn entries(&mut self, dir: &Path) -> Result<Vec<(String, bool)>> {
        let mut entries: Vec<(String, bool)> = self
            .ole
            .read_storage(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .map(|entry| (entry.name().to_owned(), entry.is_storage()))
            .collect();
        entries.sort();
        Ok(entries)
    }

    fn stream(&mut self, path: &Path) -> Option<Vec<u8>> {
        let len = self.ole.entry(path).ok()?.len().min(self.limit);
        if !self.budget.take(len) {
            return None;
        }
        let stream = self.ole.open_stream(path).ok()?;
        let mut bytes = Vec::new();
        stream.take(self.limit).read_to_end(&mut bytes).ok()?;
        Some(bytes)
    }

    /// A string property, stored as UTF-16 or in the message's code page.
    fn string(&mut self, dir: &Path, id: u16, encoding: &'static Encoding) -> Option<String> {
        let text = if let Some(bytes) = self.stream(&dir.join(format!("__substg1.0_{id:04X}001F")))
        {
            let units = bytes
                .chunks_exact(2)
                .map(|u| u16::from_le_bytes([u[0], u[1]]));
            char::decode_utf16(units)
                .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
                .collect::<String>()
        } else {
            let bytes = self.stream(&dir.join(format!("__substg1.0_{id:04X}001E")))?;
            encoding.decode_without_bom_handling(&bytes).0.into_owned()
        };
        Some(text.trim_end_matches('\0').to_owned())
    }

    fn binary(&mut self, dir: &Path, id: u16) -> Option<Vec<u8>> {
        self.stream(&dir.join(format!("__substg1.0_{id:04X}0102")))
    }

    /// The fixed-size properties of storage `dir`: 16-byte entries — type,
    /// id, flags, then an 8-byte value — after a header of `header` bytes.
    fn fixed_properties(&mut self, dir: &Path, header: usize) -> HashMap<u16, [u8; 8]> {
        let bytes = self.stream(&dir.join(PROPERTIES)).unwrap_or_default();
        bytes
            .get(header..)
            .unwrap_or_default()
            .chunks_exact(16)
            .map(|entry| {
                let id = u16::from_le_bytes([entry[2], entry[3]]);
                (id, entry[8..16].try_into().expect("8 bytes"))
            })
            .collect()
    }
}

/// A text file's bytes: UTF-8 or UTF-16 with a byte-order mark, else UTF-8
/// if they read as UTF-8, else `fallback`.
fn decode_text(bytes: &[u8], fallback: &'static Encoding) -> String {
    let encoding = match Encoding::for_bom(bytes) {
        Some((encoding, _)) => encoding,
        None if std::str::from_utf8(bytes).is_ok() => UTF_8,
        None => fallback,
    };
    encoding.decode(bytes).0.into_owned()
}

fn u32_of(value: &[u8; 8]) -> u32 {
    u32::from_le_bytes([value[0], value[1], value[2], value[3]])
}

fn u64_of(value: &[u8; 8]) -> u64 {
    u64::from_le_bytes(*value)
}

/// A Windows FILETIME — 100 ns ticks since 1601 — as RFC 3339, in UTC.
fn filetime(ticks: u64) -> Option<String> {
    const UNIX_EPOCH_IN_TICKS: u64 = 116_444_736_000_000_000;
    let since_epoch = ticks.checked_sub(UNIX_EPOCH_IN_TICKS)?;
    let seconds = i64::try_from(since_epoch / 10_000_000).ok()?;
    let nanos = (since_epoch % 10_000_000) as u32 * 100;
    chrono::DateTime::from_timestamp(seconds, nanos)
        .map(|date| date.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

/// The dictionary compressed RTF starts from ([MS-OXRTFCP] 2.1.3.1.1).
const RTF_PRELOAD: &[u8] = b"{\\rtf1\\ansi\\mac\\deff0\\deftab720{\\fonttbl;}{\\f0\\fnil \\froman \\fswiss \\fmodern \\fscript \\fdecor MS Sans SerifSymbolArialTimes New RomanCourier{\\colortbl\\red0\\green0\\blue0\r\n\\par \\pard\\plain\\f0\\fs20\\b\\i\\u\\tab\\tx";
/// `compType` of compressed and of stored RTF.
const LZFU: u32 = 0x7546_5A4C;
const MELA: u32 = 0x414C_454D;

/// Decompresses Outlook's compressed RTF ([MS-OXRTFCP]): LZ77 over a 4 KiB
/// dictionary. A stream can declare any size, and expand to eight times its
/// own, so the output stops at the smaller of the two — drawn from `budget`
/// before anything is written: `None` when not that much is left.
fn decompress_rtf(data: &[u8], budget: &Budget) -> Result<Option<Vec<u8>>> {
    let header = |at: usize| {
        data.get(at..at + 4)
            .map(|b| u32::from_le_bytes(b.try_into().expect("4 bytes")))
    };
    let (Some(raw_size), Some(kind)) = (header(4), header(8)) else {
        bail!("msg: the compressed RTF body is truncated");
    };
    let input = data.get(16..).unwrap_or_default();
    // A literal is one byte in and out; a reference two bytes in and at
    // most 17 out, eight of them to a control byte.
    let size = match kind {
        MELA => input.len(),
        LZFU => input.len().saturating_mul(8),
        _ => bail!("msg: the RTF body is compressed in an unknown way"),
    }
    .min(raw_size as usize);
    if !budget.take(size as u64) {
        return Ok(None);
    }
    if kind == MELA {
        return Ok(Some(input[..size].to_vec()));
    }
    let mut dictionary = [0u8; 4096];
    dictionary[..RTF_PRELOAD.len()].copy_from_slice(RTF_PRELOAD);
    let mut write = RTF_PRELOAD.len();
    let mut out = Vec::with_capacity(size);
    let mut bytes = input.iter().copied();
    'stream: while let Some(control) = bytes.next() {
        for bit in 0..8 {
            if out.len() == size {
                break 'stream;
            }
            if control & (1 << bit) == 0 {
                let Some(byte) = bytes.next() else {
                    break 'stream;
                };
                out.push(byte);
                dictionary[write] = byte;
                write = (write + 1) % 4096;
            } else {
                let (Some(high), Some(low)) = (bytes.next(), bytes.next()) else {
                    break 'stream;
                };
                let reference = u16::from_be_bytes([high, low]);
                let offset = usize::from(reference >> 4);
                if offset == write {
                    break 'stream;
                }
                let run = (usize::from(reference & 0x0F) + 2).min(size - out.len());
                for i in 0..run {
                    let byte = dictionary[(offset + i) % 4096];
                    out.push(byte);
                    dictionary[write] = byte;
                    write = (write + 1) % 4096;
                }
            }
        }
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn the_preloaded_dictionary_is_the_specified_one() {
        assert_eq!(RTF_PRELOAD.len(), 207);
    }

    #[test]
    fn compressed_rtf_decompresses() {
        // The example from [MS-OXRTFCP] 3.1.1.
        let compressed: [u8; 49] = [
            0x2d, 0x00, 0x00, 0x00, 0x2b, 0x00, 0x00, 0x00, 0x4c, 0x5a, 0x46, 0x75, 0xf1, 0xc5,
            0xc7, 0xa7, 0x03, 0x00, 0x0a, 0x00, 0x72, 0x63, 0x70, 0x67, 0x31, 0x32, 0x35, 0x42,
            0x32, 0x0a, 0xf3, 0x20, 0x68, 0x65, 0x6c, 0x09, 0x00, 0x20, 0x62, 0x77, 0x05, 0xb0,
            0x6c, 0x64, 0x7d, 0x0a, 0x80, 0x0f, 0xa0,
        ];
        let rtf = decompress_rtf(&compressed, &Budget::new())
            .unwrap()
            .unwrap();
        assert_eq!(rtf, b"{\\rtf1\\ansi\\ansicpg1252\\pard hello world}\r\n");
        assert_eq!(super::super::rtf::text_of(&rtf).unwrap(), "hello world");
    }

    /// A compressed RTF header declaring `raw_size`, then `chunks` control
    /// bytes each followed by eight references of the longest run.
    fn crafted_rtf(raw_size: u32, chunks: usize) -> Vec<u8> {
        let mut data = vec![0x00, 0x00, 0x00, 0x00];
        data.extend(raw_size.to_le_bytes());
        data.extend(LZFU.to_le_bytes());
        data.extend([0; 4]);
        let chunk: Vec<u8> = std::iter::once(0xFF)
            .chain([0x00, 0x0F].repeat(8))
            .collect();
        data.extend(chunk.repeat(chunks));
        data
    }

    #[test]
    fn a_crafted_compressed_body_stays_within_its_declared_size() {
        let data = crafted_rtf(16, 1000);
        let budget = Budget::of(1000);
        assert_eq!(decompress_rtf(&data, &budget).unwrap().unwrap().len(), 16);
        assert!(budget.take(1000 - 16), "only the declared size was taken");
        assert!(decompress_rtf(&data[..6], &Budget::new()).is_err());
    }

    #[test]
    fn a_compressed_body_is_drawn_from_the_budget_before_it_expands() {
        // 17 bytes a chunk, each expanding to 136: eight times the input,
        // whatever size the header claims. (Few enough chunks that the
        // write position never meets the references' offset, 0, which
        // would end the stream.)
        let data = crafted_rtf(u32::MAX, 400);
        let expands_to = 136 * 400;
        let budget = Budget::of(expands_to);
        let rtf = decompress_rtf(&data, &budget).unwrap().unwrap();
        assert_eq!(rtf.len() as u64, expands_to);
        assert!(!budget.take(1), "all of it was taken");
        assert_eq!(
            decompress_rtf(&data, &Budget::of(expands_to - 1)).unwrap(),
            None,
            "refused before anything is written"
        );
    }

    fn utf16(text: &str) -> Vec<u8> {
        text.encode_utf16().flat_map(u16::to_le_bytes).collect()
    }

    /// A properties stream: `header` bytes, then one PT_LONG or PT_SYSTIME
    /// entry per (id, type, value).
    fn properties(header: usize, entries: &[(u16, u16, u64)]) -> Vec<u8> {
        let mut out = vec![0; header];
        for (id, kind, value) in entries {
            out.extend(kind.to_le_bytes());
            out.extend(id.to_le_bytes());
            out.extend([0; 4]);
            out.extend(value.to_le_bytes());
        }
        out
    }

    fn write(ole: &mut cfb::CompoundFile<std::fs::File>, path: &str, bytes: &[u8]) {
        ole.create_stream(path).unwrap().write_all(bytes).unwrap();
    }

    #[test]
    fn a_message_with_recipients_and_a_forwarded_one_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.msg");
        {
            let mut ole = cfb::CompoundFile::create(std::fs::File::create(&path).unwrap()).unwrap();
            // 2026-09-01T07:30:00Z as a FILETIME.
            let sent = 116_444_736_000_000_000 + 1_788_247_800 * 10_000_000;
            write(
                &mut ole,
                "/__properties_version1.0",
                &properties(32, &[(CLIENT_SUBMIT_TIME, 0x0040, sent)]),
            );
            write(
                &mut ole,
                "/__substg1.0_0037001F",
                &utf16("Riunione di lunedì\0"),
            );
            write(&mut ole, "/__substg1.0_0C1A001F", &utf16("Mario Rossi"));
            write(
                &mut ole,
                "/__substg1.0_5D01001F",
                &utf16("mario@example.com"),
            );
            write(
                &mut ole,
                "/__substg1.0_1000001E",
                b"La riunione \xe8 alle 10.\0",
            );
            for (i, (name, address, kind)) in [
                ("Anna", "anna@example.com", 1u64),
                ("Luca", "luca@example.com", 2),
            ]
            .into_iter()
            .enumerate()
            {
                let recipient = format!("/__recip_version1.0_#{i:08X}");
                ole.create_storage(&recipient).unwrap();
                write(
                    &mut ole,
                    &format!("{recipient}/__properties_version1.0"),
                    &properties(8, &[(RECIPIENT_TYPE, 0x0003, kind)]),
                );
                write(
                    &mut ole,
                    &format!("{recipient}/__substg1.0_3001001F"),
                    &utf16(name),
                );
                write(
                    &mut ole,
                    &format!("{recipient}/__substg1.0_39FE001F"),
                    &utf16(address),
                );
            }
            ole.create_storage("/__attach_version1.0_#00000000")
                .unwrap();
            write(
                &mut ole,
                "/__attach_version1.0_#00000000/__substg1.0_3707001F",
                &utf16("Original.msg"),
            );
            ole.create_storage("/__attach_version1.0_#00000001")
                .unwrap();
            write(
                &mut ole,
                "/__attach_version1.0_#00000001/__substg1.0_3707001F",
                &utf16("notes.txt"),
            );
            write(
                &mut ole,
                "/__attach_version1.0_#00000001/__substg1.0_37010102",
                b"Notes in a file.",
            );
            let inner = "/__attach_version1.0_#00000000/__substg1.0_3701000D";
            ole.create_storage(inner).unwrap();
            write(
                &mut ole,
                &format!("{inner}/__properties_version1.0"),
                &properties(24, &[]),
            );
            write(
                &mut ole,
                &format!("{inner}/__substg1.0_0037001F"),
                &utf16("Original"),
            );
            write(
                &mut ole,
                &format!("{inner}/__substg1.0_10130102"),
                b"<p>The <b>original</b> text.</p>",
            );
            ole.flush().unwrap();
        }
        assert_eq!(
            extract_text(&path, dir.path()).unwrap(),
            "Subject: Riunione di lunedì\nFrom: Mario Rossi <mario@example.com>\nTo: Anna <anna@example.com>\n\
             Cc: Luca <luca@example.com>\nDate: 2026-09-01T07:30:00Z\nAttachments: Original.msg, notes.txt\n\n\
             La riunione è alle 10.\n\nAttachment: notes.txt\nNotes in a file.\n\nSubject: Original\n\nThe original text."
        );
    }

    #[test]
    fn an_ole_file_that_is_not_a_message_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.msg");
        {
            let mut ole = cfb::CompoundFile::create(std::fs::File::create(&path).unwrap()).unwrap();
            write(&mut ole, "/WordDocument", b"not mail");
            ole.flush().unwrap();
        }
        assert!(extract_text(&path, dir.path())
            .unwrap_err()
            .to_string()
            .contains("not an Outlook message"));
    }

    #[test]
    fn a_spent_budget_skips_attachments_not_the_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.msg");
        {
            let mut ole = cfb::CompoundFile::create(std::fs::File::create(&path).unwrap()).unwrap();
            write(&mut ole, "/__properties_version1.0", &properties(32, &[]));
            write(&mut ole, "/__substg1.0_0037001F", &utf16("Big"));
            write(&mut ole, "/__substg1.0_1000001F", &utf16("The body."));
            ole.create_storage("/__attach_version1.0_#00000000")
                .unwrap();
            write(
                &mut ole,
                "/__attach_version1.0_#00000000/__substg1.0_3707001F",
                &utf16("notes.txt"),
            );
            write(
                &mut ole,
                "/__attach_version1.0_#00000000/__substg1.0_37010102",
                &[b'x'; 5000],
            );
            ole.flush().unwrap();
        }
        let text = extract_at(&path, dir.path(), 0, &Budget::of(1000)).unwrap();
        assert!(text.contains("The body."), "{text}");
        assert!(!text.contains("Attachment: notes.txt"), "{text}");
        assert!(extract_text(&path, dir.path())
            .unwrap()
            .contains("Attachment: notes.txt"));
    }

    #[test]
    fn attached_documents_are_read_but_not_pictures_or_ole_objects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.msg");
        let odt = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/documents/testdata/book.odt"
        ))
        .unwrap();
        {
            let mut ole = cfb::CompoundFile::create(std::fs::File::create(&path).unwrap()).unwrap();
            write(&mut ole, "/__properties_version1.0", &properties(32, &[]));
            write(&mut ole, "/__substg1.0_0037001F", &utf16("Documents"));
            // By value (method 1): a document, then a picture.
            for (i, (name, data)) in [("book.odt", &odt[..]), ("logo.png", b"\x89PNG not really")]
                .into_iter()
                .enumerate()
            {
                let attachment = format!("/__attach_version1.0_#{i:08X}");
                ole.create_storage(&attachment).unwrap();
                write(
                    &mut ole,
                    &format!("{attachment}/__properties_version1.0"),
                    &properties(8, &[(ATTACH_METHOD, 0x0003, 1)]),
                );
                write(
                    &mut ole,
                    &format!("{attachment}/__substg1.0_3707001F"),
                    &utf16(name),
                );
                write(
                    &mut ole,
                    &format!("{attachment}/__substg1.0_37010102"),
                    data,
                );
            }
            // An embedded OLE object (method 6): its storage has the name
            // an embedded message's has, and is not one.
            let object = "/__attach_version1.0_#00000002";
            ole.create_storage(object).unwrap();
            write(
                &mut ole,
                &format!("{object}/__properties_version1.0"),
                &properties(8, &[(ATTACH_METHOD, 0x0003, 6)]),
            );
            write(
                &mut ole,
                &format!("{object}/__substg1.0_3707001F"),
                &utf16("Chart"),
            );
            ole.create_storage(format!("{object}/{EMBEDDED_MESSAGE}"))
                .unwrap();
            write(
                &mut ole,
                &format!("{object}/{EMBEDDED_MESSAGE}/CONTENTS"),
                b"object data",
            );
            ole.flush().unwrap();
        }
        let text = extract_text(&path, dir.path()).unwrap();
        assert!(
            text.starts_with("Subject: Documents\nAttachments: book.odt, logo.png, Chart\n"),
            "{text}"
        );
        assert!(
            text.contains("\n\nAttachment: book.odt\nOpenDocument fixture\n"),
            "{text}"
        );
        assert!(
            !text.contains("Attachment: logo.png"),
            "pictures are not read: {text}"
        );
        assert_eq!(
            text.matches("Subject:").count(),
            1,
            "an OLE object is not a message: {text}"
        );
    }
}
