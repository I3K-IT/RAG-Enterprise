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

/// Bumped whenever CHUNK_SIZE/CHUNK_OVERLAP/SEPARATORS change, OR the
/// stored/embedded text for a given chunk_index would otherwise differ from
/// a prior version — e.g. `inject_heading_context` (below) started
/// prepending heading context in v2, so a chunk re-ingested under v2 is
/// genuinely different content from the same chunk_index under v1, even
/// though `split_text`'s own boundaries are unchanged. Baked into every
/// `provenance_id` (see below) so that re-chunking the same document under a
/// different configuration produces visibly different ids instead of
/// silently colliding with — or worse, being mistaken for — the old ones:
/// chunk_index=5 under v1 and chunk_index=5 under v2 can be completely
/// different text.
pub const CHUNKING_CONFIG_VERSION: u32 = 2;

/// Deterministic, versioned identifier for one chunk — stable across
/// re-ingesting the SAME file content under the SAME extraction AND
/// chunking configuration. Deliberately NOT derived from `document_id`:
/// that is a fresh UUID minted on every upload (see api/documents.rs), so
/// re-ingesting an unchanged file would otherwise mint an unrelated id for
/// what is, for citation purposes, "the same" chunk. `content_sha256`
/// anchors identity to the file's own bytes instead — the hex sha256
/// digest of the raw uploaded content, computed once at upload time.
///
/// `extraction_config_version` is the caller's
/// `documents::parser::EXTRACTION_CONFIG_VERSION`, passed in rather than
/// read directly from the parser module so this module — generic text
/// chunking — stays free of a dependency on document-format extraction.
/// Bumping EITHER that or `CHUNKING_CONFIG_VERSION` changes every id it
/// touches, independently of the other: an extraction fix (e.g. the
/// native/OCR threshold, or how page spans are computed) and a chunking
/// change (CHUNK_SIZE/OVERLAP/SEPARATORS) are different pipeline stages
/// with different change cadences, and conflating them into one counter
/// would make old ids less diagnostic when something changes.
pub fn provenance_id(content_sha256: &str, chunk_index: usize, extraction_config_version: u32) -> String {
    format!("{content_sha256}:{chunk_index}:pv{extraction_config_version}:cv{CHUNKING_CONFIG_VERSION}")
}

/// One chunk of text plus its byte-offset span `[start_byte, end_byte)`
/// within the source text passed to `split_text` — the universal locator
/// (§ design note in api/documents.rs): available for every format, since
/// every parser already produces a single flat extracted-text string
/// before this module ever sees it. BYTE offsets, not char offsets —
/// named accordingly rather than left implicit, since this codebase
/// already has a documented history of char-vs-byte mixups (see `clen`
/// below): O(1) to compute via pointer arithmetic on the original
/// allocation (see `offset_of`), where a char-index scheme would need an
/// O(n) count per chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub text: String,
    pub start_byte: usize,
    pub end_byte: usize,
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
                result.push(Chunk {
                    text: piece.to_owned(),
                    start_byte: start,
                    end_byte: start + piece.len(),
                });
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
        Chunk { text, start_byte: start, end_byte: end }
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
        chunks.push(Chunk {
            text: s,
            start_byte: base + byte_at[start],
            end_byte: base + byte_at[end],
        });
        if end == chars.len() {
            break;
        }
        start = end - CHUNK_OVERLAP;
    }
    chunks
}

// ── Structural heading context ──────────────────────────────────────────
//
// Chunk boundaries are byte-count-driven and know nothing about document
// structure, so a chunk can easily fall entirely inside the BODY of e.g.
// "Article 100" without containing that heading itself — the heading landed
// a chunk or two earlier. Retrieval then hands the LLM a fragment like "...
// shall be subject to administrative fines of up to EUR 1 500 000" with no
// indication of which article it belongs to, which is how two independently
// built RAG stacks (this one and the old Python one) both misattributed the
// EU AI Act's Article 99/100/101 penalty clauses to the wrong article.
//
// The fix: detect short, standalone structural-heading lines ("Article 99",
// "Chapter XII", ...) and prepend the nearest PRECEDING one to any chunk
// that doesn't already start with it. This only touches the text that gets
// embedded/stored (see `inject_heading_context`) — `Chunk.start_byte`/
// `end_byte` themselves are untouched, so page numbers and citation spans
// keep pointing at the real source location, not the injected label.

/// A structural heading found in the source text, with the byte offset of
/// its own line (in the ORIGINAL text, matching `Chunk::start_byte`'s
/// coordinate space) and the label to inject.
#[derive(Debug)]
struct HeadingMarker {
    byte_offset: usize,
    label: String,
}

/// Recognised as a structural heading when a line starts with one of these
/// (case-insensitively) followed immediately by a digit or an uppercase
/// letter (roman numerals). Deliberately a small, explicit list rather than
/// a generic "short capitalised line" heuristic — precision matters more
/// than recall here, since a wrong injected heading actively misleads
/// rather than just failing to help. Covers the structural units common to
/// contracts, regulations and specs, not just this one EU regulation.
const HEADING_PREFIXES: &[&str] = &[
    "article ", "articolo ", "art. ",
    "chapter ", "capitolo ", "cap. ",
    "section ", "sezione ", "sec. ",
    "annex ", "allegato ",
    "clause ", "clausola ",
];

/// Standalone heading lines are short by nature ("Article 99", "Chapter
/// XII") — this bound is what excludes a sentence that merely starts with
/// the same word ("Article 99 provides that operators shall..." is prose,
/// not a heading, and is long enough to fail this check regardless of its
/// opening words).
const MAX_HEADING_LINE_CHARS: usize = 60;
/// A heading's title line ("Penalties" following "Article 99") is also
/// short; this bound keeps a heading from accidentally swallowing the
/// start of actual body text as its "title".
const MAX_TITLE_LINE_CHARS: usize = 120;

fn is_heading_line(line: &str) -> bool {
    if line.is_empty() || line.chars().count() > MAX_HEADING_LINE_CHARS {
        return false;
    }
    HEADING_PREFIXES.iter().any(|&prefix| {
        line.len() >= prefix.len()
            && line[..prefix.len()].eq_ignore_ascii_case(prefix)
            && line[prefix.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
    })
}

/// Whether `line` looks like the start of a numbered body paragraph ("1. "
/// or "1) "), as opposed to a heading's title line — used to stop
/// `detect_headings` from swallowing real content as a "title".
fn looks_like_numbered_paragraph(line: &str) -> bool {
    let digits_end = line.find(|c: char| !c.is_ascii_digit()).unwrap_or(line.len());
    digits_end > 0 && matches!(line[digits_end..].chars().next(), Some('.') | Some(')'))
}

/// Scan `text` for structural headings, in ascending byte order. Each
/// heading also absorbs the following line as its title when that line is
/// short and doesn't itself look like body text — "Article 99" followed by
/// "Penalties" becomes the single label "Article 99 — Penalties".
fn detect_headings(text: &str) -> Vec<HeadingMarker> {
    let mut lines: Vec<(usize, &str)> = Vec::new();
    let mut offset = 0usize;
    for raw_line in text.split_inclusive('\n') {
        let line = raw_line.trim_end_matches('\n').trim();
        lines.push((offset, line));
        offset += raw_line.len();
    }

    let mut markers = Vec::new();
    for (i, &(line_offset, line)) in lines.iter().enumerate() {
        if !is_heading_line(line) {
            continue;
        }
        let mut label = line.to_string();
        if let Some(&(_, title)) = lines.get(i + 1) {
            if !title.is_empty()
                && title.chars().count() <= MAX_TITLE_LINE_CHARS
                && !looks_like_numbered_paragraph(title)
                && !is_heading_line(title)
            {
                label = format!("{label} — {title}");
            }
        }
        markers.push(HeadingMarker { byte_offset: line_offset, label });
    }
    markers
}

/// Returns one String per chunk (same order as `chunks`): the chunk's own
/// text, prefixed with `[nearest preceding heading]\n\n` when that heading
/// isn't already how the chunk itself starts. A document with no detected
/// headings — prose, a spreadsheet dump, anything non-legal-structured —
/// makes every call a no-op, returning each chunk's text unchanged.
pub fn inject_heading_context(text: &str, chunks: &[Chunk]) -> Vec<String> {
    let headings = detect_headings(text);
    if headings.is_empty() {
        return chunks.iter().map(|c| c.text.clone()).collect();
    }
    chunks
        .iter()
        .map(|chunk| {
            match headings.iter().rev().find(|h| h.byte_offset <= chunk.start_byte) {
                Some(h) => {
                    let heading_only = h.label.split(" — ").next().unwrap_or(&h.label);
                    if chunk.text.trim_start().starts_with(heading_only) {
                        chunk.text.clone()
                    } else {
                        format!("[{}]\n\n{}", h.label, chunk.text)
                    }
                }
                None => chunk.text.clone(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every chunk's [start_byte, end_byte) must slice out of `text` to
    /// exactly its own `text` field — the property the whole
    /// pointer-arithmetic scheme exists to guarantee. Checked once,
    /// generically, instead of repeating it by hand in every test below.
    fn assert_spans_correct(source: &str, chunks: &[Chunk]) {
        for (i, c) in chunks.iter().enumerate() {
            assert!(c.start_byte <= c.end_byte, "chunk {i}: start_byte > end_byte");
            assert!(c.end_byte <= source.len(), "chunk {i}: end_byte past source length");
            assert_eq!(
                &source[c.start_byte..c.end_byte],
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
        let a = provenance_id("abc123", 5, 1);
        let b = provenance_id("abc123", 5, 1);
        assert_eq!(a, b, "same content hash + chunk_index + versions must yield the same provenance_id");

        let different_chunk = provenance_id("abc123", 6, 1);
        assert_ne!(a, different_chunk, "different chunk_index must yield a different provenance_id");

        let different_content = provenance_id("def456", 5, 1);
        assert_ne!(a, different_content, "different content hash must yield a different provenance_id");

        let different_extraction_version = provenance_id("abc123", 5, 2);
        assert_ne!(
            a, different_extraction_version,
            "different extraction_config_version must yield a different provenance_id, \
             independently of CHUNKING_CONFIG_VERSION"
        );

        assert!(a.contains(&format!("cv{CHUNKING_CONFIG_VERSION}")));
        assert!(a.contains("pv1"));
        assert!(different_extraction_version.contains("pv2"));
    }

    #[test]
    fn detect_headings_finds_article_and_captures_title() {
        let text = "Article 99\nPenalties\n\n1. In accordance with this Regulation...";
        let headings = detect_headings(text);
        assert_eq!(headings.len(), 1);
        assert_eq!(headings[0].byte_offset, 0);
        assert_eq!(headings[0].label, "Article 99 — Penalties");
    }

    #[test]
    fn detect_headings_recognises_chapter_roman_numerals_and_other_languages() {
        let text = "CHAPTER XII\nPENALTIES\n\nArticolo 99\nSanzioni\n\n1. Testo qui.";
        let headings = detect_headings(text);
        assert_eq!(headings.len(), 2, "expected CHAPTER XII and Articolo 99, got {headings:?}",);
        assert_eq!(headings[0].label, "CHAPTER XII — PENALTIES");
        assert_eq!(headings[1].label, "Articolo 99 — Sanzioni");
    }

    #[test]
    fn detect_headings_ignores_mid_sentence_mentions() {
        let text = "This clause refers back to Article 99 of the Regulation in passing, \
                     as part of a longer sentence that is clearly not a standalone heading line.";
        assert!(detect_headings(text).is_empty(), "a long prose line must not be mistaken for a heading");
    }

    #[test]
    fn detect_headings_does_not_swallow_numbered_body_text_as_a_title() {
        // No blank/title line between "Article 5" and its first numbered
        // paragraph: "1. Foo" must NOT be absorbed into the label.
        let text = "Article 5\n1. The following AI practices shall be prohibited...";
        let headings = detect_headings(text);
        assert_eq!(headings.len(), 1);
        assert_eq!(headings[0].label, "Article 5", "numbered paragraph text must not become the title");
    }

    #[test]
    fn inject_heading_context_is_noop_without_any_headings() {
        let text = "word ".repeat(400);
        let text = text.trim();
        let chunks = split_text(text);
        let injected = inject_heading_context(text, &chunks);
        assert_eq!(injected.len(), chunks.len());
        for (c, i) in chunks.iter().zip(&injected) {
            assert_eq!(&c.text, i, "no headings in the document must leave every chunk untouched");
        }
    }

    #[test]
    fn inject_heading_context_skips_chunk_that_already_starts_with_its_own_heading() {
        let text = "Article 99\nPenalties\n\n1. Short paragraph.";
        let chunks = split_text(text);
        assert_eq!(chunks.len(), 1);
        let injected = inject_heading_context(text, &chunks);
        assert_eq!(injected[0], chunks[0].text, "a chunk that already opens with its heading must not be double-prefixed");
    }

    #[test]
    fn inject_heading_context_prefixes_orphaned_chunk_with_nearest_preceding_heading() {
        // A short heading followed by enough body text that a second,
        // 1000-char chunk starts WITHOUT the heading line in it.
        let body = "This is filler body text about the article's substance. ".repeat(30);
        let text = format!("Article 42\nSome Title\n\n{}", body.trim());
        let chunks = split_text(&text);
        assert!(chunks.len() >= 2, "expected the filler to force a second chunk");
        assert!(!chunks[1].text.contains("Article 42"), "test setup: chunk 1 must be the orphaned one");

        let injected = inject_heading_context(&text, &chunks);
        assert_eq!(injected[0], chunks[0].text, "chunk 0 already has its own heading");
        assert!(
            injected[1].starts_with("[Article 42 — Some Title]\n\n"),
            "orphaned chunk 1 must be prefixed with the nearest preceding heading, got: {:?}",
            &injected[1][..injected[1].len().min(80)]
        );
        assert!(injected[1].ends_with(chunks[1].text.as_str()), "injection must not alter the chunk's own text, only prefix it");
    }

    /// Regression test for the actual failure this feature exists to fix:
    /// verbatim text of EU AI Act Article 100 (transcribed from the real
    /// Official Journal PDF page). Its own EUR 1 500 000 / EUR 750 000
    /// figures land in a later chunk that does NOT contain the "Article
    /// 100" heading — exactly the case that made two independent RAG
    /// stacks misattribute this article's fines to Article 99 or 101.
    #[test]
    fn real_article_100_text_orphaned_fine_amounts_get_heading_injected() {
        let text = "Article 100\n\
Administrative fines on Union institutions, bodies, offices and agencies\n\n\
1. The European Data Protection Supervisor may impose administrative fines on Union institutions, bodies, offices and agencies falling within the scope of this Regulation. When deciding whether to impose an administrative fine and when deciding on the amount of the administrative fine in each individual case, all relevant circumstances of the specific situation shall be taken into account and due regard shall be given to the following:\n\n\
(a) the nature, gravity and duration of the infringement and of its consequences, taking into account the purpose of the AI system concerned, as well as, where appropriate, the number of affected persons and the level of damage suffered by them;\n\n\
(b) the degree of responsibility of the Union institution, body, office or agency, taking into account technical and organisational measures implemented by them;\n\n\
(c) any action taken by the Union institution, body, office or agency to mitigate the damage suffered by affected persons;\n\n\
(d) the degree of cooperation with the European Data Protection Supervisor in order to remedy the infringement and mitigate the possible adverse effects of the infringement, including compliance with any of the measures previously ordered by the European Data Protection Supervisor against the Union institution, body, office or agency concerned with regard to the same subject matter;\n\n\
(e) any similar previous infringements by the Union institution, body, office or agency;\n\n\
(f) the manner in which the infringement became known to the European Data Protection Supervisor, in particular whether, and if so to what extent, the Union institution, body, office or agency notified the infringement;\n\n\
(g) the annual budget of the Union institution, body, office or agency.\n\n\
2. Non-compliance with the prohibition of the AI practices referred to in Article 5 shall be subject to administrative fines of up to EUR 1 500 000.\n\n\
3. The non-compliance of the AI system with any requirements or obligations under this Regulation, other than those laid down in Article 5, shall be subject to administrative fines of up to EUR 750 000.\n\n\
4. Before taking decisions pursuant to this Article, the European Data Protection Supervisor shall give the Union institution, body, office or agency which is the subject of the proceedings conducted by the European Data Protection Supervisor the opportunity of being heard on the matter regarding the possible infringement.";

        let chunks = split_text(text);
        assert!(chunks.len() >= 2, "expected Article 100's real length to force multiple chunks");

        let fines_chunk_idx = chunks
            .iter()
            .position(|c| c.text.contains("EUR 1 500 000") && c.text.contains("EUR 750 000"))
            .expect("a chunk containing both real fine amounts must exist");
        assert!(
            !chunks[fines_chunk_idx].text.contains("Article 100"),
            "test precondition: the fines chunk must NOT already contain its own heading \
             (otherwise this isn't reproducing the bug)"
        );

        let injected = inject_heading_context(text, &chunks);
        assert!(
            injected[fines_chunk_idx].starts_with("[Article 100 — Administrative fines on Union institutions, bodies, offices and agencies]\n\n"),
            "the chunk carrying the real EUR 1 500 000 / EUR 750 000 figures must now be \
             traceable back to Article 100, got: {:?}",
            &injected[fines_chunk_idx][..injected[fines_chunk_idx].len().min(120)]
        );
    }
}
