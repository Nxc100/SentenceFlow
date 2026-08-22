//! 64-bit simhash over normalized words.
//!
//! **近重判定已不在这里** —— 短句压进 64 bit 分辨率不够,判定搬到了
//! [`crate::dedupe`](crate::dedupe)(精确 Jaccard,附标定数据)。
//! 这里留下的用途只有两个,都只需要"稳定的句子指纹":
//! * `sentence.simhash` 列(已落库多版,不改格式);
//! * 「帮我选模型」微基准的名单指纹。
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

/// Simhash over word unigrams + bigrams(bigram 承载词序,所以
/// "the cat chased the dog" 与 "the dog chased the cat" 不同)。
///
/// 输出会落库(`sentence.simhash`),因此**算法不可变** —— 变了旧库里的
/// 指纹就对不上了。
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

/// 短十六进制指纹 —— 用于「帮我选模型」微基准的名单指纹。
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

    /// simhash 已落库(`sentence.simhash`)且跨版本比对,算法**不可改**。
    /// 这里把几个固定输入的哈希值钉死 —— 一旦有人动了归一化或特征权重,
    /// 这个测试立刻炸,而不是等到旧库指纹对不上才发现。
    ///
    /// (近重判定不再看这个值,搬去了 crate::dedupe —— 短句压进 64 bit
    ///  分辨率不够,实测"不同句"与"真近重"的距离分布首尾相叠。)
    #[test]
    fn hash_values_are_frozen() {
        for (text, expected) in [
            ("May I see your passport, please?", 0xc07f_4e4d_86a7_3003u64),
            ("I am fine.", 0xaad7_a54c_b621_ea6du64),
        ] {
            let got = simhash64(text);
            assert_eq!(
                got, expected,
                "simhash({text:?}) = {got:#018x};算法变了会让已落库的指纹全部失效"
            );
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
