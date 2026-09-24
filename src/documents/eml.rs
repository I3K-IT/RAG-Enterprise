//! E-mail messages (.eml), and what an Outlook message (.msg) comes down to
//! as well: the headers a reader looks at — subject, sender, recipients,
//! date, the names of the attachments — then the body, the attachments
//! that are plain text, and any message forwarded as an attachment, in
//! turn. Other attachments are listed but not read: an attached PDF or
//! spreadsheet is a document of its own, best uploaded as one.

use std::path::Path;

use anyhow::{Context, Result};
use mail_parser::{Address, Message, MessageParser, MimeHeaders, PartType};

use super::html::tidy;

/// Forwarded messages inside forwarded messages are read this deep.
pub(crate) const MAX_DEPTH: usize = 8;

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
    /// Plain-text attachments: name, content.
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

pub fn extract_text(path: &Path) -> Result<String> {
    let raw = std::fs::read(path).with_context(|| format!("reading eml {}", path.display()))?;
    let message = MessageParser::default()
        .parse(&raw)
        .context("not an e-mail message")?;
    let mut out = Vec::new();
    read_message(&message, 0, &mut out);
    out.retain(|part| !part.is_empty());
    Ok(out.join("\n\n"))
}

fn read_message(message: &Message<'_>, depth: usize, out: &mut Vec<String>) {
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
        attached_text: message
            .attachments()
            .filter(|part| part.is_content_type("text", "plain"))
            .filter_map(|part| match &part.body {
                PartType::Text(text) => Some((
                    part.attachment_name().unwrap_or("text").to_owned(),
                    text.to_string(),
                )),
                _ => None,
            })
            .collect(),
    };
    out.push(letter.render());
    if depth < MAX_DEPTH {
        for forwarded in message.attachments().filter_map(|part| part.message()) {
            read_message(forwarded, depth + 1, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(eml: &str) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mail.eml");
        std::fs::write(&path, eml.replace('\n', "\r\n")).unwrap();
        extract_text(&path).unwrap()
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
