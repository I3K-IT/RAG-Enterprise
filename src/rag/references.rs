//! Articles a question names by number, found by name rather than by meaning.
//!
//! Vector search ranks by meaning, and a number carries almost none: asked
//! "Che cosa prevede l'art. 2875 del codice civile?", bge-m3 returned
//! articles 2475, 2645-bis and 2645-ter, and the model rightly said the
//! article was not among its sources. On the 472-question statute exam of
//! the eullm project the article asked was among the sources for 37% of
//! questions (2026-10-10). A lawyer asks by number all the time.
//!
//! So when a question names "art. N" — of a code, if it names one — the
//! documents whose file name is that article are looked up directly and
//! their chunks placed before the vector results. The file names follow
//! the LEGAL_PACK convention, one article per file named as it is cited:
//! "Codice civile, art. 2875 - Rubrica.txt". A file named otherwise never
//! matches, so a library without such files behaves exactly as before.

use std::sync::LazyLock;

use regex::Regex;

/// The nine Latin suffixes Italian legislation uses: "art. 360-bis".
const SUFFIXES: &str = "bis|ter|quater|quinquies|sexies|septies|octies|novies|decies";

/// How many referenced documents one question may pull in: a question naming
/// "art. 54" with no code matches the art. 54 of every code in the library.
pub const MAX_REFERENCED: usize = 4;

/// How the codes are named in questions (normalised: lowercase, punctuation
/// as spaces), longest first, mapped to the name their files start with.
/// "c p" must come after "c p c", "c p p" and "c p a", which contain it.
const CODE_ALIASES: &[(&str, &str)] = &[
    (
        "codice del processo amministrativo",
        "Codice del processo amministrativo",
    ),
    (
        "processo amministrativo",
        "Codice del processo amministrativo",
    ),
    ("codice di procedura civile", "Codice di procedura civile"),
    ("codice di procedura penale", "Codice di procedura penale"),
    ("legge sul procedimento amministrativo", "Legge n. 241-1990"),
    ("codice del consumo", "Codice del consumo"),
    ("codice civile", "Codice civile"),
    ("codice penale", "Codice penale"),
    ("costituzione", "Costituzione"),
    ("procedura civile", "Codice di procedura civile"),
    ("procedura penale", "Codice di procedura penale"),
    ("241 1990", "Legge n. 241-1990"),
    ("1199 1971", "D.P.R. n. 1199-1971"),
    ("104 2010", "Codice del processo amministrativo"),
    ("cod civ", "Codice civile"),
    ("cod pen", "Codice penale"),
    ("cod cons", "Codice del consumo"),
    ("c p a", "Codice del processo amministrativo"),
    ("c p c", "Codice di procedura civile"),
    ("c p p", "Codice di procedura penale"),
    ("c c", "Codice civile"),
    ("c p", "Codice penale"),
    ("cost", "Costituzione"),
];

/// "art. 2875", "articolo 360-bis", every number of "artt. 1176 e 1375" —
/// but not "art. 1453 e 3 mesi" or "art. 360, 1° comma": a number with a
/// unit or an ordinal after it is a quantity, not a second article.
static ARTICLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\bart(?:icol[oi]|t)?\.?\s*(\d+)(?:[\s-]*({SUFFIXES}))?\b((?:\s*(?:,|\be\b|\bed\b)\s*\d+(?:[\s-]*(?:{SUFFIXES}))?\b(?:\s*(?:°|º|comm|co\.|giorn|mes|ann|or[ae]\b))?)*)"
    ))
    .expect("article pattern")
});
static MORE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)(?:,|\be\b|\bed\b)\s*(\d+)(?:[\s-]*({SUFFIXES}))?\b(\s*(?:°|º|comm|co\.|giorn|mes|ann|or[ae]\b))?"
    ))
    .expect("list pattern")
});
/// The article a file is: the first "art. N" before the " - " that starts the
/// rubrica.
static FILE_ARTICLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"(?i)\bart\.?\s*(\d+)(?:[\s-]*({SUFFIXES}))?\b"))
        .expect("file article pattern")
});

/// One article a question names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArticleRef {
    /// As the files spell it: "2875", "360-bis".
    pub number: String,
    /// The name the code's files start with ("Codice civile"), when the
    /// question names a code.
    pub code: Option<&'static str>,
}

fn key(num: &str, suffix: Option<&str>) -> String {
    let n = num.trim_start_matches('0');
    let n = if n.is_empty() { "0" } else { n };
    match suffix {
        Some(s) => format!("{n}-{}", s.to_lowercase()),
        None => n.to_owned(),
    }
}

fn normalise(text: &str) -> String {
    let lowered: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    format!(
        " {} ",
        lowered.split_whitespace().collect::<Vec<_>>().join(" ")
    )
}

/// The code a question names, as its files are named.
pub fn named_code(question: &str) -> Option<&'static str> {
    let norm = normalise(question);
    CODE_ALIASES
        .iter()
        .find(|(alias, _)| norm.contains(&format!(" {alias} ")))
        .map(|(_, name)| *name)
}

/// The articles a question names, in order, without repeats.
pub fn named_articles(question: &str) -> Vec<ArticleRef> {
    let code = named_code(question);
    let mut out: Vec<ArticleRef> = Vec::new();
    let mut push = |number: String| {
        if !out.iter().any(|r| r.number == number) {
            out.push(ArticleRef { number, code });
        }
    };
    for m in ARTICLE.captures_iter(question) {
        push(key(&m[1], m.get(2).map(|s| s.as_str())));
        if let Some(rest) = m.get(3) {
            for more in MORE.captures_iter(rest.as_str()) {
                if more.get(3).is_none() {
                    push(key(&more[1], more.get(2).map(|s| s.as_str())));
                }
            }
        }
    }
    out
}

/// Whether a file is the article referenced: its name, up to the rubrica,
/// names that article number and, when the question names a code, starts
/// with that code's name ("Codice civile, art. 2875 - ...").
pub fn file_is(filename: &str, r: &ArticleRef) -> bool {
    let head = filename.split(" - ").next().unwrap_or(filename);
    let head = head.strip_suffix(".txt").unwrap_or(head);
    let Some(m) = FILE_ARTICLE.captures(head) else {
        return false;
    };
    if key(&m[1], m.get(2).map(|s| s.as_str())) != r.number {
        return false;
    }
    match r.code {
        Some(code) => head
            .to_lowercase()
            .starts_with(&format!("{}, ", code.to_lowercase())),
        None => true,
    }
}

/// The two LIKE patterns that find a referenced article's file by name
/// before `file_is` decides: "art. 2875 - <rubrica>" or "art. 2875" alone
/// at the end of the name. Exact on the number, so "art. 1" does not
/// match the art. 10, 100 and 1000 of every code.
pub fn like_patterns(r: &ArticleRef) -> [String; 2] {
    [
        format!("%art. {} - %", r.number),
        format!("%art. {}.txt", r.number),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(q: &str) -> Vec<(String, Option<&'static str>)> {
        named_articles(q)
            .into_iter()
            .map(|r| (r.number, r.code))
            .collect()
    }

    #[test]
    fn a_question_names_its_article_and_code() {
        assert_eq!(
            refs("Che cosa prevede l'art. 2875 del codice civile?"),
            vec![("2875".into(), Some("Codice civile"))]
        );
        assert_eq!(
            refs("Cosa dice l'art. 327 c.p.c.?"),
            vec![("327".into(), Some("Codice di procedura civile"))]
        );
        assert_eq!(
            refs("Che cosa prevede l'art. 21-octies della legge n. 241/1990?"),
            vec![("21-octies".into(), Some("Legge n. 241-1990"))]
        );
        assert_eq!(
            refs("Che cosa prevede l'articolo 27 della Costituzione?")[0].1,
            Some("Costituzione")
        );
        // "c.p." must not swallow "c.p.c."
        assert_eq!(
            named_code("art. 360 c.p.c."),
            Some("Codice di procedura civile")
        );
        assert_eq!(named_code("art. 54 c.p."), Some("Codice penale"));
    }

    #[test]
    fn lists_are_read_and_quantities_are_not() {
        let nums = |q: &str| refs(q).into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(nums("gli artt. 1176 e 1375 c.c."), vec!["1176", "1375"]);
        assert_eq!(
            nums("artt. 2043, 2059 e 2087 c.c."),
            vec!["2043", "2059", "2087"]
        );
        assert_eq!(nums("l'art. 1453 e 3 mesi"), vec!["1453"]);
        assert_eq!(nums("l'art. 360, 1° comma, n. 3 c.p.c."), vec!["360"]);
        assert_eq!(
            nums("in materia di stato di necessità"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_file_is_the_article_only_by_its_own_name() {
        let r = |n: &str, c| ArticleRef {
            number: n.into(),
            code: c,
        };
        let f = "Codice civile, art. 2875 - Riduzione dell'ipoteca.txt";
        assert!(file_is(f, &r("2875", Some("Codice civile"))));
        assert!(file_is(f, &r("2875", None)));
        assert!(
            !file_is(f, &r("287", Some("Codice civile"))),
            "a prefix is not the article"
        );
        assert!(!file_is(f, &r("2875", Some("Codice penale"))));
        assert!(!file_is(
            "Codice civile, art. 2875-bis - X.txt",
            &r("2875", None)
        ));
        assert!(file_is(
            "Codice civile, art. 2875-bis - X.txt",
            &r("2875-bis", None)
        ));
        // a rubrica that cites another article does not make the file that one
        assert!(!file_is(
            "Codice civile, art. 12 - Rinvio all'art. 2875.txt",
            &r("2875", None)
        ));
        // a document not named after an article never matches
        assert!(!file_is(
            "Contratto di locazione 2875.pdf",
            &r("2875", None)
        ));
    }

    #[test]
    fn the_like_patterns_are_exact_on_the_number() {
        let r = ArticleRef {
            number: "360-bis".into(),
            code: None,
        };
        assert_eq!(
            like_patterns(&r),
            ["%art. 360-bis - %".to_owned(), "%art. 360-bis.txt".into()]
        );
    }
}
