//! Source formatting for API responses.
//! display_score = similarity.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Source {
    pub document_id: String,
    pub filename: String,
    pub chunk_index: usize,
    pub similarity: f32,
    /// Absent from sources stored in the chat history (see
    /// [`history_json`]), hence the default when reading one back.
    #[serde(default)]
    pub text: String,
    // Source Provenance Foundation — infrastructural locator only, see
    // rag::vector_store::ChunkPayload for what each field means and why
    // they're all optional (absent on sources predating this feature).
    // source_start_byte/source_end_byte are BYTE offsets, not char offsets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_start_byte: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_end_byte: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page_end: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provenance_id: Option<String>,
}

/// The JSON stored with an answer in the chat history: every field of every
/// source except its chunk text.
///
/// That text is what the model was shown — up to `TOP_K` chunks of up to
/// `CHUNK_SIZE` characters, so 10–15 KB per answer — and the history kept
/// it forever, one copy per question, although nothing reads it back: the
/// web UI shows a source's file, pages and score, never its text, and the
/// live response still carries the text for any client that wants it. What
/// stays is enough to find the passage again (document, chunk index, byte
/// range, pages, provenance id).
pub fn history_json(sources: &[Source]) -> String {
    let mut value = serde_json::to_value(sources).unwrap_or_default();
    if let Some(items) = value.as_array_mut() {
        for item in items {
            if let Some(fields) = item.as_object_mut() {
                fields.remove("text");
            }
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(text: &str) -> Source {
        Source {
            document_id: "doc-1".into(),
            filename: "report.pdf".into(),
            chunk_index: 7,
            similarity: 0.82,
            text: text.into(),
            source_start_byte: Some(1200),
            source_end_byte: Some(2150),
            page_start: Some(3),
            page_end: Some(4),
            provenance_id: Some("prov-1".into()),
        }
    }

    #[test]
    fn history_keeps_everything_but_the_chunk_text() {
        let long = "x".repeat(1000);
        let json = history_json(&[source(&long), source(&long)]);

        assert!(!json.contains(&long), "the chunk text was stored");
        assert!(json.len() < 600, "two sources still take {} bytes", json.len());
        let stored: serde_json::Value = serde_json::from_str(&json).unwrap();
        let first = &stored[0];
        assert!(first.get("text").is_none());
        for (field, expected) in [
            ("document_id", serde_json::json!("doc-1")),
            ("filename", serde_json::json!("report.pdf")),
            ("chunk_index", serde_json::json!(7)),
            ("source_start_byte", serde_json::json!(1200)),
            ("page_start", serde_json::json!(3)),
            ("page_end", serde_json::json!(4)),
            ("provenance_id", serde_json::json!("prov-1")),
        ] {
            assert_eq!(first[field], expected, "{field}");
        }
    }

    /// Nothing reads the history back into `Source` today, but if something
    /// does, a stored source without text must still deserialize.
    #[test]
    fn a_stored_source_reads_back_with_empty_text() {
        let json = history_json(&[source("chunk")]);
        let back: Vec<Source> = serde_json::from_str(&json).unwrap();
        assert_eq!(back[0].text, "");
        assert_eq!(back[0].filename, "report.pdf");
    }
}
