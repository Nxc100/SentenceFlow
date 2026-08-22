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

    /// 遍历全部词条(顺序不保证——HashMap 底层;需要确定性的消费方
    /// 自行排序,如定级测试的分层抽样)。
    pub fn entries(&self) -> impl Iterator<Item = &LexEntry> {
        self.entries.values()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 归一化查表键:保留字母、撇号与连字符。
    ///
    /// 连字符必须留着 —— 滤掉的话 `hard-working` 变成 `hardworking`,
    /// 既查不到整词也没机会按成分拆(见 [`Self::hyphenated`]);
    /// 表里唯一的连字符词条 `e-mail` 也会因此永远命中不了。
    fn normalize(word: &str) -> String {
        word.chars()
            .filter(|c| c.is_ascii_alphabetic() || *c == '\'' || *c == '-')
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
        //
        // 两条都**递归走完整 lookup**,不能只查 entries:`doesn't` 剥出来是
        // `does`,而 does 只在不规则表里(词元是 do)—— 直查 entries 会漏,
        // 于是 doesn't/isn't/wasn't 这类最常见的口语缩写全判成生词。
        // 递归安全:head 与 stem 都已不含撇号,不会再走进这个分支。
        if let Some((head, _)) = w.split_once('\'') {
            if !head.is_empty()
                && let Some(e) = self.lookup(head)
            {
                return Some(e);
            }
            if let Some(stem) = w.strip_suffix("n't")
                && let Some(e) = self.lookup(stem)
            {
                return Some(e);
            }
        }
        // 不规则表命中就用它的词根;但**词根不在词表里时不能就此收手** ——
        // 例如 leaves→leaf,而 leaf 不在 NGSL 里,直接返回 None 就把
        // leaves(动词 leave 的三单,词表里有 leave)也一起判成生词了。
        if let Some(&lemma) = self.irregular.get(w.as_str())
            && let Some(e) = self.entries.get(lemma)
        {
            return Some(e);
        }
        for candidate in strip_suffix_candidates(&w) {
            if let Some(e) = self.entries.get(candidate.as_str()) {
                return Some(e);
            }
        }
        self.hyphenated(&w)
    }

    /// 连字符复合词:每一段都认识就算认识,难度取**最难的那一段**。
    ///
    /// `hard-working` / `twenty-five` / `check-in` 的组成部分全在词表里,
    /// 整体却查不到 —— 实测里这是"词表外"误判的一大来源。取最大 band 而非
    /// 最小,是因为复合词至少和它最难的成分一样难,宁严勿松。
    ///
    /// 只在前面所有还原都失败后才走这条路,所以不会影响已能直接命中的词。
    fn hyphenated(&self, w: &str) -> Option<&LexEntry> {
        if !w.contains('-') {
            return None;
        }
        let mut hardest: Option<&LexEntry> = None;
        for part in w.split('-').filter(|p| !p.is_empty()) {
            // 递归:每段自己也可能需要还原(check-ins → check + in)
            let e = self.lookup(part)?;
            if hardest.is_none_or(|h| e.band > h.band) {
                hardest = Some(e);
            }
        }
        hardest
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
    // -ves → f / fe(leaves→leaf、knives→knife、lives→life)
    if let Some(stem) = w.strip_suffix("ves") {
        push(format!("{stem}f"));
        push(format!("{stem}fe"));
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
        if ends_with_double_consonant(stem) {
            push(stem[..stem.len() - 1].to_string()); // biggest → big
        }
    }
    if let Some(stem) = w.strip_suffix("er") {
        push(stem.to_string());
        push(format!("{stem}e"));
        if ends_with_double_consonant(stem) {
            push(stem[..stem.len() - 1].to_string()); // bigger → big
        }
    }
    // adverb: -ly / -ily→y (happily → happy)
    if let Some(stem) = w.strip_suffix("ily") {
        push(format!("{stem}y"));
    }
    if let Some(stem) = w.strip_suffix("ly") {
        push(stem.to_string());
        push(format!("{stem}e"));
        // -le 结尾的形容词换 e 为 y:simple → simply、possible → possibly
        push(format!("{stem}le"));
    }
    // 派生词缀:refundable → refund、careless → care、payment → pay。
    // 这些只在直接命中与屈折还原都失败后才试,所以不会改变已认识的词;
    // 难度取词根的 band(与复数/过去式的既有处理一致)。
    for suffix in [
        "able", "ible", "ful", "less", "ness", "ment", "ship", "hood",
    ] {
        if let Some(stem) = w.strip_suffix(suffix) {
            push(stem.to_string());
            push(format!("{stem}e")); // usable → use
            if let Some(rest) = stem.strip_suffix('i') {
                push(format!("{rest}y")); // happiness → happy
            }
        }
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
    // ↓ 连字符复合词回归(见 Lexicon::hyphenated)
    #[test]
    fn hyphenated_compound_takes_hardest_part() {
        let lex = super::Lexicon::from_tsv(
            "hard\t323\t\t\t\nwork\t75\t\t\t\ntwenty\t543\t\t\t\nfive\t286\t\t\t\ncheck\t400\t\t\t\nin\t6\t\t\t\n",
        )
        .unwrap();
        // 整体查不到,但每段都在 —— 取最难的一段的 band
        assert_eq!(lex.band_of("hard-working"), Some(323));
        assert_eq!(lex.band_of("twenty-five"), Some(543));
        assert_eq!(lex.band_of("check-in"), Some(400));
        // 有一段不认识就整体不认识,不能放水
        assert_eq!(lex.band_of("hard-zzzz"), None);
    }

    /// 回归:不规则表命中、但它的词根不在词表里时,必须继续走后缀还原。
    /// `leaves` 被映射到 `leaf`(NGSL 没有),此前直接返回 None,
    /// 连"leave 的三单"这条明路都不走了。
    #[test]
    fn irregular_miss_falls_through_to_suffix_rules() {
        let lex = super::Lexicon::from_tsv("leave\t250\t\t\t\n").unwrap();
        assert_eq!(lex.band_of("leaves"), Some(250));
    }

    /// 比较级的双写辅音还原:bigger → big(此前只有 -ed/-ing 做了去重复)。
    #[test]
    fn comparative_undoubles_consonant() {
        let lex = super::Lexicon::from_tsv("big\t180\t\t\t\nhot\t400\t\t\t\n").unwrap();
        assert_eq!(lex.band_of("bigger"), Some(180));
        assert_eq!(lex.band_of("biggest"), Some(180));
        assert_eq!(lex.band_of("hotter"), Some(400));
    }

    /// 回归:缩写要走完整还原链。`doesn't` 剥掉 n't 是 `does`,而 does 只在
    /// 不规则表里(词元 do)—— 只查 entries 的话这类最常见的口语缩写全成生词。
    #[test]
    fn contractions_resolve_through_irregular_forms() {
        let lex =
            super::Lexicon::from_tsv("do\t20\t\t\t\nbe\t2\t\t\t\nhave\t9\t\t\t\nlike\t65\t\t\t\n")
                .unwrap();
        assert_eq!(lex.band_of("doesn't"), Some(20));
        assert_eq!(lex.band_of("don't"), Some(20));
        assert_eq!(lex.band_of("isn't"), Some(2));
        assert_eq!(lex.band_of("wasn't"), Some(2));
        assert_eq!(lex.band_of("haven't"), Some(9));
        assert_eq!(lex.band_of("I'd"), None, "词表里没有 I,不该硬凑");
        // 撇号后的部分不参与:we'll 只看 we
        assert_eq!(lex.band_of("like's"), Some(65));
    }

    /// 派生词缀还原:词根认识就算认识(实测 refundable / careless
    /// 这类词此前一律判"词表外",整句被丢)。
    #[test]
    fn derivational_suffixes_resolve_to_stem() {
        let lex = super::Lexicon::from_tsv(
            "refund\t1900\t\t\t\ncare\t120\t\t\t\npay\t200\t\t\t\nuse\t60\t\t\t\nhappy\t300\t\t\t\nsimple\t400\t\t\t\n",
        )
        .unwrap();
        assert_eq!(lex.band_of("refundable"), Some(1900));
        assert_eq!(lex.band_of("careless"), Some(120));
        assert_eq!(lex.band_of("careful"), Some(120));
        assert_eq!(lex.band_of("payment"), Some(200));
        assert_eq!(lex.band_of("usable"), Some(60));
        assert_eq!(lex.band_of("happiness"), Some(300));
        assert_eq!(lex.band_of("happily"), Some(300));
        assert_eq!(lex.band_of("simply"), Some(400));
        // 词根不认识就不认识
        assert_eq!(lex.band_of("zzzzable"), None);
    }

    /// 表里带连字符的词条(base.tsv 的 `e-mail`)必须查得到 ——
    /// normalize 早先滤掉连字符,把它变成了永远命中不了的死条目。
    #[test]
    fn hyphenated_entry_is_reachable() {
        let lex = super::Lexicon::from_tsv("e-mail\t900\t\t\t\n").unwrap();
        assert_eq!(lex.band_of("e-mail"), Some(900));
        assert_eq!(lex.band_of("E-Mail"), Some(900));
    }

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
