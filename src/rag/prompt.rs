//! RAG prompt templates.
//!
//! Enterprise prompt verbatim da MAPPA §5 (da usare nel Community base — stessa struttura,
//! senza structured_data_section che nel Community è vuota).
//!
//! Golden-test: stesso prompt → stesso testo generato con lo stesso LLM.

pub const QA_PROMPT: &str = r#"/no_think
You are an expert research analyst. Answer based on the data below.

INSTRUCTIONS:
1. STRUCTURE: Start with a direct answer, then provide detailed supporting information with dates, names, locations, context, and explanations.
2. VERIFIED DATA FIRST: Use STRUCTURED DATA as verified facts (dates, event types). Use RETRIEVED CHUNKS for additional context and details.
3. CITATIONS: Cite sources as [Filename] after each fact.
4. SEMANTIC UNDERSTANDING: Recognize that terms may appear in different languages or synonyms (e.g., "attentato" = "attack" = "bombing").
5. THOROUGH: Provide rich, detailed answers. Extract ALL relevant information from the chunks - don't summarize, elaborate. Include background, consequences, related events.
6. LANGUAGE: Respond in the SAME LANGUAGE as the question.
7. NO INVENTION: Only use information present in the data. If inferring a connection, say "possibly related" or "may be connected".

{structured_data_section}

{history_section}

RETRIEVED CHUNKS:
{context}

QUESTION: {question}

ANSWER:"#;

/// Format the history section (last 3 exchanges, 800-char truncation per message).
/// Mirrors Python `_format_history` from rag_pipeline_enterprise.py.
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

/// Tronca a `max_chars` CARATTERI Unicode (parità con Python `s[:800]`, che
/// conta code point, non byte).
///
/// SICUREZZA: la versione precedente faceva slicing a byte `&s[..max]`, che va
/// in panic se `max` cade a metà di un carattere multibyte — banale da
/// innescare con le accentate italiane in un messaggio storico > 800 byte,
/// e raggiungibile nel percorso query principale. `chars().take()` non può
/// mai spezzare un carattere.
fn truncate(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

/// Build the full prompt string.
pub fn build_prompt(context: &str, question: &str, history: &[(String, String)]) -> String {
    let history_section = format_history(history);
    QA_PROMPT
        .replace("{structured_data_section}", "")
        .replace("{history_section}", &history_section)
        .replace("{context}", context)
        .replace("{question}", question)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_does_not_panic_on_multibyte_boundary() {
        // 800 caratteri accentati = 1600 byte: il vecchio slicing a byte
        // `&s[..800]` cadeva a metà di 'à' → panic. Deve troncare pulito.
        let s: String = "à".repeat(1000);
        let out = truncate(&s, 800);
        assert_eq!(out.chars().count(), 800);
        assert!(out.chars().all(|c| c == 'à'));
    }

    #[test]
    fn truncate_counts_chars_not_bytes() {
        // Parità Python s[:800]: conta code point, non byte.
        let s: String = "é".repeat(500); // 500 char, 1000 byte
        assert_eq!(truncate(&s, 800), s); // sotto soglia in CHAR → invariato
    }

    #[test]
    fn truncate_short_ascii_unchanged() {
        assert_eq!(truncate("hello", 800), "hello");
    }

    #[test]
    fn format_history_survives_long_accented_message() {
        // Percorso reale: messaggio storico lungo e accentato non deve panicare.
        let long = "à".repeat(2000);
        let hist = vec![(long.clone(), long)];
        let out = format_history(&hist);
        assert!(out.contains("CONVERSATION HISTORY:"));
    }
}
