//! 64-bit simhash over normalized words — near-duplicate detection for
//! generated sentences (spec §7.4 查重) and the "避开指纹" prompt tail (§11.D).
//!
//! Self-contained and deterministic: the same sentence always hashes to the
//! same value on every platform and release, because fingerprints are stored
//! in content.db / progress.db and compared across versions.

/// FNV-1a 64-bit — tiny, stable, good enough as a feature hash.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

fn normalize_words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_ascii_alphabetic() || *c == '\'')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// Simhash over word unigrams + bigrams (bigrams capture word order, so
/// "the cat chased the dog" and "the dog chased the cat" differ).
///
/// Feature weights and the [`NEAR_DUP_MAX_DISTANCE`] threshold were picked
/// empirically on short practice sentences: true near-duplicates (one word
/// dropped/swapped) land at distance 6–16, while distinct sentences bottom
/// out at 17 (short 4–6-word sentences have few features, so even unrelated
/// ones sit closer than long-text simhash intuition suggests). The threshold
/// sits at 16 — the rare near-dup at 17+ slips through on purpose: a false
/// "duplicate" silently discards a good sentence, a false "unique" is caught
/// by 人工抽审 (§8) or bothers no one.
pub fn simhash64(text: &str) -> u64 {
    let words = normalize_words(text);
    if words.is_empty() {
        return 0;
    }
    let mut weights = [0i32; 64];
    let mut add = |feature: &str, weight: i32| {
        let h = fnv1a64(feature.as_bytes());
        for (bit, w) in weights.iter_mut().enumerate() {
            if h >> bit & 1 == 1 {
                *w += weight;
            } else {
                *w -= weight;
            }
        }
    };
    for w in &words {
        add(w, 1);
    }
    for pair in words.windows(2) {
        add(&format!("{} {}", pair[0], pair[1]), 1);
    }
    let mut out = 0u64;
    for (bit, w) in weights.iter().enumerate() {
        if *w > 0 {
            out |= 1 << bit;
        }
    }
    out
}

pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Default near-duplicate threshold used by the validator (see the
/// calibration note on [`simhash64`]).
pub const NEAR_DUP_MAX_DISTANCE: u32 = 16;

/// Short hex fingerprint for the prompt tail (指纹列表,§11.D).
pub fn fingerprint16(hash: u64) -> String {
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_sentences_hash_equal() {
        assert_eq!(simhash64("I am fine."), simhash64("i am FINE"));
    }

    #[test]
    fn near_duplicates_are_close() {
        let pairs = [
            (
                "May I see your passport, please?",
                "May I see your passport?",
            ),
            (
                "May I see your passport, please?",
                "Could I see your passport, please?",
            ),
            (
                "I would like a cup of coffee.",
                "I would like a cup of tea.",
            ),
            ("Can you help me with this?", "Could you help me with that?"),
        ];
        for (a, b) in pairs {
            let d = hamming_distance(simhash64(a), simhash64(b));
            assert!(d <= NEAR_DUP_MAX_DISTANCE, "d = {d} for {a:?} vs {b:?}");
        }
    }

    #[test]
    fn different_sentences_are_far() {
        let pairs = [
            (
                "May I see your passport, please?",
                "The weather is really nice today.",
            ),
            (
                "I went to school yesterday.",
                "She plays tennis every weekend.",
            ),
            ("The meeting starts at nine.", "Our flight leaves at noon."),
        ];
        for (a, b) in pairs {
            let d = hamming_distance(simhash64(a), simhash64(b));
            assert!(d > NEAR_DUP_MAX_DISTANCE, "d = {d} for {a:?} vs {b:?}");
        }
    }

    #[test]
    fn word_order_matters() {
        let a = simhash64("the cat chased the dog");
        let b = simhash64("the dog chased the cat");
        assert_ne!(a, b);
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(simhash64("  ...  "), 0);
    }
}
