//! Word lexicon: NGSL frequency bands + IPA + glosses (spec §7.4 / §7.7).
//!
//! The lexicon backs two validator checks:
//! * **band lookup** — is every word inside the level's vocab band (越级)?
//! * **IPA reconciliation** — lemma IPA from the lexicon overrides model
//!   output for exact surface matches (音标漂移归零).
//!
//! Source data lives in `content/lexicon/` as TSV
//! (`lemma \t band \t ipa_gb \t ipa_us \t zh_gloss`), derived from NGSL
//! (CC BY 3.0 — attribution shipped in-app, spec §4.9).
//!
//! Lookup normalizes the surface form, then tries: exact → irregular form →
//! suffix-stripping heuristics (plural/past/progressive/comparative).

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexEntry {
    pub lemma: String,
    /// 1-based frequency rank band (e.g. 500 means "within the first 500").
    pub band: u32,
    pub ipa_gb: String,
    pub ipa_us: String,
    pub zh_gloss: String,
}

#[derive(Debug, Default)]
pub struct Lexicon {
    entries: HashMap<String, LexEntry>,
    irregular: HashMap<&'static str, &'static str>,
}

impl Lexicon {
    /// Parse the TSV lexicon. Lines starting with `#` and blank lines are
    /// skipped; a malformed line is an error (factory data must be clean).
    pub fn from_tsv(tsv: &str) -> Result<Self, String> {
        let mut entries = HashMap::new();
        for (ln, line) in tsv.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 2 {
                return Err(format!(
                    "lexicon line {}: expected ≥2 tab-separated columns",
                    ln + 1
                ));
            }
            let lemma = cols[0].trim().to_lowercase();
            let band: u32 = cols[1]
                .trim()
                .parse()
                .map_err(|_| format!("lexicon line {}: bad band '{}'", ln + 1, cols[1]))?;
            entries.insert(
                lemma.clone(),
                LexEntry {
                    lemma,
                    band,
                    ipa_gb: cols.get(2).unwrap_or(&"").trim().to_string(),
                    ipa_us: cols.get(3).unwrap_or(&"").trim().to_string(),
                    zh_gloss: cols.get(4).unwrap_or(&"").trim().to_string(),
                },
            );
        }
        Ok(Self {
            entries,
            irregular: irregular_forms(),
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn normalize(word: &str) -> String {
        word.chars()
            .filter(|c| c.is_ascii_alphabetic() || *c == '\'')
            .collect::<String>()
            .to_lowercase()
    }

    /// Exact-surface entry (no lemmatization) — used for IPA reconciliation.
    pub fn exact(&self, word: &str) -> Option<&LexEntry> {
        self.entries.get(&Self::normalize(word))
    }

    /// Entry for a surface form, trying lemmatization fallbacks.
    pub fn lookup(&self, word: &str) -> Option<&LexEntry> {
        let w = Self::normalize(word);
        if w.is_empty() {
            return None;
        }
        if let Some(e) = self.entries.get(&w) {
            return Some(e);
        }
        // Contractions: I'm / we'll → try the part before the apostrophe;
        // n't forms (don't / isn't) additionally strip the fused "n".
        if let Some((head, _)) = w.split_once('\'') {
            if let Some(e) = self.entries.get(head) {
                return Some(e);
            }
            if let Some(stem) = w.strip_suffix("n't")
                && let Some(e) = self.entries.get(stem)
            {
                return Some(e);
            }
        }
        if let Some(&lemma) = self.irregular.get(w.as_str()) {
            return self.entries.get(lemma);
        }
        for candidate in strip_suffix_candidates(&w) {
            if let Some(e) = self.entries.get(candidate.as_str()) {
                return Some(e);
            }
        }
        None
    }

    /// Frequency band of a surface form, if known.
    pub fn band_of(&self, word: &str) -> Option<u32> {
        self.lookup(word).map(|e| e.band)
    }
}

/// Candidate lemmas from regular inflection suffixes, in priority order.
fn strip_suffix_candidates(w: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |s: String| {
        if s.len() >= 2 && !out.contains(&s) {
            out.push(s);
        }
    };
    // plural / 3sg: -s, -es, -ies→y
    if let Some(stem) = w.strip_suffix("ies") {
        push(format!("{stem}y"));
    }
    if let Some(stem) = w.strip_suffix("es") {
        push(stem.to_string());
    }
    if let Some(stem) = w.strip_suffix('s') {
        push(stem.to_string());
    }
    // past: -ed, -ied→y, -d, doubled consonant (stopped→stop)
    if let Some(stem) = w.strip_suffix("ied") {
        push(format!("{stem}y"));
    }
    if let Some(stem) = w.strip_suffix("ed") {
        push(stem.to_string());
        push(format!("{stem}e")); // liked → like
        if ends_with_double_consonant(stem) {
            push(stem[..stem.len() - 1].to_string());
        }
    }
    // progressive: -ing (+e restore, de-doubling)
    if let Some(stem) = w.strip_suffix("ing") {
        push(stem.to_string());
        push(format!("{stem}e")); // making → make
        if ends_with_double_consonant(stem) {
            push(stem[..stem.len() - 1].to_string());
        }
    }
    // comparative / superlative: -er/-est (+e restore, -ier/-iest→y)
    if let Some(stem) = w.strip_suffix("iest") {
        push(format!("{stem}y"));
    }
    if let Some(stem) = w.strip_suffix("ier") {
        push(format!("{stem}y"));
    }
    if let Some(stem) = w.strip_suffix("est") {
        push(stem.to_string());
        push(format!("{stem}e"));
    }
    if let Some(stem) = w.strip_suffix("er") {
        push(stem.to_string());
        push(format!("{stem}e"));
    }
    // adverb: -ly
    if let Some(stem) = w.strip_suffix("ly") {
        push(stem.to_string());
    }
    out
}

fn ends_with_double_consonant(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2
        && b[b.len() - 1] == b[b.len() - 2]
        && !matches!(b[b.len() - 1], b'a' | b'e' | b'i' | b'o' | b'u')
        && b[b.len() - 1].is_ascii_alphabetic()
}

/// Common irregular surface → lemma forms (verbs, plurals, comparatives,
/// pronouns' possessives are their own lexicon entries so not listed).
fn irregular_forms() -> HashMap<&'static str, &'static str> {
    let pairs: &[(&str, &str)] = &[
        // be family
        ("am", "be"),
        ("is", "be"),
        ("are", "be"),
        ("was", "be"),
        ("were", "be"),
        ("been", "be"),
        ("being", "be"),
        // frequent irregular verbs (past, participle)
        ("went", "go"),
        ("gone", "go"),
        ("did", "do"),
        ("done", "do"),
        ("does", "do"),
        ("had", "have"),
        ("has", "have"),
        ("said", "say"),
        ("made", "make"),
        ("got", "get"),
        ("gotten", "get"),
        ("took", "take"),
        ("taken", "take"),
        ("came", "come"),
        ("saw", "see"),
        ("seen", "see"),
        ("knew", "know"),
        ("known", "know"),
        ("thought", "think"),
        ("found", "find"),
        ("gave", "give"),
        ("given", "give"),
        ("told", "tell"),
        ("felt", "feel"),
        ("left", "leave"),
        ("put", "put"),
        ("kept", "keep"),
        ("let", "let"),
        ("began", "begin"),
        ("begun", "begin"),
        ("brought", "bring"),
        ("bought", "buy"),
        ("built", "build"),
        ("caught", "catch"),
        ("chose", "choose"),
        ("chosen", "choose"),
        ("cost", "cost"),
        ("cut", "cut"),
        ("drank", "drink"),
        ("drunk", "drink"),
        ("drove", "drive"),
        ("driven", "drive"),
        ("ate", "eat"),
        ("eaten", "eat"),
        ("fell", "fall"),
        ("fallen", "fall"),
        ("flew", "fly"),
        ("flown", "fly"),
        ("forgot", "forget"),
        ("forgotten", "forget"),
        ("grew", "grow"),
        ("grown", "grow"),
        ("heard", "hear"),
        ("held", "hold"),
        ("hit", "hit"),
        ("hurt", "hurt"),
        ("lost", "lose"),
        ("meant", "mean"),
        ("met", "meet"),
        ("paid", "pay"),
        ("read", "read"),
        ("ran", "run"),
        ("rang", "ring"),
        ("rung", "ring"),
        ("rose", "rise"),
        ("risen", "rise"),
        ("sang", "sing"),
        ("sung", "sing"),
        ("sat", "sit"),
        ("slept", "sleep"),
        ("spoke", "speak"),
        ("spoken", "speak"),
        ("spent", "spend"),
        ("stood", "stand"),
        ("swam", "swim"),
        ("swum", "swim"),
        ("taught", "teach"),
        ("threw", "throw"),
        ("thrown", "throw"),
        ("understood", "understand"),
        ("wore", "wear"),
        ("worn", "wear"),
        ("won", "win"),
        ("wrote", "write"),
        ("written", "write"),
        ("sent", "send"),
        ("sold", "sell"),
        ("shut", "shut"),
        ("spoiled", "spoil"),
        ("woke", "wake"),
        ("woken", "wake"),
        ("became", "become"),
        ("become", "become"),
        ("sought", "seek"),
        ("dealt", "deal"),
        ("led", "lead"),
        // irregular plurals
        ("children", "child"),
        ("men", "man"),
        ("women", "woman"),
        ("people", "person"),
        ("feet", "foot"),
        ("teeth", "tooth"),
        ("mice", "mouse"),
        ("lives", "life"),
        ("wives", "wife"),
        ("knives", "knife"),
        ("leaves", "leaf"),
        ("shelves", "shelf"),
        ("halves", "half"),
        // irregular comparatives / adverbs
        ("better", "good"),
        ("best", "good"),
        ("worse", "bad"),
        ("worst", "bad"),
        ("more", "many"),
        ("most", "many"),
        ("less", "little"),
        ("least", "little"),
        ("further", "far"),
        ("farther", "far"),
        // auxiliaries / contraction tails as lemmas of their full forms
        ("won't", "will"),
        ("can't", "can"),
        ("cannot", "can"),
        ("n't", "not"),
    ];
    pairs.iter().copied().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lex() -> Lexicon {
        Lexicon::from_tsv(
            "# test lexicon\n\
             be\t1\tbi\tbi\t是\n\
             go\t40\tɡəʊ\tɡoʊ\t去\n\
             like\t60\tlaɪk\tlaɪk\t喜欢\n\
             stop\t200\tstɒp\tstɑːp\t停\n\
             city\t150\tˈsɪti\tˈsɪti\t城市\n\
             good\t30\tɡʊd\tɡʊd\t好\n\
             child\t180\ttʃaɪld\ttʃaɪld\t孩子\n\
             happy\t250\tˈhæpi\tˈhæpi\t快乐\n\
             do\t20\tduː\tduː\t做\n\
             not\t10\tnɒt\tnɑːt\t不\n\
             i\t2\taɪ\taɪ\t我\n",
        )
        .unwrap()
    }

    #[test]
    fn exact_lookup() {
        assert_eq!(lex().band_of("go"), Some(40));
        assert_eq!(lex().band_of("GO"), Some(40));
    }

    #[test]
    fn irregular_forms_resolve() {
        let l = lex();
        assert_eq!(l.band_of("went"), Some(40));
        assert_eq!(l.band_of("was"), Some(1));
        assert_eq!(l.band_of("children"), Some(180));
        assert_eq!(l.band_of("better"), Some(30));
    }

    #[test]
    fn regular_inflections_resolve() {
        let l = lex();
        assert_eq!(l.band_of("likes"), Some(60));
        assert_eq!(l.band_of("liked"), Some(60));
        assert_eq!(l.band_of("liking"), Some(60));
        assert_eq!(l.band_of("stopped"), Some(200));
        assert_eq!(l.band_of("stopping"), Some(200));
        assert_eq!(l.band_of("cities"), Some(150));
        assert_eq!(l.band_of("happier"), Some(250));
        assert_eq!(l.band_of("happiest"), Some(250));
    }

    #[test]
    fn contractions_resolve_via_head() {
        let l = lex();
        assert_eq!(l.band_of("I'm"), Some(2));
        assert_eq!(l.band_of("don't"), Some(20));
    }

    #[test]
    fn unknown_word_is_none() {
        assert_eq!(lex().band_of("xylophone"), None);
    }

    #[test]
    fn malformed_tsv_rejected() {
        assert!(Lexicon::from_tsv("word_without_band").is_err());
        assert!(Lexicon::from_tsv("w\tnotanumber").is_err());
    }
}
