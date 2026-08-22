//! 近重判定(spec §7.4 查重)。
//!
//! # 为什么不再让 simhash 定生死
//!
//! 原实现用 64 位 simhash + 汉明距离 ≤16 判近重,并且拿**整个句库**当比较
//! 范围。在真实语料上实测(出厂库 132 句 + 用户库 69 句):
//!
//! | 分布 | simhash 汉明距离 |
//! |------|------------------|
//! | 不同句(应放行)| 最小 15,主体 17–22 |
//! | 真近重(应拦下)| 中位 11–13,95 分位 17–19 |
//!
//! 两个分布**首尾相叠**,阈值 16 正落在"不同句"分布内部。后果是随库增长
//! 误杀急剧上升 —— 随机两句距离 ≤16 的概率约 1.0e-4,那么一句全新好句被
//! 误判重复的概率是 `1-(1-1e-4)^N`:N=1000 时 9.5%,**N=5000 时 39%**。
//! 也就是说句库越大,生成入库率越低,而且看不出原因(卡片只说"与已有句重复")。
//!
//! 短句本来就没几个特征位,把它们压进 64 bit 必然丢分辨率。这里改成**精确**
//! 计算词形特征的 Jaccard 相似度 —— 句子最长 20 词,直接算比哈希还快,且不丢信息:
//!
//! ```text
//! similarity = 0.5 × Jaccard(unigram 集合) + 0.5 × Jaccard(bigram 集合)
//! ```
//!
//! 同一批语料上的实测(权重 0.5/0.5 是在 0.0–0.5 里分离度最好的一档):
//!
//! | 分布 | 相似度 |
//! |------|--------|
//! | 不同句(20100 对)| 中位 0.000,99.9 分位 0.330,**最大 0.600** |
//! | 真近重(删词/换序/加词)| 5 分位 0.571,中位 0.67–0.85 |
//!
//! 阈值 [`NEAR_DUP_MIN_SIMILARITY`] 取 0.65,**高于**实测"不同句"的最大值,
//! 与其 99.9 分位(0.330)之间留着极宽的余量 —— 这正是 simhash 方案缺的东西。
//!
//! # 比较范围
//!
//! 近重只在**同一场景内**比较。工坊的硬承诺是"每个场景中的句子不重复"
//! (见 `workshop::exists_by_en` 的注释),跨场景撞车既无害也不该拦:
//! 「多少钱?」在购物和打车两个场景里各有一句是正常的。缩到场景内之后,
//! N 从整库(数千)降到几十,误杀概率再降两个数量级。完全同文另有全局精确
//! 检查兜底,不受此影响。

/// 判为近重的最低相似度。见模块文档的标定表。
pub const NEAR_DUP_MIN_SIMILARITY: f64 = 0.65;

/// 一条句子的查重特征:词形归一后的 unigram / bigram 哈希集合(升序去重)。
///
/// 存哈希而非原词是为了省内存与加速交集:一句约 20 个 u64,5000 句不到 1 MB。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DedupeKey {
    unigrams: Vec<u64>,
    bigrams: Vec<u64>,
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}

/// 词形归一:只留字母与撇号,转小写。与 simhash 的归一保持一致,
/// 这样"改动大小写/标点"不会被当成新句子。
fn normalized_words(text: &str) -> Vec<String> {
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

fn sorted_unique(mut v: Vec<u64>) -> Vec<u64> {
    v.sort_unstable();
    v.dedup();
    v
}

/// 两个升序去重序列的交集大小(线性归并,不分配)。
fn intersection_len(a: &[u64], b: &[u64]) -> usize {
    let (mut i, mut j, mut n) = (0usize, 0usize, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                n += 1;
                i += 1;
                j += 1;
            }
        }
    }
    n
}

fn jaccard(a: &[u64], b: &[u64]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let inter = intersection_len(a, b);
    let union = a.len() + b.len() - inter;
    if union == 0 {
        0.0
    } else {
        inter as f64 / union as f64
    }
}

impl DedupeKey {
    /// 由英文句构造查重特征。
    pub fn of(en: &str) -> Self {
        let words = normalized_words(en);
        let unigrams = sorted_unique(words.iter().map(|w| fnv1a64(w.as_bytes())).collect());
        let bigrams = sorted_unique(
            words
                .windows(2)
                .map(|p| fnv1a64(format!("{} {}", p[0], p[1]).as_bytes()))
                .collect(),
        );
        Self { unigrams, bigrams }
    }

    /// 0.0(毫无关系)– 1.0(词与词序完全一致)。见模块文档的权重标定。
    pub fn similarity(&self, other: &Self) -> f64 {
        0.5 * jaccard(&self.unigrams, &other.unigrams)
            + 0.5 * jaccard(&self.bigrams, &other.bigrams)
    }

    pub fn is_empty(&self) -> bool {
        self.unigrams.is_empty()
    }
}

/// 已接受句子的查重索引(按场景装载,见模块文档"比较范围")。
///
/// 同时留着原句文本:补足批的 prompt 要拿**真句子**告诉模型"别再写这些"
/// (此前传的是 simhash 十六进制指纹,模型无从理解,纯属白烧 token)。
#[derive(Debug, Default, Clone)]
pub struct DedupeIndex {
    entries: Vec<(String, DedupeKey)>,
}

impl DedupeIndex {
    /// 从已有句子的英文原文建索引。
    pub fn new<I, S>(existing: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut idx = Self::default();
        for en in existing {
            idx.add(en);
        }
        idx
    }

    pub fn add(&mut self, en: impl Into<String>) {
        let en = en.into();
        let key = DedupeKey::of(&en);
        if !key.is_empty() {
            self.entries.push((en, key));
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 与索引中最像的一条的相似度;索引为空时 None。
    pub fn nearest_similarity(&self, key: &DedupeKey) -> Option<f64> {
        self.entries
            .iter()
            .map(|(_, k)| k.similarity(key))
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// 最近接受的 n 条原句(新→旧),给 prompt 当"已经写过、别重复"的样例。
    pub fn recent(&self, n: usize) -> Vec<&str> {
        self.entries
            .iter()
            .rev()
            .take(n)
            .map(|(en, _)| en.as_str())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sim(a: &str, b: &str) -> f64 {
        DedupeKey::of(a).similarity(&DedupeKey::of(b))
    }

    #[test]
    fn identical_sentences_score_one() {
        assert!((sim("How much is it?", "How much is it?") - 1.0).abs() < 1e-9);
        // 大小写与标点不构成新句子
        assert!((sim("How much is it?", "how much IS it") - 1.0).abs() < 1e-9);
    }

    #[test]
    fn near_duplicates_clear_the_threshold() {
        // 删一个词 / 加一个词 —— 典型的"换个说法其实是同一句"
        assert!(
            sim(
                "Could I get a medium latte, please?",
                "Could I get a latte, please?"
            ) >= NEAR_DUP_MIN_SIMILARITY
        );
        assert!(
            sim("I would like some water.", "I would like some water now.")
                >= NEAR_DUP_MIN_SIMILARITY
        );
    }

    /// 回归:这两句是实测里 simhash 距离 15(判为重复)的**误杀**样本,
    /// 精确相似度必须把它们判成不同句。
    #[test]
    fn distinct_sentences_stay_below_threshold() {
        let s = sim("How much is this red shirt?", "It's Lin. How much is it?");
        assert!(
            s < NEAR_DUP_MIN_SIMILARITY,
            "误杀样本相似度 {s} 不应达到阈值"
        );
    }

    #[test]
    fn word_order_matters() {
        // 词集合相同、语序不同:unigram 满分,bigram 拉开差距
        let s = sim("The cat chased the dog.", "The dog chased the cat.");
        assert!(s < 1.0, "语序不同不该判成同一句(similarity={s})");
        assert!(s > 0.5, "但仍应足够像(similarity={s})");
    }

    #[test]
    fn index_reports_nearest_and_recent() {
        let mut idx = DedupeIndex::new(["How much is it?", "I would like some water."]);
        idx.add("Where is the gate?");
        assert_eq!(idx.len(), 3);
        assert_eq!(
            idx.recent(2),
            vec!["Where is the gate?", "I would like some water."]
        );
        let near = idx
            .nearest_similarity(&DedupeKey::of("I would like some water now."))
            .unwrap();
        assert!(near >= NEAR_DUP_MIN_SIMILARITY, "near={near}");
        let far = idx
            .nearest_similarity(&DedupeKey::of("My flight leaves tomorrow."))
            .unwrap();
        assert!(far < NEAR_DUP_MIN_SIMILARITY, "far={far}");
    }

    #[test]
    fn empty_index_has_no_nearest() {
        assert_eq!(
            DedupeIndex::default().nearest_similarity(&DedupeKey::of("hi there")),
            None
        );
    }
}
