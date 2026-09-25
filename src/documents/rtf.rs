//! Text of an RTF document (.rtf — and the .doc files that are really RTF,
//! as Word saves them when asked to).
//!
//! RTF is plain text: groups in braces, control words after a backslash,
//! and the document's own text in between. Reading it means following the
//! groups and keeping the text of the body — `\par` and `\line` end lines,
//! `\cell` separates the cells of a table row — while skipping the groups
//! that hold something else: the font, colour and style tables, pictures,
//! embedded objects, document properties, comments, field instructions
//! (their results are kept), hidden text, and every group marked `\*`,
//! which RTF defines as safe to skip for a reader that does not know it —
//! except a drawn shape's, whose text box holds text: of a shape, only its
//! properties and the fallback copy for older readers are skipped. Headers
//! and footers are kept, as the .doc reader keeps them.
//!
//! Text outside ASCII comes as `\uN` — a UTF-16 unit, followed by a
//! fallback for older readers, which is skipped — or as bytes in a code
//! page: that of the current font's character set, or else the document's.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use encoding_rs::{Encoding, WINDOWS_1252};

use super::codepage;

/// Groups whose content is not the document's text.
const SKIPPED: &[&str] = &[
    "annotation",
    "atnauthor",
    "atnid",
    "author",
    "buptim",
    "colortbl",
    "comment",
    "creatim",
    "doccomm",
    "fldinst",
    "info",
    "keywords",
    "listtext",
    "object",
    "operator",
    "pict",
    "pntext",
    "pntxta",
    "pntxtb",
    "printim",
    "revtim",
    "shprslt",
    "sp",
    "stylesheet",
    "subject",
    "title",
];

/// A fallback longer than this is not a fallback.
const MAX_FALLBACK: usize = 16;

pub fn extract_text(path: &Path) -> Result<String> {
    let rtf = std::fs::read(path).with_context(|| format!("reading rtf {}", path.display()))?;
    text_of(&rtf)
}

/// What a group sets, and its subgroups inherit.
#[derive(Clone, Copy)]
struct Group {
    /// Its text is not the document's.
    skip: bool,
    /// Formatted as hidden text (`\v`).
    hidden: bool,
    /// Inside the font table, whose entries give each font's encoding.
    font_table: bool,
    /// How many characters follow a `\uN` for readers without Unicode.
    fallback: usize,
    /// Of the bytes it holds: its font's, or the document's.
    encoding: &'static Encoding,
}

pub fn text_of(rtf: &[u8]) -> Result<String> {
    if !rtf.trim_ascii_start().starts_with(b"{\\rtf") {
        bail!("not an RTF document: it does not start with {{\\rtf");
    }
    let mut reader = Reader {
        out: String::new(),
        pending: Vec::new(),
        pending_encoding: WINDOWS_1252,
        high_surrogate: None,
    };
    let mut group = Group {
        skip: false,
        hidden: false,
        font_table: false,
        fallback: 1,
        encoding: WINDOWS_1252,
    };
    let mut stack: Vec<Group> = Vec::new();
    let mut document_encoding = WINDOWS_1252;
    let mut fonts: HashMap<i32, &'static Encoding> = HashMap::new();
    let mut default_font = None;
    let mut defining_font = None;
    // Fallback characters still to skip after a \uN.
    let mut to_skip = 0usize;
    // Right after a `{`, where `\*` marks the group as skippable…
    let mut group_start = false;
    // …by the control word that follows it.
    let mut star = false;
    let mut i = 0;
    while i < rtf.len() {
        let byte = rtf[i];
        i += 1;
        let at_group_start = std::mem::take(&mut group_start);
        let starred = std::mem::take(&mut star);
        match byte {
            b'{' => {
                stack.push(group);
                to_skip = 0;
                group_start = true;
            }
            b'}' => {
                reader.flush();
                let closed = group;
                let Some(parent) = stack.pop() else { continue };
                group = parent;
                // The document's own group has closed: anything after it
                // is not part of it.
                if stack.is_empty() {
                    break;
                }
                // Text after the font table, before any \f, is in the
                // default font.
                if closed.font_table && !group.font_table {
                    group.encoding = default_font
                        .and_then(|f| fonts.get(&f).copied())
                        .unwrap_or(document_encoding);
                }
                to_skip = 0;
            }
            b'\r' | b'\n' => {}
            b'\\' => {
                let Some(&next) = rtf.get(i) else { break };
                if !next.is_ascii_alphabetic() {
                    // A control symbol.
                    i += 1;
                    let text = match next {
                        b'\'' => {
                            let hex = rtf.get(i..i + 2).and_then(|h| std::str::from_utf8(h).ok());
                            let value = hex.and_then(|h| u8::from_str_radix(h, 16).ok());
                            i += 2;
                            if let Some(value) = value {
                                if to_skip > 0 {
                                    to_skip -= 1;
                                } else if group.visible() {
                                    reader.byte(value, group.encoding);
                                }
                            }
                            continue;
                        }
                        b'\\' | b'{' | b'}' => Symbol::Byte(next),
                        b'~' => Symbol::Char('\u{A0}'),
                        b'_' => Symbol::Char('-'),
                        b'\r' | b'\n' => Symbol::Char('\n'),
                        b'*' => {
                            // Anywhere else it means nothing.
                            star = at_group_start;
                            continue;
                        }
                        _ => Symbol::Nothing,
                    };
                    if to_skip > 0 {
                        to_skip -= 1;
                    } else if group.visible() {
                        match text {
                            Symbol::Byte(b) => reader.byte(b, group.encoding),
                            Symbol::Char(c) => reader.char(c),
                            Symbol::Nothing => {}
                        }
                    }
                    continue;
                }
                // A control word: letters, an optional signed number, and a
                // space that belongs to it.
                let start = i;
                while i < rtf.len() && rtf[i].is_ascii_alphabetic() && i - start < 32 {
                    i += 1;
                }
                let word = std::str::from_utf8(&rtf[start..i]).unwrap_or_default();
                let negative = rtf.get(i) == Some(&b'-');
                if negative {
                    i += 1;
                }
                let digits = i;
                let mut value: i64 = 0;
                while i < rtf.len() && rtf[i].is_ascii_digit() && i - digits < 10 {
                    value = value * 10 + i64::from(rtf[i] - b'0');
                    i += 1;
                }
                let param = (i > digits).then(|| {
                    let value = if negative { -value } else { value };
                    value.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
                });
                if rtf.get(i) == Some(&b' ') {
                    i += 1;
                }
                if starred && word != "shpinst" {
                    group.skip = true;
                }

                match word {
                    // Raw bytes, whatever the group: skipped by count, as
                    // they may contain braces and backslashes.
                    "bin" => {
                        i = i
                            .saturating_add(param.unwrap_or(0).max(0) as usize)
                            .min(rtf.len());
                        continue;
                    }
                    "u" => {
                        if group.visible() {
                            reader.unicode(param.unwrap_or(0) as u16);
                        }
                        to_skip = group.fallback;
                        continue;
                    }
                    "uc" => {
                        group.fallback = (param.unwrap_or(1).max(0) as usize).min(MAX_FALLBACK);
                        continue;
                    }
                    _ => {}
                }
                if to_skip > 0 {
                    to_skip -= 1;
                    continue;
                }
                match word {
                    "fonttbl" => {
                        group.font_table = true;
                        group.skip = true;
                    }
                    "f" if group.font_table => defining_font = param,
                    "fcharset" | "cpg" if group.font_table => {
                        let found = param.and_then(|p| {
                            if word == "cpg" {
                                codepage::by_number(p as u32)
                            } else {
                                codepage::by_charset(p as u8)
                            }
                        });
                        if let (Some(font), Some(encoding)) = (defining_font, found) {
                            fonts.insert(font, encoding);
                        }
                    }
                    "f" => {
                        reader.flush();
                        group.encoding = param
                            .and_then(|font| fonts.get(&font).copied())
                            .unwrap_or(document_encoding);
                    }
                    "plain" => {
                        reader.flush();
                        group.hidden = false;
                        group.encoding = default_font
                            .and_then(|font| fonts.get(&font).copied())
                            .unwrap_or(document_encoding);
                    }
                    "deff" => default_font = param,
                    "ansicpg" => {
                        if let Some(encoding) = param.and_then(|p| codepage::by_number(p as u32)) {
                            reader.flush();
                            document_encoding = encoding;
                            group.encoding = encoding;
                        }
                    }
                    "mac" => {
                        reader.flush();
                        document_encoding = encoding_rs::MACINTOSH;
                        group.encoding = document_encoding;
                    }
                    "v" => group.hidden = param != Some(0),
                    word if SKIPPED.contains(&word) => group.skip = true,
                    _ if !group.visible() => {}
                    "par" | "line" | "sect" | "page" | "row" | "lbr" => reader.char('\n'),
                    "tab" | "cell" | "nestcell" => reader.char('\t'),
                    "emdash" => reader.char('—'),
                    "endash" => reader.char('–'),
                    "bullet" => reader.char('•'),
                    "lquote" => reader.char('‘'),
                    "rquote" => reader.char('’'),
                    "ldblquote" => reader.char('“'),
                    "rdblquote" => reader.char('”'),
                    "emspace" | "enspace" | "qmspace" => reader.char(' '),
                    _ => {}
                }
            }
            _ => {
                if to_skip > 0 {
                    to_skip -= 1;
                } else if group.visible() {
                    reader.byte(byte, group.encoding);
                }
            }
        }
    }
    reader.flush();
    reader.settle();
    Ok(super::html::tidy(&reader.out))
}

impl Group {
    fn visible(&self) -> bool {
        !self.skip && !self.hidden
    }
}

enum Symbol {
    Byte(u8),
    Char(char),
    Nothing,
}

struct Reader {
    out: String,
    /// Bytes not yet decoded: a character in a multi-byte code page is
    /// written as consecutive bytes, so they are decoded together.
    pending: Vec<u8>,
    pending_encoding: &'static Encoding,
    high_surrogate: Option<u16>,
}

impl Reader {
    fn byte(&mut self, byte: u8, encoding: &'static Encoding) {
        self.settle();
        if encoding != self.pending_encoding {
            self.flush();
            self.pending_encoding = encoding;
        }
        self.pending.push(byte);
    }

    fn char(&mut self, c: char) {
        self.flush();
        self.settle();
        self.out.push(c);
    }

    /// A `\uN` value: a character, or half of one.
    fn unicode(&mut self, unit: u16) {
        self.flush();
        match unit {
            0xD800..=0xDBFF => {
                self.settle();
                self.high_surrogate = Some(unit);
            }
            0xDC00..=0xDFFF => {
                let c = self.high_surrogate.take().and_then(|high| {
                    char::from_u32(
                        0x10000 + ((u32::from(high) - 0xD800) << 10) + (u32::from(unit) - 0xDC00),
                    )
                });
                self.out.push(c.unwrap_or(char::REPLACEMENT_CHARACTER));
            }
            _ => {
                self.settle();
                self.out
                    .push(char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER));
            }
        }
    }

    /// A first half of a surrogate pair that no second half followed.
    fn settle(&mut self) {
        if self.high_surrogate.take().is_some() {
            self.out.push(char::REPLACEMENT_CHARACTER);
        }
    }

    fn flush(&mut self) {
        if !self.pending.is_empty() {
            let (text, _) = self
                .pending_encoding
                .decode_without_bom_handling(&self.pending);
            self.out.push_str(&text);
            self.pending.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(rtf: &str) -> String {
        text_of(rtf.as_bytes()).unwrap()
    }

    #[test]
    fn the_body_is_read_and_the_tables_are_not() {
        let rtf = r"{\rtf1\ansi\ansicpg1252\deff0{\fonttbl{\f0\fswiss\fcharset0 Arial;}}
            {\colortbl;\red0\green0\blue0;}{\stylesheet{\s1 heading 1;}}
            {\info{\title Secret title}{\author Someone}}
            {\header Page header\par}
            \pard Hello {\b bold} world\par
            Second line: caff\'e8 costs 3\u8364? here\line soft break\par
            {\*\generator Writer}{\pict\wmetafile8 0102abcdef}Done\par}";
        assert_eq!(
            text(rtf),
            "Page header\nHello bold world\nSecond line: caffè costs 3€ here\nsoft break\nDone"
        );
    }

    #[test]
    fn a_font_character_set_decides_its_bytes() {
        let rtf = r"{\rtf1\ansi\deff0{\fonttbl{\f0\fnil\fcharset0 Arial;}{\f1\fnil\fcharset204 Arial Cyr;}}
            \f1 \'cf\'f0\'e8\'e2\'e5\'f2 \plain caf\'e9\par}";
        assert_eq!(text(rtf), "Привет café");
        let by_default =
            r"{\rtf1\ansi\deff1{\fonttbl{\f0 Arial;}{\f1\fcharset238 Arial CE;}}\'9ailuta\par}";
        assert_eq!(text(by_default), "šiluta");
    }

    #[test]
    fn double_byte_code_pages_are_decoded_whole() {
        assert_eq!(text(r"{\rtf1\ansi\ansicpg932 \'82\'a0\'83A\par}"), "あア");
    }

    #[test]
    fn unicode_fallbacks_are_skipped_and_surrogates_paired() {
        assert_eq!(
            text(r"{\rtf1\uc1\u233?t\u233? \uc2\u8364\'80\'80!\uc0\u233 x\par}"),
            "été €!éx"
        );
        assert_eq!(text(r"{\rtf1 smile \u-10179?\u-8704?\par}"), "smile 😀");
        assert_eq!(
            text(r"{\rtf1 hi \u-10179? here, lo \u-8704? here, pair \u-10179?\u-10179? here\par}"),
            "hi \u{FFFD} here, lo \u{FFFD} here, pair \u{FFFD}\u{FFFD} here"
        );
    }

    #[test]
    fn fields_show_their_result() {
        let rtf = r#"{\rtf1 See {\field{\*\fldinst HYPERLINK "https://example.com"}{\fldrslt the site}} now.\par}"#;
        assert_eq!(text(rtf), "See the site now.");
    }

    #[test]
    fn a_table_row_stays_on_one_line() {
        let rtf = r"{\rtf1\trowd\cellx1000\cellx2000 Quarter\cell Amount\cell\row\trowd Q1\cell 1250\cell\row}";
        assert_eq!(text(rtf), "Quarter\tAmount\nQ1\t1250");
    }

    #[test]
    fn binary_data_can_hold_braces() {
        let mut rtf = b"{\\rtf1 before{\\pict\\bin6 ".to_vec();
        rtf.extend(b"}{\\x}{");
        rtf.extend(b"}after\\par}");
        assert_eq!(text_of(&rtf).unwrap(), "beforeafter");
    }

    #[test]
    fn hidden_text_and_escapes() {
        assert_eq!(text(r"{\rtf1 a{\v hidden}b \v x\v0 c\par}"), "ab c");
        assert_eq!(
            text(r"{\rtf1 \{c\} d\\e\~f\-g\_h\par}"),
            "{c} d\\e\u{A0}fg-h"
        );
    }

    #[test]
    fn a_text_box_is_read_once() {
        let rtf = r"{\rtf1 Before\par{\shp{\*\shpinst\shpleft0{\sp{\sn fillColor}{\sv 255}}
            {\shptxt In the box\par}}{\shprslt{\*\do\dptxbx{\dptxbxtext In the box\par}}}}After\par}";
        assert_eq!(text(rtf), "Before\nIn the box\nAfter");
    }

    #[test]
    fn a_star_counts_only_at_the_start_of_a_group() {
        assert_eq!(
            text(r"{\rtf1 {\f2\i0\*\cs7 The quick brown fox}\par}"),
            "The quick brown fox"
        );
    }

    #[test]
    fn nothing_after_the_document_is_read() {
        assert_eq!(
            text_of(b"{\\rtf1 Body\\par}\n\xef\xbf\xbd8\x11$DM").unwrap(),
            "Body"
        );
    }

    #[test]
    fn broken_documents_do_not_panic() {
        assert!(text_of(b"not rtf").is_err());
        for rtf in [
            r"{\rtf1 unclosed {\b group",
            r"{\rtf1 }}}} extra",
            r"{\rtf1 \'",
            r"{\rtf1 \u",
            r"{\rtf1 \bin99999999999",
        ] {
            text_of(rtf.as_bytes()).unwrap();
        }
    }
}
