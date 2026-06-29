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

/// Split `text` into overlapping chunks of at most `CHUNK_SIZE` characters.
/// Empty input returns an empty vec.
pub fn split_text(text: &str) -> Vec<String> {
    if text.is_empty() {
        return vec![];
    }
    split_recursive(text, SEPARATORS)
}

// LangChain measures length in characters (Unicode code points), not bytes.
#[inline]
fn clen(s: &str) -> usize {
    s.chars().count()
}

fn split_recursive(text: &str, separators: &[&str]) -> Vec<String> {
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
        return char_chunks(text);
    }

    // Split on the chosen separator, discarding empty pieces.
    let pieces: Vec<&str> = text.split(chosen).filter(|s| !s.is_empty()).collect();

    let mut result: Vec<String> = Vec::new();
    let mut small: Vec<&str> = Vec::new(); // pieces that fit within CHUNK_SIZE

    for &piece in &pieces {
        if clen(piece) < CHUNK_SIZE {
            small.push(piece);
        } else {
            // Flush accumulated small pieces first.
            if !small.is_empty() {
                result.extend(merge_small(&small, chosen));
                small.clear();
            }
            // Recursively break the oversized piece, or emit as-is.
            if tail_seps.is_empty() {
                result.push(piece.to_owned());
            } else {
                result.extend(split_recursive(piece, tail_seps));
            }
        }
    }

    if !small.is_empty() {
        result.extend(merge_small(&small, chosen));
    }
    result
}

/// Combine small pieces into chunks ≤ CHUNK_SIZE with CHUNK_OVERLAP.
/// Mirrors LangChain's `_merge_splits`.
fn merge_small(pieces: &[&str], sep: &str) -> Vec<String> {
    let sep_len = clen(sep);
    let mut docs: Vec<String> = Vec::new();
    let mut window: Vec<&str> = Vec::new();
    let mut total: usize = 0; // clen(window.join(sep))

    for &piece in pieces {
        let plen = clen(piece);
        let sep_before = if window.is_empty() { 0 } else { sep_len };

        if total + plen + sep_before > CHUNK_SIZE && !window.is_empty() {
            docs.push(window.join(sep));

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
        docs.push(window.join(sep));
    }
    docs
}

/// Last-resort character-level chunking with overlap (used when no separator is found).
fn char_chunks(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + CHUNK_SIZE).min(chars.len());
        chunks.push(chars[start..end].iter().collect());
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

    #[test]
    fn empty_input() {
        assert_eq!(split_text(""), Vec::<String>::new());
    }

    #[test]
    fn short_text_single_chunk() {
        let text = "Hello, world! This is a short sentence.";
        let chunks = split_text(text);
        assert_eq!(chunks, vec![text.to_owned()]);
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
            chunks[0].len() > CHUNK_SIZE / 2,
            "first chunk too small: {} chars",
            chunks[0].len()
        );
    }

    #[test]
    fn respects_chunk_size() {
        let text = "word ".repeat(1000);
        for chunk in split_text(text.trim()) {
            assert!(
                clen(&chunk) <= CHUNK_SIZE,
                "chunk exceeds CHUNK_SIZE: {} chars",
                clen(&chunk)
            );
        }
    }

    #[test]
    fn overlap_is_present() {
        // Last few words of chunk N must appear at the start of chunk N+1.
        let text = "word ".repeat(300);
        let chunks = split_text(text.trim());
        if chunks.len() >= 2 {
            // Take a word near the end of chunk 0 and verify it's in chunk 1.
            let last_word = chunks[0].split_whitespace().last().unwrap_or("");
            assert!(
                chunks[1].contains(last_word),
                "no overlap found between chunk 0 and chunk 1"
            );
        }
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
        assert!(chunks[0].contains("paragraph one"));
        assert!(chunks[0].contains("paragraph two"));
    }

    #[test]
    fn char_level_fallback_for_long_nospace_text() {
        let s = "a".repeat(2500);
        let chunks = split_text(&s);
        assert!(chunks.len() >= 2);
        assert_eq!(clen(&chunks[0]), CHUNK_SIZE);
        for c in &chunks {
            assert!(clen(c) <= CHUNK_SIZE);
        }
    }
}
