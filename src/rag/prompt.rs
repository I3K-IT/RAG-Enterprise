//! RAG prompt: the administrator's instructions plus a frame the code owns.
//!
//! Two parts. The instructions say how to answer - role, structure, how to
//! cite, language - and an administrator can replace them from the admin
//! panel (db::settings::ANSWER_INSTRUCTIONS); until they do, or when what they
//! saved is blank, DEFAULT_INSTRUCTIONS apply. The frame says what the answer
//! is about - the history, the retrieved chunks, the question - and is not
//! editable: retrieval fills it, and an answer without it has nothing to go
//! on.
//!
//! No chat template and no model-specific switch here: eullm applies the
//! loaded model's own format, and thinking is turned off in the request
//! (clients::eullm).

/// How to answer, until an administrator says otherwise.
pub const DEFAULT_INSTRUCTIONS: &str = r#"You are an expert research analyst. Answer based on the data below.

INSTRUCTIONS:
1. LANGUAGE: Respond in the SAME LANGUAGE as the question.
2. STRUCTURE: Start with a direct answer, then provide detailed supporting information with dates, names, locations, context, and explanations.
3. VERIFIED DATA FIRST: Use STRUCTURED DATA as verified facts (dates, event types). Use RETRIEVED CHUNKS for additional context and details.
4. CITATIONS: After each fact, cite its source by the file name shown in square brackets above the chunk it comes from, copied exactly; cite only those file names; never write a placeholder, a shorthand such as “ibidem”, or a name that is not among them.
5. SEMANTIC UNDERSTANDING: Recognize that terms may appear in different languages or synonyms (e.g., "attentato" = "attack" = "bombing").
6. THOROUGH: Provide rich, detailed answers. Extract ALL relevant information from the chunks - don't summarize, elaborate. Include background, consequences, related events.
7. NO INVENTION: Only use information present in the data. If inferring a connection, say "possibly related" or "may be connected"."#;

/// The longest instructions an administrator may save, in characters. Every
/// question pays for them in context, before a single chunk is added.
pub const MAX_INSTRUCTIONS_CHARS: usize = 8000;

/// The part of the prompt the code owns. `{instructions}` is filled with a
/// value like the other slots, so whatever the instructions contain -
/// `{context}` and `{question}` included - stays text (see `render`).
const FRAME: &str = r#"{instructions}

{structured_data_section}

{history_section}

RETRIEVED CHUNKS:
{context}

QUESTION: {question}

{language_reminder}ANSWER:"#;

/// Closes the prompt when the default instructions are in effect: their
/// language rule again, last thing before ANSWER:. With English instructions
/// a model like Qwen3.5-9B answered Italian questions in English, the rule
/// being one point among seven above pages of chunks. Not added to an
/// administrator's own instructions: language is theirs to set, and a
/// closing line saying otherwise would overrule "always answer in English".
const LANGUAGE_REMINDER: &str = "Answer in the language of the QUESTION.\n\n";

/// The instructions to use: `custom` when it has any text, the default
/// otherwise.
pub fn effective_instructions(custom: Option<&str>) -> &str {
    match custom.map(str::trim) {
        Some(text) if !text.is_empty() => text,
        _ => DEFAULT_INSTRUCTIONS,
    }
}

/// Format the history section (last 3 exchanges, 800-char truncation per message).
/// Mirrors the history formatting of the Python implementation.
pub fn format_history(history: &[(String, String)]) -> String {
    let last3: Vec<_> = history.iter().rev().take(3).collect();
    if last3.is_empty() {
        return String::new();
    }
    let mut lines = vec!["CONVERSATION HISTORY:".to_owned()];
    for (user_msg, assistant_msg) in last3.iter().rev() {
        let u = truncate(user_msg, 800);
        let a = truncate(assistant_msg, 800);
        lines.push(format!("User: {u}"));
        lines.push(format!("Assistant: {a}"));
    }
    lines.join("\n")
}

/// Truncates to `max_chars` Unicode CHARACTERS, matching Python's `s[:800]`,
/// which counts code points rather than bytes.
///
/// SECURITY: an earlier version sliced by byte with `&s[..max]`, which panics
/// when `max` lands in the middle of a multibyte character — trivial to
/// trigger with accented characters in a stored message longer than 800 bytes,
/// and reachable from the main query path. `chars().take()` can never split a
/// character.
fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Build the full prompt string. `instructions` is what the administrator
/// saved, if anything; blank or absent means DEFAULT_INSTRUCTIONS.
pub fn build_prompt(
    context: &str,
    question: &str,
    history: &[(String, String)],
    instructions: Option<&str>,
) -> String {
    let history_section = format_history(history);
    let instructions = effective_instructions(instructions);
    let language_reminder =
        if instructions == DEFAULT_INSTRUCTIONS { LANGUAGE_REMINDER } else { "" };
    render(
        FRAME,
        &[
            ("{instructions}", instructions),
            ("{language_reminder}", language_reminder),
            ("{structured_data_section}", ""),
            ("{history_section}", &history_section),
            ("{context}", context),
            ("{question}", question),
        ],
    )
}

/// Substitutes each `key` in `template` with its value, in one pass.
///
/// Deliberately not a chain of `str::replace` calls. Each of those scans the
/// string produced by the previous one, so a value that itself contains a
/// later key is substituted too: a document whose text contains the literal
/// `{question}` — a template guide, a config file, this very project — had
/// that token replaced by the user's question, inside the evidence. The
/// retrieved chunks stopped being what the document actually says, and the
/// `{ context }`/`{ question }` markers of an injected history could be
/// rewritten the same way.
///
/// Here `rest` only ever points into `template`, never into what has already
/// been emitted, so an inserted value is appended to the output and never
/// looked at again. Within the template the first occurrence of a key wins,
/// which is the one the author wrote.
fn render(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    // The key that comes first in what is left, whichever it is in the list:
    // taking keys in list order would copy an earlier, different key past it
    // verbatim, leaving that slot unfilled.
    while let Some((at, key, value)) = values
        .iter()
        .filter_map(|&(key, value)| rest.find(key).map(|at| (at, key, value)))
        .min_by_key(|&(at, _, _)| at)
    {
        out.push_str(&rest[..at]);
        out.push_str(value);
        rest = &rest[at + key.len()..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_does_not_panic_on_multibyte_boundary() {
        // 800 accented characters = 1600 bytes: the old byte slicing
        // `&s[..800]` used to land mid-way through 'à' → panic. It must cut cleanly.
        let s: String = "à".repeat(1000);
        let out = truncate(&s, 800);
        assert_eq!(out.chars().count(), 800);
        assert!(out.chars().all(|c| c == 'à'));
    }

    #[test]
    fn truncate_counts_chars_not_bytes() {
        // Parity with Python's s[:800]: counts code points, not bytes.
        let s: String = "é".repeat(500); // 500 char, 1000 byte
        assert_eq!(truncate(&s, 800), s); // below threshold in CHARs → unchanged
    }

    #[test]
    fn truncate_short_ascii_unchanged() {
        assert_eq!(truncate("hello", 800), "hello");
    }

    #[test]
    fn format_history_survives_long_accented_message() {
        // The real path: a long, accented stored message must not panic.
        let long = "à".repeat(2000);
        let hist = vec![(long.clone(), long)];
        let out = format_history(&hist);
        assert!(out.contains("CONVERSATION HISTORY:"));
    }

    /// The shape the template is supposed to produce, pinned so the one-pass
    /// substitution below cannot quietly change the prompt.
    #[test]
    fn the_prompt_is_the_template_with_its_four_slots_filled() {
        let out = build_prompt("[report.pdf]\nRevenue grew.", "How much?", &[], None);
        assert!(!out.contains("{structured_data_section}"), "{out}");
        assert!(!out.contains("{history_section}"), "{out}");
        assert!(
            out.contains("RETRIEVED CHUNKS:\n[report.pdf]\nRevenue grew."),
            "{out}"
        );
        assert!(out.contains("QUESTION: How much?"), "{out}");
        assert!(
            out.starts_with("You are an expert research analyst."),
            "{out}"
        );
        assert!(out.ends_with("ANSWER:"), "{out}");
    }

    /// The whole default prompt, pinned: what the model gets with nothing
    /// saved. Any change to the defaults or the frame shows up here first.
    #[test]
    fn with_nothing_saved_the_prompt_is_the_default_one() {
        let history = vec![("Earlier?".to_owned(), "Yes.".to_owned())];
        let expected = format!(
            "{DEFAULT_INSTRUCTIONS}\n\n\n\nCONVERSATION HISTORY:\nUser: Earlier?\nAssistant: Yes.\n\n\
             RETRIEVED CHUNKS:\n[a.pdf]\nText.\n\nQUESTION: Why?\n\n\
             Answer in the language of the QUESTION.\n\nANSWER:"
        );
        assert_eq!(build_prompt("[a.pdf]\nText.", "Why?", &history, None), expected);
    }

    /// Two findings of the evaluation, pinned. The citation line carries no
    /// example, which models copied as a citation ([annual-report-2025.pdf]);
    /// and the language rule comes first, then again right before ANSWER:.
    #[test]
    fn the_default_cites_without_an_example_and_states_the_language_twice() {
        let citations = DEFAULT_INSTRUCTIONS.lines().find(|l| l.contains("CITATIONS:")).unwrap();
        assert!(!citations.contains("e.g."), "{citations}");
        assert!(!citations.contains(".pdf"), "{citations}");
        let first = DEFAULT_INSTRUCTIONS.lines().find(|l| l.starts_with("1. ")).unwrap();
        assert!(first.starts_with("1. LANGUAGE:"), "{first}");
        for instructions in [None, Some(""), Some("  ")] {
            let out = build_prompt("c", "q", &[], instructions);
            assert!(out.ends_with("QUESTION: q\n\nAnswer in the language of the QUESTION.\n\nANSWER:"), "{out}");
        }
    }

    /// An administrator's instructions decide the language: no closing line
    /// may contradict "Always answer in English".
    #[test]
    fn custom_instructions_get_no_language_line_of_ours() {
        let out = build_prompt("c", "Che cosa dice?", &[], Some("Always answer in English."));
        assert!(!out.contains("Answer in the language of the QUESTION."), "{out}");
        assert!(out.ends_with("QUESTION: Che cosa dice?\n\nANSWER:"), "{out}");
    }

    #[test]
    fn custom_instructions_replace_the_default_and_the_frame_stays_in_order() {
        let history = vec![("Earlier?".to_owned(), "Yes.".to_owned())];
        let out = build_prompt("[a.pdf]\nText.", "Why?", &history, Some("Answer in one line."));
        assert!(out.starts_with("Answer in one line.\n\n"), "{out}");
        assert!(!out.contains("expert research analyst"), "{out}");
        let order = ["Answer in one line.", "CONVERSATION HISTORY:", "RETRIEVED CHUNKS:\n[a.pdf]\nText.", "QUESTION: Why?", "ANSWER:"];
        let mut at = 0;
        for part in order {
            let found = out[at..].find(part).unwrap_or_else(|| panic!("{part:?} missing or out of order in {out}"));
            at += found + part.len();
        }
        assert!(out.ends_with("ANSWER:"), "{out}");
    }

    #[test]
    fn placeholders_written_in_the_instructions_stay_text() {
        let out = build_prompt("the-evidence", "the-question?", &[], Some("Quote {context} and {question} literally."));
        assert!(out.starts_with("Quote {context} and {question} literally."), "{out}");
        assert_eq!(out.matches("the-evidence").count(), 1, "{out}");
        assert_eq!(out.matches("the-question?").count(), 1, "{out}");
    }

    #[test]
    fn blank_instructions_mean_the_default() {
        for blank in [Some(""), Some("   "), Some("\n\t \n"), None] {
            assert_eq!(effective_instructions(blank), DEFAULT_INSTRUCTIONS, "{blank:?}");
            assert!(build_prompt("c", "q", &[], blank).starts_with("You are an expert research analyst."));
        }
        assert_eq!(effective_instructions(Some("  Be brief.  ")), "Be brief.");
    }

    /// The bug: evidence is untrusted input, and it is substituted before the
    /// question. A chain of `replace` calls rewrote the token inside it.
    #[test]
    fn a_placeholder_inside_a_retrieved_chunk_is_left_alone() {
        let chunk = "To render a field use {context}, then {question}.";
        let out = build_prompt(chunk, "What is the revenue?", &[], None);
        assert!(
            out.contains("To render a field use {context}, then {question}."),
            "the chunk was rewritten: {out}"
        );
        assert!(
            out.contains("QUESTION: What is the revenue?"),
            "the template's own slot went unfilled: {out}"
        );
    }

    /// Same, one step earlier in the chain: a stored message that happens to
    /// contain a later key.
    #[test]
    fn a_placeholder_inside_the_history_is_left_alone() {
        let history = vec![(
            "How do I use {context}?".to_owned(),
            "It is filled in already.".to_owned(),
        )];
        let out = build_prompt("Revenue grew.", "And now?", &history, None);
        assert!(out.contains("User: How do I use {context}?"), "{out}");
        assert!(out.contains("Assistant: It is filled in already."), "{out}");
    }

    /// A value carrying a key for a slot that comes *before* it in the
    /// template must not be re-scanned either.
    #[test]
    fn a_value_containing_an_earlier_key_is_not_resubstituted() {
        let out = build_prompt("see {history_section}", "q", &[], None);
        assert!(out.contains("see {history_section}"), "{out}");
        assert!(!out.contains("CONVERSATION HISTORY"), "{out}");
    }

    /// A key absent from the template is simply not substituted, and text
    /// with no key at all is copied through untouched.
    #[test]
    fn render_copies_what_it_does_not_fill() {
        assert_eq!(render("a{b}c", &[("{b}", "B")]), "aBc");
        assert_eq!(render("abc", &[("{b}", "B")]), "abc");
        assert_eq!(render("a{b}c", &[("{z}", "Z")]), "a{b}c");
        assert_eq!(render("", &[("{b}", "B")]), "");
    }

    /// Keys out of the list's order are all filled: whichever comes first in
    /// the template is substituted first.
    #[test]
    fn keys_are_filled_in_the_order_they_appear_in_the_template() {
        assert_eq!(
            render(
                "Q: {question}\nC: {context}",
                &[("{context}", "CTX"), ("{question}", "Q?")]
            ),
            "Q: Q?\nC: CTX"
        );
    }
}
