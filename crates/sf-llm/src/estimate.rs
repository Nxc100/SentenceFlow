//! Token estimation for the CostBar's pre-flight number (§5.5 预估).
//!
//! Deliberate deviation from the doc's `tiktoken-rs` suggestion: none of the
//! four channels actually bills by tiktoken vocabularies (DeepSeek, Zen and
//! local models each have their own tokenizers), so an exact tiktoken count
//! would still be wrong — while costing a heavyweight dependency. We use a
//! calibrated character-class heuristic instead; the UI already pins every
//! estimate with "以官方账单为准" (§4.7), and real usage is backfilled from the
//! API response (§7.5).

/// Estimate the token count of `text` for budgeting purposes.
///
/// Heuristic: ASCII words ≈ 1.3 tokens each, CJK chars ≈ 1 token each,
/// other symbols ≈ 1 token per 4 chars. Errs slightly high on purpose —
/// overestimating keeps the hard budget stop conservative.
pub fn estimate_tokens(text: &str) -> u64 {
    let mut ascii_words = 0u64;
    let mut cjk_chars = 0u64;
    let mut other_chars = 0u64;
    let mut in_word = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if !in_word {
                ascii_words += 1;
                in_word = true;
            }
        } else {
            in_word = false;
            let cp = c as u32;
            // CJK Unified Ideographs + punctuation ranges commonly used in zh.
            if (0x4E00..=0x9FFF).contains(&cp)
                || (0x3000..=0x303F).contains(&cp)
                || (0xFF00..=0xFFEF).contains(&cp)
            {
                cjk_chars += 1;
            } else if !c.is_whitespace() {
                other_chars += 1;
            }
        }
    }
    (ascii_words * 13).div_ceil(10) + cjk_chars + other_chars.div_ceil(4)
}

/// Cost in CNY for a token count at a per-million price.
pub fn cost_cny(tokens: u64, price_per_million: f64) -> f64 {
    tokens as f64 * price_per_million / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_scales_with_words() {
        let short = estimate_tokens("May I see your passport");
        let long = estimate_tokens("May I see your passport please sir it is required");
        assert!(short >= 5);
        assert!(long > short);
    }

    #[test]
    fn chinese_counts_chars() {
        let t = estimate_tokens("请出示您的护照");
        assert!(t >= 7, "t = {t}");
    }

    #[test]
    fn empty_is_zero() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("   "), 0);
    }

    #[test]
    fn cost_math() {
        // 45K tokens at ¥2 / 1M ≈ ¥0.09
        let c = cost_cny(45_000, 2.0);
        assert!((c - 0.09).abs() < 1e-9);
    }
}
