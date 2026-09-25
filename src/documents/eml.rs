//! E-mail messages (.eml), and what an Outlook message (.msg) comes down to
//! as well: the headers a reader looks at — subject, sender, recipients,
//! date, the names of the attachments — then the body, then each attached
//! document, then any message forwarded as an attachment, in turn.
//!
//! An attached document — PDF, Word, Excel, PowerPoint, OpenDocument, RTF,
//! EPUB, plain text, another e-mail — is read with the reader an upload of
//! it would get, from a private copy under `{data_dir}/tmp/`. Pictures are
//! listed but not read: in e-mail they are mostly logos and signatures, and
//! each would cost an OCR run.

use std::cell::Cell;
use std::path::Path;

use anyhow::{Context, Result};
use mail_parser::{Address, Message, MessageParser, MimeHeaders, PartType};

use super::html::tidy;

/// Messages inside messages — forwarded, or attached as files — are read
/// this deep.
pub(crate) const MAX_DEPTH: usize = 8;

/// How many bytes one upload's messages may have read — attached files,
/// and in an Outlook file every stream — across all the messages nested in
/// it. Nothing stops an Outlook file from pointing any number of streams at
/// the same bytes, so without a bound a small file could have one attached
/// document read, or one attached message opened, over and over. The same
/// ceiling as for ZIP archives: far above any real mailbox export.
pub(crate) struct Budget {
    left: Cell<u64>,
    spent: Cell<bool>,
}

impl Budget {
    pub(crate) fn new() -> Self {
        Self::of(super::parser::MAX_UNCOMPRESSED_BYTES)
    }

    pub(crate) fn of(bytes: u64) -> Self {
        Self {
            left: Cell::new(bytes),
            spent: Cell::new(false),
        }
    }

    /// Takes `len` bytes from what is left; `false`, taking nothing, when
    /// there are not that many.
    pub(crate) fn take(&self, len: u64) -> bool {
        let left = self.left.get();
        if len <= left {
            self.left.set(left - len);
            return true;
        }
        if !self.spent.replace(true) {
            tracing::warn!("e-mail read budget spent: what is left of the message is not read");
        }
        false
    }
}

/// One message, whatever its format.
#[derive(Default)]
pub(crate) struct Letter {
    pub subject: Option<String>,
    pub from: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub date: Option<String>,
    pub attachments: Vec<String>,
    pub body: String,
    /// The attachments that were read: name, text.
    pub attached_text: Vec<(String, String)>,
}

impl Letter {
    /// The headers, one per line, a blank line, then the body.
    pub(crate) fn render(&self) -> String {
        let mut head = Vec::new();
        let list = |names: &[String]| (!names.is_empty()).then(|| names.join(", "));
        let fields = [
            ("Subject", self.subject.clone()),
            ("From", self.from.clone()),
            ("To", list(&self.to)),
            ("Cc", list(&self.cc)),
            ("Date", self.date.clone()),
            ("Attachments", list(&self.attachments)),
        ];
        for (label, value) in fields {
            if let Some(value) = value.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
                head.push(format!("{label}: {}", value.replace(['\r', '\n'], " ")));
            }
        }
        let mut parts = vec![head.join("\n"), tidy(&self.body)];
        for (name, text) in &self.attached_text {
            parts.push(format!("Attachment: {name}\n{}", tidy(text)));
        }
        parts.retain(|part| !part.is_empty());
        parts.join("\n\n")
    }
}

/// `Name <address>`, or whichever of the two there is.
pub(crate) fn mailbox(name: Option<&str>, address: Option<&str>) -> Option<String> {
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    let address = address.map(str::trim).filter(|a| !a.is_empty());
    match (name, address) {
        (Some(name), Some(address)) if name != address => Some(format!("{name} <{address}>")),
        (Some(name), _) => Some(name.to_owned()),
        (None, address) => address.map(str::to_owned),
    }
}

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
    let raw = std::fs::read(path).with_context(|| format!("reading eml {}", path.display()))?;
    let message = MessageParser::default()
        .parse(&raw)
        .context("not an e-mail message")?;
    let mut out = Vec::new();
    read_message(&message, data_dir, depth, budget, &mut out);
    out.retain(|part| !part.is_empty());
    Ok(out.join("\n\n"))
}

fn read_message(
    message: &Message<'_>,
    data_dir: &Path,
    depth: usize,
    budget: &Budget,
    out: &mut Vec<String>,
) {
    let addresses = |address: Option<&Address<'_>>| -> Vec<String> {
        address
            .map(|a| {
                a.iter()
                    .filter_map(|a| mailbox(a.name.as_deref(), a.address.as_deref()))
                    .collect()
            })
            .unwrap_or_default()
    };
    let mut body = Vec::new();
    for part in message.text_bodies() {
        match &part.body {
            PartType::Text(text) => body.push(text.to_string()),
            PartType::Html(html) => body.push(super::html::to_text(html)),
            _ => {}
        }
    }
    let mut attached_text = Vec::new();
    for part in message
        .attachments()
        .filter(|part| part.message().is_none())
    {
        let mime = part.content_type().map(|t| {
            format!(
                "{}/{}",
                t.c_type,
                t.c_subtype.as_deref().unwrap_or_default()
            )
        });
        let Some(ext) = attachment_extension(part.attachment_name(), mime.as_deref()) else {
            continue;
        };
        let name = part
            .attachment_name()
            .map_or_else(|| format!("attachment.{ext}"), str::to_owned);
        let text = match &part.body {
            // Already decoded from its charset.
            PartType::Text(text) if ext == "txt" => Some(text.to_string()),
            _ => read_attachment(&name, &ext, part.contents(), data_dir, depth, budget),
        };
        attached_text.extend(text.map(|text| (name, text)));
    }
    let letter = Letter {
        subject: message.subject().map(str::to_owned),
        from: addresses(message.from()).into_iter().next(),
        to: addresses(message.to()),
        cc: addresses(message.cc()),
        date: message.date().map(|date| date.to_rfc3339()),
        attachments: message
            .attachments()
            .filter_map(|part| part.attachment_name().map(str::to_owned))
            .collect(),
        body: body.join("\n"),
        attached_text,
    };
    out.push(letter.render());
    if depth < MAX_DEPTH {
        for forwarded in message.attachments().filter_map(|part| part.message()) {
            read_message(forwarded, data_dir, depth + 1, budget, out);
        }
    }
}

/// The extension an attachment is read as: its name's, or failing that its
/// media type's — `None` for pictures and for anything no reader handles.
pub(crate) fn attachment_extension(name: Option<&str>, mime: Option<&str>) -> Option<String> {
    let readable = |ext: &str| {
        super::parser::is_supported_extension(ext)
            && !super::parser::PICTURE_EXTENSIONS.contains(&ext)
    };
    let by_name = name
        .and_then(|n| Path::new(n.trim()).extension())
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    if let Some(ext) = by_name.filter(|e| readable(e)) {
        return Some(ext);
    }
    let by_type = match mime?.to_ascii_lowercase().as_str() {
        "application/pdf" => "pdf",
        "application/msword" => "doc",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.ms-excel" => "xls",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.ms-excel.sheet.macroenabled.12" => "xlsm",
        "application/vnd.ms-excel.sheet.binary.macroenabled.12" => "xlsb",
        "application/vnd.ms-powerpoint" => "ppt",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.oasis.opendocument.text" => "odt",
        "application/vnd.oasis.opendocument.spreadsheet" => "ods",
        "application/vnd.oasis.opendocument.presentation" => "odp",
        "application/rtf" | "text/rtf" => "rtf",
        "application/epub+zip" => "epub",
        "application/vnd.ms-outlook" => "msg",
        "message/rfc822" => "eml",
        "text/plain" => "txt",
        "text/csv" => "csv",
        "text/html" => "html",
        _ => return None,
    };
    Some(by_type.to_owned())
}

/// The text of an attached document, read with the reader an upload of it
/// would get — from a private copy, as every reader works on a file. `None`
/// when it cannot be read: the message itself still is.
pub(crate) fn read_attachment(
    name: &str,
    ext: &str,
    bytes: &[u8],
    data_dir: &Path,
    depth: usize,
    budget: &Budget,
) -> Option<String> {
    if matches!(ext, "eml" | "msg") && depth >= MAX_DEPTH || !budget.take(bytes.len() as u64) {
        return None;
    }
    let read = || -> Result<String> {
        let dir = private_temp_dir(data_dir)?;
        let copy = dir.path().join(format!("attachment.{ext}"));
        std::fs::write(&copy, bytes).context("copying the attachment")?;
        match ext {
            "eml" => extract_at(&copy, data_dir, depth + 1, budget),
            "msg" => super::msg::extract_at(&copy, data_dir, depth + 1, budget),
            _ => super::parser::extract_text(&copy, data_dir).map(|extracted| extracted.text),
        }
    };
    match read() {
        Ok(text) => Some(text),
        Err(e) => {
            tracing::warn!(attachment = name, error = %format!("{e:#}"), "attachment not read");
            None
        }
    }
}

/// A directory only this user can open, under `{data_dir}/tmp/` — not the
/// system temp dir, which on most Linux installs is RAM (see
/// `upload_tmp_path` in api/documents.rs) — removed when dropped.
fn private_temp_dir(data_dir: &Path) -> Result<tempfile::TempDir> {
    let root = data_dir.join("tmp");
    std::fs::create_dir_all(&root).with_context(|| format!("creating {}", root.display()))?;
    let mut builder = tempfile::Builder::new();
    builder.prefix("attachment-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    builder
        .tempdir_in(&root)
        .with_context(|| format!("creating a directory in {}", root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TESTDATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/documents/testdata");

    fn base64(bytes: &[u8]) -> String {
        const DIGITS: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
            for i in 0..4 {
                out.push(if i <= chunk.len() {
                    DIGITS[(n >> (18 - 6 * i) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out.as_bytes()
            .chunks(76)
            .map(|line| std::str::from_utf8(line).unwrap())
            .collect::<Vec<_>>()
            .join("\r\n")
    }

    /// A message with a one-line body and `attachments`: file name (or
    /// none), media type, content.
    fn message(subject: &str, attachments: &[(Option<&str>, &str, &[u8])]) -> Vec<u8> {
        let mut eml = format!(
            "From: a@example.com\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\n\
             Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
             --b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nBody of {subject}.\r\n"
        );
        for (name, media_type, content) in attachments {
            let disposition = name.map_or_else(
                || "attachment".to_owned(),
                |n| format!("attachment; filename=\"{n}\""),
            );
            eml.push_str(&format!(
                "--b\r\nContent-Type: {media_type}\r\nContent-Disposition: {disposition}\r\n\
                 Content-Transfer-Encoding: base64\r\n\r\n{}\r\n",
                base64(content)
            ));
        }
        eml.push_str("--b--\r\n");
        eml.into_bytes()
    }

    fn read(eml: &[u8], data_dir: &Path) -> String {
        let path = data_dir.join("mail.eml");
        std::fs::write(&path, eml).unwrap();
        extract_text(&path, data_dir).unwrap()
    }

    #[test]
    fn attached_documents_are_read_and_pictures_are_not() {
        let dir = tempfile::tempdir().unwrap();
        let offer = "Offerta economica per il cliente Rossi, valida fino a marzo. ".repeat(12);
        let pdf = super::super::parser::tests::minimal_multi_page_pdf(&[&offer]);
        let odt = std::fs::read(Path::new(TESTDATA).join("book.odt")).unwrap();
        let eml = message(
            "Documents",
            &[
                (Some("offerta.pdf"), "application/pdf", &pdf),
                (Some("book.odt"), "application/octet-stream", &odt),
                (None, "application/vnd.oasis.opendocument.text", &odt),
                (Some("logo.png"), "image/png", b"\x89PNG not really"),
                (
                    Some("broken.docx"),
                    "application/octet-stream",
                    b"PK\x03\x04 truncated",
                ),
            ],
        );
        let text = read(&eml, dir.path());
        assert!(
            text.contains("Attachments: offerta.pdf, book.odt, logo.png, broken.docx"),
            "{text}"
        );
        assert!(
            text.contains("Attachment: offerta.pdf\nOfferta economica per il cliente Rossi"),
            "{text}"
        );
        assert!(
            text.contains("Attachment: book.odt\nOpenDocument fixture"),
            "{text}"
        );
        assert!(
            text.contains("Attachment: attachment.odt\nOpenDocument fixture"),
            "found by its media type: {text}"
        );
        assert!(
            !text.contains("Attachment: logo.png"),
            "pictures are not read: {text}"
        );
        assert!(!text.contains("Attachment: broken.docx"), "{text}");
        assert!(
            text.starts_with("Subject: Documents"),
            "an unreadable attachment does not sink the message"
        );
        let left = std::fs::read_dir(dir.path().join("tmp")).unwrap().count();
        assert_eq!(left, 0, "the copies are removed");
    }

    #[test]
    fn a_spent_budget_skips_attachments_not_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let odt = std::fs::read(Path::new(TESTDATA).join("book.odt")).unwrap();
        let path = dir.path().join("mail.eml");
        std::fs::write(
            &path,
            message(
                "Big",
                &[(Some("book.odt"), "application/octet-stream", &odt)],
            ),
        )
        .unwrap();
        let text = extract_at(&path, dir.path(), 0, &Budget::of(1000)).unwrap();
        assert!(text.contains("Body of Big."), "{text}");
        assert!(!text.contains("Attachment: book.odt"), "{text}");

        let budget = Budget::of(10);
        assert!(budget.take(4) && budget.take(6));
        assert!(!budget.take(1), "nothing is left");
    }

    #[test]
    fn attached_messages_are_read_no_deeper_than_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let mut eml = message("Level 12", &[]);
        for level in (0..12).rev() {
            eml = message(
                &format!("Level {level}"),
                &[(Some("earlier.eml"), "application/octet-stream", &eml)],
            );
        }
        let text = read(&eml, dir.path());
        for level in 0..=MAX_DEPTH {
            assert!(
                text.contains(&format!("Body of Level {level}.")),
                "level {level} missing: {text}"
            );
        }
        assert!(
            !text.contains(&format!("Level {}.", MAX_DEPTH + 1)),
            "read past the limit: {text}"
        );
    }

    fn text(eml: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        read(eml.replace('\n', "\r\n").as_bytes(), dir.path())
    }

    #[test]
    fn headers_then_the_plain_text_body() {
        let eml = "From: Mario Rossi <mario@example.com>\n\
            To: Anna <anna@example.com>, luca@example.com\n\
            Subject: =?UTF-8?Q?Riunione_di_luned=C3=AC?=\n\
            Date: Tue, 1 Sep 2026 09:30:00 +0200\n\
            MIME-Version: 1.0\n\
            Content-Type: multipart/alternative; boundary=\"b\"\n\
            \n\
            --b\n\
            Content-Type: text/plain; charset=iso-8859-1\n\
            Content-Transfer-Encoding: quoted-printable\n\
            \n\
            Ciao, la riunione =E8 alle 10.\n\
            --b\n\
            Content-Type: text/html; charset=utf-8\n\
            \n\
            <p>Ciao, la riunione è alle 10.</p>\n\
            --b--\n";
        assert_eq!(
            text(eml),
            "Subject: Riunione di lunedì\nFrom: Mario Rossi <mario@example.com>\n\
             To: Anna <anna@example.com>, luca@example.com\nDate: 2026-09-01T09:30:00+02:00\n\n\
             Ciao, la riunione è alle 10."
        );
    }

    #[test]
    fn an_html_only_body_and_a_forwarded_message_are_read() {
        let eml = "From: a@example.com\n\
            Subject: Fwd\n\
            MIME-Version: 1.0\n\
            Content-Type: multipart/mixed; boundary=\"m\"\n\
            \n\
            --m\n\
            Content-Type: text/html; charset=utf-8\n\
            \n\
            <div>See <b>below</b></div><div>Thanks</div>\n\
            --m\n\
            Content-Type: application/pdf; name=\"offer.pdf\"\n\
            Content-Disposition: attachment; filename=\"offer.pdf\"\n\
            Content-Transfer-Encoding: base64\n\
            \n\
            JVBERi0xLjQK\n\
            --m\n\
            Content-Type: text/plain; charset=utf-8; name=\"notes.txt\"\n\
            Content-Disposition: attachment; filename=\"notes.txt\"\n\
            \n\
            Notes in a file.\n\
            --m\n\
            Content-Type: message/rfc822\n\
            \n\
            From: b@example.com\n\
            Subject: Original\n\
            \n\
            The original text.\n\
            --m--\n";
        assert_eq!(
            text(eml),
            "Subject: Fwd\nFrom: a@example.com\nAttachments: offer.pdf, notes.txt\n\nSee below\nThanks\n\n\
             Attachment: notes.txt\nNotes in a file.\n\nSubject: Original\nFrom: b@example.com\n\nThe original text."
        );
    }
}
