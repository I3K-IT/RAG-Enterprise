//! Text of a PowerPoint 97–2003 presentation (.ppt), read the way [MS-PPT]
//! describes it — no converter, no external process.
//!
//! A .ppt is an OLE compound file whose `PowerPoint Document` stream is a
//! sequence of records. PowerPoint saves incrementally: an edit appends new
//! versions of the records it changed and leaves the old ones in place, so
//! reading every record in the stream would bring deleted text back. The
//! current state is found the way PowerPoint finds it: the `Current User`
//! stream points to the latest edit; each edit points to the one before it
//! and to a persist directory, which says where the latest version of each
//! object lives; and the document object reached through it lists the
//! slides in presentation order.
//!
//! Each slide contributes the text of its placeholders — title, body —
//! which the document keeps in its slide list, then the text boxes drawn on
//! the slide, then its speaker notes. Every offset comes from the file, so
//! every read is bounds-checked, and each object is read at most once.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use anyhow::{bail, Context, Result};

// Record types ([MS-PPT] 2.13.24).
const RT_DOCUMENT: u16 = 0x03E8;
const RT_SLIDE: u16 = 0x03EE;
const RT_SLIDE_ATOM: u16 = 0x03EF;
const RT_NOTES: u16 = 0x03F0;
const RT_SLIDE_PERSIST_ATOM: u16 = 0x03F3;
const RT_DRAWING: u16 = 0x040C;
const RT_TEXT_CHARS_ATOM: u16 = 0x0FA0;
const RT_TEXT_BYTES_ATOM: u16 = 0x0FA8;
const RT_SLIDE_LIST_WITH_TEXT: u16 = 0x0FF0;
const RT_USER_EDIT_ATOM: u16 = 0x0FF5;
const RT_PERSIST_DIRECTORY_ATOM: u16 = 0x1772;
/// `headerToken` of the Current User record of an encrypted presentation.
const ENCRYPTED_TOKEN: u32 = 0xF3D1_C4DF;

pub fn extract_text(path: &Path) -> Result<String> {
    let file =
        std::fs::File::open(path).with_context(|| format!("opening ppt {}", path.display()))?;
    // No stream in the container can legitimately be longer than the file.
    let file_len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let mut cfb = cfb::CompoundFile::open(file).with_context(|| {
        format!(
            "ppt is not a readable OLE compound file: {}",
            path.display()
        )
    })?;
    let current_user = read_stream(&mut cfb, "Current User", file_len)?;
    let document = read_stream(&mut cfb, "PowerPoint Document", file_len)?;
    text_of(&current_user, &document)
}

fn read_stream<F: Read + std::io::Seek>(
    cfb: &mut cfb::CompoundFile<F>,
    name: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    let stream = cfb.open_stream(name).with_context(|| {
        format!("ppt has no {name} stream — not a PowerPoint 97–2003 presentation")
    })?;
    let mut buf = Vec::new();
    stream
        .take(limit)
        .read_to_end(&mut buf)
        .with_context(|| format!("reading the {name} stream"))?;
    Ok(buf)
}

fn text_of(current_user: &[u8], stream: &[u8]) -> Result<String> {
    // PowerPoint 95 wrote a shorter Current User record, and a different
    // document stream behind it.
    if current_user.len() < 28 {
        bail!("this .ppt was saved by PowerPoint 95 or earlier, which is not supported — save it as .pptx");
    }
    if u32_at(current_user, 12)? == ENCRYPTED_TOKEN {
        bail!("this .ppt is password-protected, so its text cannot be read");
    }
    let (objects, document_id) = persist_directory(stream, u32_at(current_user, 16)? as usize)?;
    let object = |id: u32, kind: u16| {
        objects
            .get(&id)
            .and_then(|&offset| record_at(stream, offset))
            .filter(|r| r.kind == kind)
    };
    let document =
        object(document_id, RT_DOCUMENT).context("ppt: the document record is missing")?;

    // The slide lists: SlideListWithText records of instance 0 for the
    // slides, 1 for the masters, 2 for the notes. In the slide list, each
    // SlidePersistAtom opens a slide, and the text atoms after it are that
    // slide's placeholders. The notes list maps each notes page's id — which
    // is how a slide names its notes — to its persist object.
    let mut slides: Vec<(u32, Vec<String>)> = Vec::new();
    let mut notes_pages: HashMap<u32, u32> = HashMap::new();
    for list in records(document.data).filter(|r| r.kind == RT_SLIDE_LIST_WITH_TEXT) {
        for record in records(list.data) {
            match (list.instance, record.kind) {
                (0, RT_SLIDE_PERSIST_ATOM) => slides.push((u32_at(record.data, 0)?, Vec::new())),
                (0, _) => {
                    if let (Some(text), Some((_, texts))) = (text_atom(&record), slides.last_mut())
                    {
                        texts.push(text);
                    }
                }
                (2, RT_SLIDE_PERSIST_ATOM) => {
                    notes_pages.insert(u32_at(record.data, 12)?, u32_at(record.data, 0)?);
                }
                _ => {}
            }
        }
    }

    let mut read = HashSet::new();
    let mut out = Vec::new();
    for (id, mut texts) in slides {
        if !read.insert(id) {
            continue;
        }
        if let Some(slide) = object(id, RT_SLIDE) {
            drawing_text(slide.data, &mut texts);
            let notes_id = records(slide.data)
                .find(|r| r.kind == RT_SLIDE_ATOM)
                .and_then(|atom| u32_at(atom.data, 16).ok())
                .and_then(|notes| notes_pages.get(&notes).copied())
                .filter(|&notes| read.insert(notes));
            if let Some(notes) = notes_id.and_then(|notes| object(notes, RT_NOTES)) {
                drawing_text(notes.data, &mut texts);
            }
        }
        out.push(super::html::tidy(&clean(&texts.join("\n"))));
    }
    out.retain(|slide| !slide.is_empty());
    Ok(out.join("\n\n"))
}

/// Follows the chain of edits from the latest back to the first and
/// returns where the latest version of each persist object lives, with the
/// id of the document object.
fn persist_directory(stream: &[u8], latest_edit: usize) -> Result<(HashMap<u32, usize>, u32)> {
    let mut objects = HashMap::new();
    let mut document = None;
    let mut edit = latest_edit;
    let mut seen = HashSet::new();
    // An edit pointing back to one already read would loop forever.
    while edit != 0 && seen.insert(edit) {
        let atom = record_at(stream, edit)
            .filter(|r| r.kind == RT_USER_EDIT_ATOM && r.data.len() >= 28)
            .context("ppt: the chain of edits is broken — not a readable PowerPoint document")?;
        // Only an encrypted presentation has the field after the 28th byte.
        if atom.data.len() > 28 {
            bail!("this .ppt is password-protected, so its text cannot be read");
        }
        document.get_or_insert(u32_at(atom.data, 16)?);
        let directory = record_at(stream, u32_at(atom.data, 12)? as usize)
            .filter(|r| r.kind == RT_PERSIST_DIRECTORY_ATOM)
            .context("ppt: an edit's persist directory is missing")?;
        // Runs of offsets, each preceded by the id of its first object (20
        // bits) and the run's length (12 bits). An older edit never
        // overrides a newer one.
        let mut entries = directory.data;
        while let Some(info) = entries.get(..4) {
            let info = u32::from_le_bytes(info.try_into().expect("4 bytes"));
            let (first, count) = (info & 0x000F_FFFF, (info >> 20) as usize);
            let run = entries.get(4..).unwrap_or_default();
            for (i, offset) in run.chunks_exact(4).take(count).enumerate() {
                let offset = u32::from_le_bytes(offset.try_into().expect("4 bytes")) as usize;
                objects.entry(first + i as u32).or_insert(offset);
            }
            entries = run.get(count * 4..).unwrap_or_default();
        }
        edit = u32_at(atom.data, 8)? as usize;
    }
    let document = document
        .context("ppt: the chain of edits is empty — not a readable PowerPoint document")?;
    Ok((objects, document))
}

/// Appends the text of every text atom in `container`'s drawing — its text
/// boxes, and the placeholders that keep their own text — in document order.
fn drawing_text(container: &[u8], texts: &mut Vec<String>) {
    let Some(drawing) = records(container).find(|r| r.kind == RT_DRAWING) else {
        return;
    };
    // An explicit stack rather than recursion: containers can be nested as
    // deep as the file is long.
    let mut stack = vec![drawing.data];
    while let Some(rest) = stack.last_mut() {
        let Some((record, after)) = next_record(rest) else {
            stack.pop();
            continue;
        };
        *rest = after;
        if record.container {
            stack.push(record.data);
        } else if let Some(text) = text_atom(&record) {
            texts.push(text);
        }
    }
}

/// The text a TextCharsAtom (UTF-16LE) or TextBytesAtom (the low bytes of
/// UTF-16, i.e. Latin-1) holds.
fn text_atom(record: &Record<'_>) -> Option<String> {
    match record.kind {
        RT_TEXT_CHARS_ATOM => Some(
            char::decode_utf16(
                record
                    .data
                    .chunks_exact(2)
                    .map(|u| u16::from_le_bytes([u[0], u[1]])),
            )
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
        ),
        RT_TEXT_BYTES_ATOM => Some(record.data.iter().map(|&b| char::from(b)).collect()),
        _ => None,
    }
}

/// PowerPoint's paragraph (`\r`) and line (`\v`) breaks become newlines;
/// other control characters are dropped.
fn clean(raw: &str) -> String {
    raw.chars()
        .filter_map(|c| match c {
            '\r' | '\u{0B}' => Some('\n'),
            '\t' | '\n' => Some(c),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

struct Record<'a> {
    kind: u16,
    instance: u16,
    container: bool,
    data: &'a [u8],
}

/// The record at `offset`, if its header and all its data lie inside
/// `stream`.
fn record_at(stream: &[u8], offset: usize) -> Option<Record<'_>> {
    next_record(stream.get(offset..)?).map(|(record, _)| record)
}

/// The first record of `data` and what follows it.
fn next_record(data: &[u8]) -> Option<(Record<'_>, &[u8])> {
    let header = data.get(..8)?;
    let ver_instance = u16::from_le_bytes([header[0], header[1]]);
    let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
    let end = len.checked_add(8)?;
    let record = Record {
        kind: u16::from_le_bytes([header[2], header[3]]),
        instance: ver_instance >> 4,
        container: ver_instance & 0x000F == 0x000F,
        data: data.get(8..end)?,
    };
    Some((record, &data[end..]))
}

/// The records `data` holds one after another, up to the first that does
/// not fit.
fn records(data: &[u8]) -> impl Iterator<Item = Record<'_>> {
    let mut rest = data;
    std::iter::from_fn(move || {
        let (record, after) = next_record(rest)?;
        rest = after;
        Some(record)
    })
}

fn u32_at(data: &[u8], at: usize) -> Result<u32> {
    let bytes = data
        .get(at..at + 4)
        .context("ppt: a record is shorter than its format requires")?;
    Ok(u32::from_le_bytes(bytes.try_into().expect("4 bytes")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record: header, then `data`.
    fn rec(ver_instance: u16, kind: u16, data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend(ver_instance.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend((data.len() as u32).to_le_bytes());
        out.extend(data);
        out
    }

    fn container(kind: u16, instance: u16, children: &[Vec<u8>]) -> Vec<u8> {
        rec(0x000F | instance << 4, kind, &children.concat())
    }

    fn chars(text: &str) -> Vec<u8> {
        rec(
            0,
            RT_TEXT_CHARS_ATOM,
            &text
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        )
    }

    fn bytes(text: &str) -> Vec<u8> {
        rec(
            0,
            RT_TEXT_BYTES_ATOM,
            &text.chars().map(|c| c as u8).collect::<Vec<_>>(),
        )
    }

    /// A SlidePersistAtom (or NotesPersistAtom): the persist id, then the
    /// slide's (or notes page's) own id at byte 12.
    fn slide_persist(id: u32, own_id: u32) -> Vec<u8> {
        let mut data = id.to_le_bytes().to_vec();
        data.extend([0; 8]);
        data.extend(own_id.to_le_bytes());
        data.extend([0; 4]);
        rec(0, RT_SLIDE_PERSIST_ATOM, &data)
    }

    /// A slide (or notes) container whose drawing holds one text box, and
    /// whose SlideAtom names its notes page by the notes page's own id.
    fn slide(kind: u16, notes_id: u32, text_box: &str) -> Vec<u8> {
        let mut atom = vec![0; 16];
        atom.extend(notes_id.to_le_bytes());
        atom.extend([0; 4]);
        let drawing = container(
            RT_DRAWING,
            0,
            &[container(
                0xF002,
                0,
                &[container(
                    0xF004,
                    0,
                    &[container(0xF00D, 0, &[chars(text_box)])],
                )],
            )],
        );
        container(kind, 0, &[rec(2, RT_SLIDE_ATOM, &atom), drawing])
    }

    struct Deck {
        stream: Vec<u8>,
        latest_edit: usize,
    }

    impl Deck {
        /// Appends an edit that places `objects` (id, record) and points
        /// back to the previous edit, as PowerPoint's incremental save does.
        fn save(&mut self, document_id: u32, objects: &[(u32, Vec<u8>)]) {
            let mut directory = Vec::new();
            for (id, record) in objects {
                directory.extend((id | 1 << 20).to_le_bytes());
                directory.extend((self.stream.len() as u32).to_le_bytes());
                self.stream.extend(record);
            }
            let directory_at = self.stream.len() as u32;
            self.stream
                .extend(rec(0, RT_PERSIST_DIRECTORY_ATOM, &directory));
            let mut edit = vec![0; 8];
            edit.extend((self.latest_edit as u32).to_le_bytes());
            edit.extend(directory_at.to_le_bytes());
            edit.extend(document_id.to_le_bytes());
            edit.extend([0; 8]);
            self.latest_edit = self.stream.len();
            self.stream.extend(rec(0, RT_USER_EDIT_ATOM, &edit));
        }

        fn current_user(&self) -> Vec<u8> {
            let mut data = vec![0; 28];
            data[12..16].copy_from_slice(&0xE391_C05Fu32.to_le_bytes());
            data[16..20].copy_from_slice(&(self.latest_edit as u32).to_le_bytes());
            data
        }
    }

    /// A document listing `slides` (persist id, title) and `notes` (persist
    /// id, notes id).
    fn document(slides: &[(u32, &str)], notes: &[(u32, u32)]) -> Vec<u8> {
        let mut list = Vec::new();
        for (i, (id, title)) in slides.iter().enumerate() {
            list.push(slide_persist(*id, 256 + i as u32));
            list.push(bytes(title));
        }
        let notes: Vec<Vec<u8>> = notes
            .iter()
            .map(|(id, notes_id)| slide_persist(*id, *notes_id))
            .collect();
        container(
            RT_DOCUMENT,
            0,
            &[
                container(RT_SLIDE_LIST_WITH_TEXT, 0, &list),
                container(RT_SLIDE_LIST_WITH_TEXT, 2, &notes),
            ],
        )
    }

    #[test]
    fn slides_follow_the_slide_list_with_boxes_and_notes() {
        let mut deck = Deck {
            stream: Vec::new(),
            latest_edit: 0,
        };
        deck.save(
            1,
            &[
                (
                    1,
                    document(
                        &[(3, "Second file, first slide"), (2, "Città\rdi Roma")],
                        &[(4, 0x100)],
                    ),
                ),
                (2, slide(RT_SLIDE, 0x100, "A text box")),
                (3, slide(RT_SLIDE, 0, "Opening box")),
                (4, slide(RT_NOTES, 0, "Speaker note\u{0B}on two lines")),
            ],
        );
        assert_eq!(
            text_of(&deck.current_user(), &deck.stream).unwrap(),
            "Second file, first slide\nOpening box\n\nCittà\ndi Roma\nA text box\nSpeaker note\non two lines"
        );
    }

    #[test]
    fn an_incremental_save_supersedes_the_old_versions() {
        let mut deck = Deck {
            stream: Vec::new(),
            latest_edit: 0,
        };
        deck.save(
            1,
            &[
                (1, document(&[(2, "Title")], &[])),
                (2, slide(RT_SLIDE, 0, "Deleted words")),
            ],
        );
        deck.save(1, &[(2, slide(RT_SLIDE, 0, "Current words"))]);
        assert_eq!(
            text_of(&deck.current_user(), &deck.stream).unwrap(),
            "Title\nCurrent words"
        );
    }

    #[test]
    fn broken_or_protected_files_are_errors_not_panics() {
        let mut deck = Deck {
            stream: Vec::new(),
            latest_edit: 0,
        };
        deck.save(1, &[(1, document(&[(2, "Title")], &[]))]);
        let mut encrypted = deck.current_user();
        encrypted[12..16].copy_from_slice(&ENCRYPTED_TOKEN.to_le_bytes());
        assert!(text_of(&encrypted, &deck.stream)
            .unwrap_err()
            .to_string()
            .contains("password"));
        assert!(text_of(&[0; 20], &deck.stream)
            .unwrap_err()
            .to_string()
            .contains("PowerPoint 95"));

        // An edit that points back to itself, and one that points nowhere.
        let mut looping = deck.stream.clone();
        let at = deck.latest_edit + 8 + 8;
        looping[at..at + 4].copy_from_slice(&(deck.latest_edit as u32).to_le_bytes());
        assert_eq!(text_of(&deck.current_user(), &looping).unwrap(), "Title");
        let mut nowhere = deck.current_user();
        nowhere[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(text_of(&nowhere, &deck.stream).is_err());

        // Every truncation of a valid stream.
        for len in 0..deck.stream.len() {
            let _ = text_of(&deck.current_user(), &deck.stream[..len]);
        }
    }

    #[test]
    fn nesting_as_deep_as_the_file_is_long_is_fine() {
        const DEPTH: usize = 100_000;
        let text = chars("deep");
        let mut drawing = Vec::new();
        for level in 0..=DEPTH {
            let kind = if level == 0 { RT_DRAWING } else { 0xF003 };
            let len = (DEPTH - level) * 8 + text.len();
            drawing.extend(rec(0x000F, kind, &[])[..4].iter());
            drawing.extend((len as u32).to_le_bytes());
        }
        drawing.extend(&text);
        let mut texts = Vec::new();
        drawing_text(&drawing, &mut texts);
        assert_eq!(texts, ["deep"]);
    }
}
