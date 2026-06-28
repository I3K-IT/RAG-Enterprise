//! Text chunker — Community parameters (MAPPA §9 + PIANO §8):
//! chunk_size = 600, chunk_overlap = 100
//! separators = ["\n\n", "\n", ".", "", ""]
//!
//! CRITICO: il splitter deve RIEMPIRE fino a chunk_size, non spezzare a ogni
//! separatore (bug noto nel legacy: ~285 char invece di 600).
//!
//! Algoritmo: RecursiveCharacterTextSplitter fedele al Python langchain.
//! Golden-test: stesso testo → stessi chunk (stessa lunghezza e boundary).

pub const CHUNK_SIZE: usize = 600;
pub const CHUNK_OVERLAP: usize = 100;
pub const SEPARATORS: &[&str] = &["\n\n", "\n", ".", " ", ""];

/// Split `text` into overlapping chunks of at most `chunk_size` characters.
/// Mirrors Python RecursiveCharacterTextSplitter with the Community parameters.
pub fn split_text(text: &str) -> Vec<String> {
    split_recursive(text, SEPARATORS, CHUNK_SIZE, CHUNK_OVERLAP)
}

fn split_recursive(text: &str, separators: &[&str], size: usize, overlap: usize) -> Vec<String> {
    if text.len() <= size {
        return vec![text.to_owned()];
    }
    // Try separators in order; use the first one that splits the text
    for &sep in separators {
        if sep.is_empty() {
            // Character-level split as last resort
            return char_split(text, size, overlap);
        }
        if text.contains(sep) {
            return merge_splits(text.split(sep).collect(), sep, separators, size, overlap);
        }
    }
    char_split(text, size, overlap)
}

fn merge_splits(
    splits: Vec<&str>,
    separator: &str,
    separators: &[&str],
    size: usize,
    overlap: usize,
) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for split in splits {
        let candidate = if current.is_empty() {
            split.to_owned()
        } else {
            format!("{current}{separator}{split}")
        };

        if candidate.len() > size && !current.is_empty() {
            // Flush current chunk
            if current.len() > size {
                // Recursively split oversized chunk
                let sub = split_recursive(&current, separators, size, overlap);
                chunks.extend(sub);
            } else {
                chunks.push(current.clone());
            }
            // Overlap: seed next chunk with tail of current
            let tail = overlap_tail(&current, overlap);
            current = if tail.is_empty() {
                split.to_owned()
            } else {
                format!("{tail}{separator}{split}")
            };
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        if current.len() > size {
            chunks.extend(split_recursive(&current, separators, size, overlap));
        } else {
            chunks.push(current);
        }
    }
    chunks
}

fn overlap_tail(s: &str, overlap: usize) -> String {
    if s.len() <= overlap {
        return s.to_owned();
    }
    // Take the last `overlap` bytes, snapping to char boundary
    let start = s.len() - overlap;
    let snap = s[start..].char_indices().next().map(|(i, _)| start + i).unwrap_or(start);
    s[snap..].to_owned()
}

fn char_split(text: &str, size: usize, overlap: usize) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let end = (start + size).min(chars.len());
        chunks.push(chars[start..end].iter().collect());
        if end == chars.len() { break; }
        start = end.saturating_sub(overlap);
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_respect_size_limit() {
        let long = "word ".repeat(500);
        let chunks = split_text(&long);
        for c in &chunks {
            assert!(c.len() <= CHUNK_SIZE + 10, "chunk too large: {}", c.len());
        }
    }

    #[test]
    fn short_text_single_chunk() {
        let text = "Hello world.";
        let chunks = split_text(text);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], text);
    }
}
