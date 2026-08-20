//! Text chunker — faithful port of LangChain RecursiveCharacterTextSplitter.
//!
//! Parameters verified against Community source (RAG-Enterprise/backend/app.py:687):
//!   chunk_size=1000, chunk_overlap=100
//!   separators=["\n\n", "\n", " ", ""]  — LangChain defaults; chunk_text() does not
//!   specify separators, so the "." present in the constructor is NOT used in production.
//!
//! Key invariant: the splitter FILLS chunks up to chunk_size by accumulating small
//! pieces before flushing. Breaking at every separator would produce tiny chunks
//! (~5 chars per "word") — this was a known bug in the old port (~285 chars vs 600).

pub const CHUNK_SIZE: usize = 1000;
pub const CHUNK_OVERLAP: usize = 100;
const SEPARATORS: &[&str] = &["\n\n", "\n", " ", ""];

/// Bumped whenever CHUNK_SIZE/CHUNK_OVERLAP/SEPARATORS change. Baked into
/// every `provenance_id` (see below) so that re-chunking the same document
/// under a different configuration produces visibly different ids instead
/// of silently colliding with — or worse, being mistaken for — the old
/// ones: chunk_index=5 under v1 and chunk_index=5 under v2 can be
/// completely different text.
pub const CHUNKING_CONFIG_VERSION: u32 = 1;

/// Deterministic, versioned identifier for one chunk — stable across
/// re-ingesting the SAME file content under the SAME chunking
/// configuration. Deliberately NOT derived from `document_id`: that is a
/// fresh UUID minted on every upload (see api/documents.rs), so re-ingesting
/// an unchanged file would otherwise mint an unrelated id for what is, for
/// citation purposes, "the same" chunk. `content_sha256` anchors identity to
/// the file's own bytes instead — the hex sha256 digest of the raw uploaded
/// content, computed once at upload time.
pub fn provenance_id(content_sha256: &str, chunk_index: usize) -> String {
    format!("{content_sha256}:{chunk_index}:cv{CHUNKING_CONFIG_VERSION}")
}

/// One chunk of text plus its byte-offset span `[start, end)` within the
/// source text passed to `split_text` — the universal locator (§ design
/// note in api/documents.rs): available for every format, since every
/// parser already produces a single flat extracted-text string before this
/// module ever sees it. Byte offsets, not char offsets: O(1) to compute via
/// pointer arithmetic on the original allocation (see `offset_of`), where a
/// char-index scheme would need an O(n) count per chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub start: usize,
    pub end: usize,
}

/// Split `text` into overlapping chunks of at most `CHUNK_SIZE` characters,
/// each carrying its byte-offset span within `text`. Empty input returns an
/// empty vec.
pub fn split_text(text: &str) -> Vec<Chunk> {
    if text.is_empty() {
        return vec![];
    }
    split_recursive(text, text, SEPARATORS)
}

// LangChain measures length in characters (Unicode code points), not bytes.
#[inline]
fn clen(s: &str) -> usize {
    s.chars().count()
}

/// Byte offset of `piece` within `root`, via pointer arithmetic — valid
/// because `piece` is always, transitively, a subslice of `root`: every
/// `&str` handled by this module either IS `root` itself or was produced by
/// slicing/splitting a text that traces back to it. No allocation, no
/// search, exact by construction (unlike searching for `piece`'s content in
/// `root`, which could match the wrong occurrence if the same text repeats).
#[inline]
fn offset_of(root: &str, piece: &str) -> usize {
    piece.as_ptr() as usize - root.as_ptr() as usize
}

fn split_recursive(root: &str, text: &str, separators: &[&str]) -> Vec<Chunk> {
    // Find the first separator that appears in the text.
    let mut chosen = "";
    let mut tail_seps: &[&str] = &[];

    for (i, &sep) in separators.iter().enumerate() {
        if sep.is_empty() {
            break; // fall through to char-level
        }
        if text.contains(sep) {
            chosen = sep;
            tail_seps = &separators[i + 1..];
            break;
        }
    }

    if chosen.is_empty() {
        return char_chunks(root, text);
    }

    // Split on the chosen separator, discarding empty pieces.
    let pieces: Vec<&str> = text.split(chosen).filter(|s| !s.is_empty()).collect();

    let mut result: Vec<Chunk> = Vec::new();
    let mut small: Vec<&str> = Vec::new(); // pieces that fit within CHUNK_SIZE

    for &piece in &pieces {
        if clen(piece) < CHUNK_SIZE {
            small.push(piece);
        } else {
            // Flush accumulated small pieces first.
            if !small.is_empty() {
                result.extend(merge_small(root, &small, chosen));
                small.clear();
            }
            // Recursively break the oversized piece, or emit as-is.
            if tail_seps.is_empty() {
                let start = offset_of(root, piece);
                result.push(Chunk { text: piece.to_owned(), start, end: start + piece.len() });
            } else {
                result.extend(split_recursive(root, piece, tail_seps));
            }
        }
    }

    if !small.is_empty() {
        result.extend(merge_small(root, &small, chosen));
    }
    result
}

/// Combine small pieces into chunks ≤ CHUNK_SIZE with CHUNK_OVERLAP.
/// Mirrors LangChain's `_merge_splits`. Each flushed chunk's span is the
/// first piece's start to the last piece's end, in `root` — correct even
/// when consecutive separators collapsed an empty piece out of `pieces`
/// (the span still bounds the real content, only the reconstructed `text`
/// normalises the gap to a single `sep`, same as before this change).
fn merge_small(root: &str, pieces: &[&str], sep: &str) -> Vec<Chunk> {
    let sep_len = clen(sep);
    let mut docs: Vec<Chunk> = Vec::new();
    let mut window: Vec<&str> = Vec::new();
    let mut total: usize = 0; // clen(window.join(sep))

    let flush = |window: &[&str]| -> Chunk {
        let text = window.join(sep);
        let start = offset_of(root, window[0]);
        let last = window[window.len() - 1];
        let end = offset_of(root, last) + last.len();
        Chunk { text, start, end }
    };

    for &piece in pieces {
        let plen = clen(piece);
        let sep_before = if window.is_empty() { 0 } else { sep_len };

        if total + plen + sep_before > CHUNK_SIZE && !window.is_empty() {
            docs.push(flush(&window));

            // Shrink window toward overlap.
            // Keep removing from the front while:
            //   total > CHUNK_OVERLAP  OR  (total+plen+sep > CHUNK_SIZE AND total > 0)
            loop {
                if window.is_empty() {
                    break;
                }
                let sep_if_nonempty = sep_len; // window non-empty (checked above)
                let above_overlap = total > CHUNK_OVERLAP;
                let still_exceeds =
                    total + plen + sep_if_nonempty > CHUNK_SIZE && total > 0;
                if !above_overlap && !still_exceeds {
                    break;
                }
                // Remove the oldest piece; also remove the separator that followed it
                // if there are more pieces remaining.
                let removed =
                    clen(window[0]) + if window.len() > 1 { sep_len } else { 0 };
                total = total.saturating_sub(removed);
                window.remove(0);
            }
        }

        window.push(piece);
        // Separator is counted only between adjacent pieces (len > 1 after push).
        total += plen + if window.len() > 1 { sep_len } else { 0 };
    }

    if !window.is_empty() {
        docs.push(flush(&window));
    }
    docs
}

/// Last-resort character-level chunking with overlap (used when no separator is found).
fn char_chunks(root: &str, text: &str) -> Vec<Chunk> {
    let base = offset_of(root, text);
    let chars: Vec<char> = text.chars().collect();

    // Cumulative byte offset before each char index, computed once (O(n))
    // so every chunk's span is an O(1) lookup instead of re-scanning from 0.
    let mut byte_at = Vec::with_capacity(chars.len() + 1);
    let mut acc = 0usize;
    byte_at.push(0);
    for c in &chars {
        acc += c.len_utf8();
        byte_at.push(acc);
    }

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + CHUNK_SIZE).min(chars.len());
        let s: String = chars[start..end].iter().collect();
        chunks.push(Chunk { text: s, start: base + byte_at[start], end: base + byte_at[end] });
        if end == chars.len() {
            break;
        }
        start = end - CHUNK_OVERLAP;
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every chunk's [start, end) must slice out of `text` to exactly its
    /// own `text` field — the property the whole pointer-arithmetic scheme
    /// exists to guarantee. Checked once, generically, instead of repeating
    /// it by hand in every test below.
    fn assert_spans_correct(source: &str, chunks: &[Chunk]) {
        for (i, c) in chunks.iter().enumerate() {
            assert!(c.start <= c.end, "chunk {i}: start > end");
            assert!(c.end <= source.len(), "chunk {i}: end past source length");
            assert_eq!(
                &source[c.start..c.end],
                c.text.as_str(),
                "chunk {i}: span does not slice back to its own text"
            );
        }
    }

    fn texts(chunks: &[Chunk]) -> Vec<String> {
        chunks.iter().map(|c| c.text.clone()).collect()
    }

    #[test]
    fn empty_input() {
        assert_eq!(split_text(""), Vec::<Chunk>::new());
    }

    #[test]
    fn short_text_single_chunk() {
        let text = "Hello, world! This is a short sentence.";
        let chunks = split_text(text);
        assert_eq!(texts(&chunks), vec![text.to_owned()]);
        assert_spans_correct(text, &chunks);
    }

    #[test]
    fn fills_to_chunk_size() {
        // 300 words × 5 chars = 1500 chars total.
        // Must produce ≤3 chunks (NOT one chunk per word).
        let text = "word ".repeat(300);
        let text = text.trim();
        let chunks = split_text(text);
        assert!(chunks.len() <= 3, "expected ≤3 chunks, got {}", chunks.len());
        // First chunk must be close to CHUNK_SIZE, not tiny.
        assert!(
            chunks[0].text.len() > CHUNK_SIZE / 2,
            "first chunk too small: {} chars",
            chunks[0].text.len()
        );
        assert_spans_correct(text, &chunks);
    }

    #[test]
    fn respects_chunk_size() {
        let text = "word ".repeat(1000);
        let text = text.trim();
        let chunks = split_text(text);
        for chunk in &chunks {
            assert!(
                clen(&chunk.text) <= CHUNK_SIZE,
                "chunk exceeds CHUNK_SIZE: {} chars",
                clen(&chunk.text)
            );
        }
        assert_spans_correct(text, &chunks);
    }

    #[test]
    fn overlap_is_present() {
        // Last few words of chunk N must appear at the start of chunk N+1.
        let text = "word ".repeat(300);
        let text = text.trim();
        let chunks = split_text(text);
        if chunks.len() >= 2 {
            // Take a word near the end of chunk 0 and verify it's in chunk 1.
            let last_word = chunks[0].text.split_whitespace().last().unwrap_or("");
            assert!(
                chunks[1].text.contains(last_word),
                "no overlap found between chunk 0 and chunk 1"
            );
        }
        assert_spans_correct(text, &chunks);
    }

    #[test]
    fn paragraph_separator_preferred_over_space() {
        // Two short paragraphs (each ~140 chars) fit in one chunk together.
        // The splitter should NOT split them on spaces — they're joined as one chunk.
        let p1 = "paragraph one ".repeat(10);
        let p2 = "paragraph two ".repeat(10);
        let text = format!("{}\n\n{}", p1.trim(), p2.trim());
        let chunks = split_text(&text);
        assert_eq!(chunks.len(), 1, "short paragraphs should stay in one chunk");
        assert!(chunks[0].text.contains("paragraph one"));
        assert!(chunks[0].text.contains("paragraph two"));
        assert_spans_correct(&text, &chunks);
    }

    #[test]
    fn char_level_fallback_for_long_nospace_text() {
        let s = "a".repeat(2500);
        let chunks = split_text(&s);
        assert!(chunks.len() >= 2);
        assert_eq!(clen(&chunks[0].text), CHUNK_SIZE);
        for c in &chunks {
            assert!(clen(&c.text) <= CHUNK_SIZE);
        }
        assert_spans_correct(&s, &chunks);
    }

    /// Spans must stay correct through every recursion level, not just at
    /// the top: mixed paragraph/newline/space text forces the splitter
    /// through split_recursive AND merge_small AND the oversized-piece
    /// recursion in the same document.
    #[test]
    fn spans_correct_through_mixed_recursion() {
        let big_paragraph = "supercalifragilisticexpialidocious ".repeat(60); // forces recursion past "\n\n" and "\n"
        let text = format!(
            "Intro short line.\n\n{}\n\nAnother short paragraph here.\n\nFinal one.",
            big_paragraph.trim()
        );
        let chunks = split_text(&text);
        assert!(chunks.len() >= 2, "expected the big paragraph to force multiple chunks");
        assert_spans_correct(&text, &chunks);
    }

    /// Non-ASCII text (multibyte UTF-8) exercises the char-vs-byte distinction
    /// directly: char_chunks must report BYTE offsets, and they must still
    /// slice correctly even though char count != byte count here.
    #[test]
    fn spans_correct_with_multibyte_utf8_char_fallback() {
        let s = "à".repeat(2500); // no separators at all → char_chunks path; 2 bytes/char
        let chunks = split_text(&s);
        assert!(chunks.len() >= 2);
        assert_spans_correct(&s, &chunks);
    }

    #[test]
    fn provenance_id_is_deterministic_and_config_aware() {
        let a = provenance_id("abc123", 5);
        let b = provenance_id("abc123", 5);
        assert_eq!(a, b, "same content hash + chunk_index must yield the same provenance_id");

        let different_chunk = provenance_id("abc123", 6);
        assert_ne!(a, different_chunk, "different chunk_index must yield a different provenance_id");

        let different_content = provenance_id("def456", 5);
        assert_ne!(a, different_content, "different content hash must yield a different provenance_id");

        assert!(a.contains(&format!("cv{CHUNKING_CONFIG_VERSION}")));
    }
}
