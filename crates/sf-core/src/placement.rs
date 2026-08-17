//! 定级测试状态机(《英语水平定级测试-实现方案》,§6.3「自适应定级」的展开)。
//!
//! 三阶段多阶段测试(MST):
//! 1. **词汇快筛(router)**——LexTALE 式「认识/不认识」判断,真词按 NGSL
//!    band 分层抽样 + 伪词做猜测校正,估计词汇量并路由起步等级;
//! 2. **整句实测**——打字版 Elicited Imitation(复用练习组件),阶梯自适应
//!    升降级,收敛出能力值 θ;
//! 3. **语法辨析**——仅当 θ 落在两级边界时,用相邻级语法白名单增量点定向加测。
//!
//! 与 sf-core 其余部分同一契约:纯逻辑、无 IO、显式 `now`/`seed`,
//! 同 seed 全程可复现。题库([`PlacementBank`])由宿主装配后传入。

use crate::rng::SplitMix64;
use crate::sentence::{LevelId, Sentence};
use crate::srs::Mode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ------------------------------------------------------------------ 题库

/// 词汇候选(真词):来自词表,band = NGSL 频次带。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VocabWord {
    pub word: String,
    pub band: u32,
}

/// 边界语法题:考察 `hi` 级相对 `lo` 级的增量语法点。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrammarItem {
    pub lo: LevelId,
    pub hi: LevelId,
    /// 语法点名(结果页解释用,如「一般过去时」)。
    pub topic_zh: String,
    pub prompt_zh: String,
    /// 挖空句,空位为 `___`。
    pub stem: String,
    pub options: Vec<String>,
    /// 正确选项下标。
    pub answer: u8,
}

/// 定级题库。`vocab_pool` 由宿主从词表装配(工厂构建产物中该字段为空);
/// 其余字段来自 content.db `meta["placement"]`(构建期已按级校验)。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlacementBank {
    pub version: u32,
    /// 词汇分层上界(升序),如 [300, 600, 1100, 1600, 2200, 2800]。
    pub strata: Vec<u32>,
    /// 每层抽取的真词数。
    pub per_stratum: u32,
    /// 抽取的伪词数。
    pub pseudo_count: u32,
    pub pseudowords: Vec<String>,
    pub vocab_pool: Vec<VocabWord>,
    /// 各级定级句(已全标注、构建期按级 spec 校验)。
    pub sentences: Vec<Sentence>,
    pub grammar: Vec<GrammarItem>,
}

// ------------------------------------------------------------------ 运行时协议

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlacementConfig {
    /// 宿主 TTS 可用时为 true;否则高段听打题自动退化为普通打字题。
    pub allow_listening: bool,
}

impl Default for PlacementConfig {
    fn default() -> Self {
        Self {
            allow_listening: true,
        }
    }
}

/// 当前应呈现的题目(`next_item` 幂等,提交后才推进)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlacementItem {
    Vocab {
        word: String,
    },
    Sentence {
        sentence: Sentence,
        mode: Mode,
    },
    Grammar {
        topic_zh: String,
        prompt_zh: String,
        stem: String,
        options: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlacementAnswer {
    Vocab {
        known: bool,
    },
    Sentence {
        word_errors: u32,
        seen_answer: bool,
        dur_ms: u32,
        wpm: f32,
    },
    Grammar {
        choice: u8,
    },
}

/// 测试结果(kv 持久化 + 结果页解释素材)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlacementResult {
    /// 推荐等级(已做保守修正)。
    pub level: LevelId,
    /// cfg 校正后的词汇量估计。
    pub vocab_est: u32,
    /// 整句词级正确率(0..1;零基础直通时为 0)。
    pub sentence_accuracy: f32,
    /// 伪词误报率(>0.5 视为乱答,低信度)。
    pub false_alarm_rate: f32,
    pub low_confidence: bool,
    /// 结果页解释,如「一般过去时已掌握」。
    pub grammar_notes: Vec<String>,
    pub taken_at: i64,
}

// ------------------------------------------------------------------ 状态机

/// 整句阶段固定题数(方案 §2.1「6–8 句」取中,时长可预期)。
const SENTENCE_ITEMS: usize = 7;
/// 边界语法加测题数;答对 ≥ [`GRAMMAR_PASS`] 取高级。
const GRAMMAR_ITEMS: usize = 4;
const GRAMMAR_PASS: u32 = 3;
/// 零基础直通阈值:词汇估计低于此值直接推荐入门,不进整句阶段。
const ZERO_BASE_VOCAB: u32 = 150;
/// 低/中/高档路由阈值(θ 起步 1.0 / 2.5 / 4.0)。
const ROUTE_MID_VOCAB: u32 = 400;
const ROUTE_HIGH_VOCAB: u32 = 1400;
/// 乱答闸门:伪词误报率超过此值置低信度。
const FALSE_ALARM_LIMIT: f32 = 0.5;
/// 整句判定:词级错误 ≤1 记通过,≥3 记失败,之间为中性。
const PASS_MAX_ERRORS: u32 = 1;
const FAIL_MIN_ERRORS: u32 = 3;
/// θ 距最近整级超过此值时触发边界语法加测。
const BOUNDARY_GAP: f32 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Vocab,
    Sentence,
    Grammar,
    Done,
}

#[derive(Debug)]
pub struct PlacementTest {
    cfg: PlacementConfig,
    taken_at: i64,
    stage: Stage,

    // ---- 词汇阶段:(词面, band;None = 伪词),已洗牌
    vocab_items: Vec<(String, Option<u32>)>,
    vocab_pos: usize,
    vocab_known: Vec<bool>,
    strata: Vec<u32>,
    vocab_est: u32,
    false_alarm_rate: f32,

    // ---- 整句阶段:各级题目队列(已洗牌,pop 取题)
    by_level: BTreeMap<LevelId, Vec<Sentence>>,
    theta: f32,
    step: f32,
    last_dir: i8,
    /// 方向反转次数(震荡 = 能力落在边界上,触发语法加测)。
    reversals: u32,
    sent_pos: usize,
    sent_word_total: u32,
    sent_word_errors: u32,
    /// 当前呈现中的整句题(提交时结算)。
    current_sentence: Option<(Sentence, Mode)>,

    // ---- 语法阶段
    grammar_pool: Vec<GrammarItem>,
    grammar_items: Vec<GrammarItem>,
    grammar_pos: usize,
    grammar_correct: u32,
    grammar_boundary: Option<(LevelId, LevelId)>,

    result: Option<PlacementResult>,
}

impl PlacementTest {
    /// 从题库构建一次测试。`seed` 决定抽词/抽题/洗牌(同 seed 可复现);
    /// `now` 记入结果的 `taken_at`。
    pub fn new(
        bank: &PlacementBank,
        seed: u64,
        now: i64,
        cfg: PlacementConfig,
    ) -> Result<Self, String> {
        if bank.strata.is_empty() || bank.per_stratum == 0 {
            return Err("题库缺少词汇分层配置".into());
        }
        if bank.pseudowords.len() < bank.pseudo_count as usize {
            return Err("题库伪词数量不足".into());
        }
        if bank.sentences.is_empty() {
            return Err("题库缺少定级句".into());
        }
        let mut rng = SplitMix64::new(seed);

        // 词汇:每层抽 per_stratum 个真词 + pseudo_count 个伪词,整体洗牌。
        // 候选池先排序,消除宿主装配顺序(HashMap 等)带来的不确定性。
        let mut pool: Vec<&VocabWord> = bank.vocab_pool.iter().collect();
        pool.sort_by(|a, b| (a.band, &a.word).cmp(&(b.band, &b.word)));
        let mut vocab_items: Vec<(String, Option<u32>)> = Vec::new();
        let mut lo = 0u32;
        for &hi in &bank.strata {
            let mut stratum: Vec<&&VocabWord> = pool
                .iter()
                .filter(|w| w.band > lo && w.band <= hi)
                .collect();
            rng.shuffle(&mut stratum);
            for w in stratum.into_iter().take(bank.per_stratum as usize) {
                vocab_items.push((w.word.clone(), Some(w.band)));
            }
            lo = hi;
        }
        if vocab_items.is_empty() {
            return Err("词表候选池为空,无法出词汇题".into());
        }
        let mut pseudo: Vec<&String> = bank.pseudowords.iter().collect();
        rng.shuffle(&mut pseudo);
        for p in pseudo.into_iter().take(bank.pseudo_count as usize) {
            vocab_items.push((p.clone(), None));
        }
        rng.shuffle(&mut vocab_items);

        // 整句:按级分组洗牌。
        let mut by_level: BTreeMap<LevelId, Vec<Sentence>> = BTreeMap::new();
        for s in &bank.sentences {
            by_level.entry(s.level).or_default().push(s.clone());
        }
        for queue in by_level.values_mut() {
            rng.shuffle(queue);
        }

        let mut grammar_pool = bank.grammar.clone();
        rng.shuffle(&mut grammar_pool);

        Ok(Self {
            cfg,
            taken_at: now,
            stage: Stage::Vocab,
            vocab_items,
            vocab_pos: 0,
            vocab_known: Vec::new(),
            strata: bank.strata.clone(),
            vocab_est: 0,
            false_alarm_rate: 0.0,
            by_level,
            theta: 1.0,
            step: 0.5,
            last_dir: 0,
            reversals: 0,
            sent_pos: 0,
            sent_word_total: 0,
            sent_word_errors: 0,
            current_sentence: None,
            grammar_pool,
            grammar_items: Vec::new(),
            grammar_pos: 0,
            grammar_correct: 0,
            grammar_boundary: None,
            result: None,
        })
    }

    /// 当前题目;`None` = 测试结束(取 [`result`])。幂等——提交后才推进。
    ///
    /// [`result`]: Self::result
    pub fn next_item(&mut self) -> Option<PlacementItem> {
        match self.stage {
            Stage::Vocab => self
                .vocab_items
                .get(self.vocab_pos)
                .map(|(word, _)| PlacementItem::Vocab { word: word.clone() }),
            Stage::Sentence => {
                if self.current_sentence.is_none() {
                    self.current_sentence = self.draw_sentence();
                    if self.current_sentence.is_none() {
                        // 题库耗尽(异常防御):以现有数据收敛。
                        self.finish_sentence_stage();
                        return self.next_item();
                    }
                }
                self.current_sentence
                    .as_ref()
                    .map(|(sentence, mode)| PlacementItem::Sentence {
                        sentence: sentence.clone(),
                        mode: *mode,
                    })
            }
            Stage::Grammar => {
                self.grammar_items
                    .get(self.grammar_pos)
                    .map(|g| PlacementItem::Grammar {
                        topic_zh: g.topic_zh.clone(),
                        prompt_zh: g.prompt_zh.clone(),
                        stem: g.stem.clone(),
                        options: g.options.clone(),
                    })
            }
            Stage::Done => None,
        }
    }

    /// 提交当前题的作答并推进。答案类型与当前阶段不符时报错(宿主 bug)。
    pub fn submit(&mut self, answer: PlacementAnswer) -> Result<(), String> {
        match (self.stage, answer) {
            (Stage::Vocab, PlacementAnswer::Vocab { known }) => {
                self.vocab_known.push(known);
                self.vocab_pos += 1;
                if self.vocab_pos >= self.vocab_items.len() {
                    self.finish_vocab_stage();
                }
                Ok(())
            }
            (
                Stage::Sentence,
                PlacementAnswer::Sentence {
                    word_errors,
                    seen_answer,
                    ..
                },
            ) => {
                let Some((sentence, _)) = self.current_sentence.take() else {
                    return Err("当前没有待提交的整句题".into());
                };
                let words = sentence.words.len() as u32;
                self.sent_word_total += words.max(1);
                self.sent_word_errors += word_errors.min(words.max(1));

                let dir: i8 = if seen_answer || word_errors >= FAIL_MIN_ERRORS {
                    -1
                } else if word_errors <= PASS_MAX_ERRORS {
                    1
                } else {
                    0
                };
                // 方向反转 → 步长减半(阶梯收敛)。
                if dir != 0 && self.last_dir != 0 && dir != self.last_dir {
                    self.step = (self.step / 2.0).max(0.25);
                    self.reversals += 1;
                }
                if dir != 0 {
                    self.last_dir = dir;
                }
                self.theta = (self.theta + self.step * dir as f32).clamp(1.0, 6.0);
                self.sent_pos += 1;
                if self.sent_pos >= SENTENCE_ITEMS {
                    self.finish_sentence_stage();
                }
                Ok(())
            }
            (Stage::Grammar, PlacementAnswer::Grammar { choice }) => {
                let Some(item) = self.grammar_items.get(self.grammar_pos) else {
                    return Err("当前没有待提交的语法题".into());
                };
                if choice == item.answer {
                    self.grammar_correct += 1;
                }
                self.grammar_pos += 1;
                if self.grammar_pos >= self.grammar_items.len() {
                    self.finish_grammar_stage();
                }
                Ok(())
            }
            (Stage::Done, _) => Err("测试已结束".into()),
            _ => Err("答案类型与当前题目不符".into()),
        }
    }

    /// 全程进度 0..1(词汇 0–0.4,整句 0.4–0.9,语法 0.9–1.0),单调不减。
    pub fn progress(&self) -> f32 {
        match self.stage {
            Stage::Vocab => 0.4 * self.vocab_pos as f32 / self.vocab_items.len().max(1) as f32,
            Stage::Sentence => 0.4 + 0.5 * self.sent_pos as f32 / SENTENCE_ITEMS as f32,
            Stage::Grammar => {
                0.9 + 0.1 * self.grammar_pos as f32 / self.grammar_items.len().max(1) as f32
            }
            Stage::Done => 1.0,
        }
    }

    pub fn result(&self) -> Option<&PlacementResult> {
        self.result.as_ref()
    }

    // -------------------------------------------------------------- 内部

    /// 词汇阶段收官:cfg 校正估词汇量,路由或零基础直通。
    fn finish_vocab_stage(&mut self) {
        let mut pseudo_total = 0u32;
        let mut pseudo_yes = 0u32;
        // 每层命中数
        let mut hits: Vec<u32> = vec![0; self.strata.len()];
        let mut totals: Vec<u32> = vec![0; self.strata.len()];
        for ((_, band), known) in self.vocab_items.iter().zip(&self.vocab_known) {
            match band {
                None => {
                    pseudo_total += 1;
                    if *known {
                        pseudo_yes += 1;
                    }
                }
                Some(b) => {
                    let idx = self.stratum_index(*b);
                    totals[idx] += 1;
                    if *known {
                        hits[idx] += 1;
                    }
                }
            }
        }
        let f = if pseudo_total > 0 {
            pseudo_yes as f32 / pseudo_total as f32
        } else {
            0.0
        };
        self.false_alarm_rate = f;

        let mut est = 0.0f32;
        let mut lo = 0u32;
        for (i, &hi) in self.strata.iter().enumerate() {
            let width = (hi - lo) as f32;
            if totals[i] > 0 && f < 1.0 {
                let h = hits[i] as f32 / totals[i] as f32;
                est += width * ((h - f) / (1.0 - f)).max(0.0);
            }
            lo = hi;
        }
        self.vocab_est = est.round() as u32;

        if self.vocab_est < ZERO_BASE_VOCAB {
            // 零基础保护:直通入门,不让用户挫败地打句子。
            self.finish(LevelId::L1, 0.0, Vec::new());
            return;
        }
        self.theta = if self.vocab_est < ROUTE_MID_VOCAB {
            1.0
        } else if self.vocab_est < ROUTE_HIGH_VOCAB {
            2.5
        } else {
            4.0
        };
        self.stage = Stage::Sentence;
    }

    fn stratum_index(&self, band: u32) -> usize {
        self.strata
            .iter()
            .position(|&hi| band <= hi)
            .unwrap_or(self.strata.len() - 1)
    }

    /// 取当前 θ 对应级的下一题;该级耗尽时就近借题(先低后高)。
    fn draw_sentence(&mut self) -> Option<(Sentence, Mode)> {
        let target = self.theta.round().clamp(1.0, 6.0) as usize;
        let order: Vec<usize> = (1..=10)
            .map(|d| target as i32 + alternate_offset(d))
            .filter(|l| (1..=6).contains(l))
            .map(|l| l as usize)
            .collect();
        let mut chosen: Option<Sentence> = None;
        for lvl_num in std::iter::once(target).chain(order) {
            let lvl = level_from_num(lvl_num);
            if let Some(queue) = self.by_level.get_mut(&lvl)
                && let Some(s) = queue.pop()
            {
                chosen = Some(s);
                break;
            }
        }
        let sentence = chosen?;
        let level_num = sentence.level as usize + 1;
        let mode = if self.sent_pos == 0 && level_num <= 2 {
            // 低档首题用拆句重组,门槛最低
            Mode::Reorder
        } else if level_num >= 5 && self.cfg.allow_listening && self.sent_pos % 2 == 1 {
            // 高段隔题听打,拉开区分度;TTS 不可用时退化为打字
            Mode::Listening
        } else {
            Mode::Typing
        };
        Some((sentence, mode))
    }

    /// 整句阶段收官:决定是否边界加测,否则直接出结果。
    fn finish_sentence_stage(&mut self) {
        let rounded = self.theta.round().clamp(1.0, 6.0);
        let gap = (self.theta - rounded).abs();
        // 不确定的三种迹象:θ 卡在半级上、阶梯未收敛(从未反转)、来回震荡。
        // 已在最低级(rounded=1)时无更低边界可辨,直接收敛。
        let unstable = gap > BOUNDARY_GAP || self.step > 0.25 || self.reversals >= 2;
        let boundary = if unstable && rounded >= 2.0 {
            let hi = rounded as usize;
            Some((level_from_num(hi - 1), level_from_num(hi)))
        } else {
            None
        };

        if let Some((lo, hi)) = boundary {
            let items: Vec<GrammarItem> = self
                .grammar_pool
                .iter()
                .filter(|g| g.lo == lo && g.hi == hi)
                .take(GRAMMAR_ITEMS)
                .cloned()
                .collect();
            if items.len() >= GRAMMAR_ITEMS {
                self.grammar_boundary = Some((lo, hi));
                self.grammar_items = items;
                self.stage = Stage::Grammar;
                return;
            }
        }
        let level = level_from_num(rounded as usize);
        self.conclude(level, Vec::new());
    }

    fn finish_grammar_stage(&mut self) {
        let (lo, hi) = self.grammar_boundary.expect("grammar stage has a boundary");
        let passed = self.grammar_correct >= GRAMMAR_PASS;
        let level = if passed { hi } else { lo };
        let topic = self
            .grammar_items
            .first()
            .map(|g| g.topic_zh.clone())
            .unwrap_or_default();
        let note = if topic.is_empty() {
            Vec::new()
        } else if passed {
            vec![format!("「{topic}」已掌握")]
        } else {
            vec![format!("「{topic}」还需巩固")]
        };
        self.conclude(level, note);
    }

    /// 保守修正后落定结果(宁可偏易勿偏难)。
    fn conclude(&mut self, level: LevelId, grammar_notes: Vec<String>) {
        let accuracy = if self.sent_word_total > 0 {
            1.0 - self.sent_word_errors as f32 / self.sent_word_total as f32
        } else {
            0.0
        };
        let mut final_level = level;
        let vocab_level = vocab_implied_level(self.vocab_est);
        let low_confidence = self.false_alarm_rate > FALSE_ALARM_LIMIT;
        let diff = (level as i32 - vocab_level as i32).abs();
        if low_confidence || diff >= 2 {
            final_level = step_down(final_level);
        }
        self.finish(final_level, accuracy.clamp(0.0, 1.0), grammar_notes);
    }

    fn finish(&mut self, level: LevelId, accuracy: f32, grammar_notes: Vec<String>) {
        self.result = Some(PlacementResult {
            level,
            vocab_est: self.vocab_est,
            sentence_accuracy: accuracy,
            false_alarm_rate: self.false_alarm_rate,
            low_confidence: self.false_alarm_rate > FALSE_ALARM_LIMIT,
            grammar_notes,
            taken_at: self.taken_at,
        });
        self.stage = Stage::Done;
    }
}

/// 1..=6 → LevelId(越界夹取)。
fn level_from_num(n: usize) -> LevelId {
    *LevelId::ALL
        .get(n.saturating_sub(1))
        .unwrap_or(&LevelId::L6)
}

fn step_down(level: LevelId) -> LevelId {
    let idx = level as usize;
    if idx == 0 {
        level
    } else {
        LevelId::ALL[idx - 1]
    }
}

/// 词汇量 → 约当等级(与词表带对齐,用于交叉校验)。
fn vocab_implied_level(est: u32) -> LevelId {
    match est {
        0..400 => LevelId::L1,
        400..900 => LevelId::L2,
        900..1400 => LevelId::L3,
        1400..1900 => LevelId::L4,
        1900..2500 => LevelId::L5,
        _ => LevelId::L6,
    }
}

/// 就近借题的偏移序列:-1, +1, -2, +2, …
fn alternate_offset(d: usize) -> i32 {
    let mag = d.div_ceil(2) as i32;
    if d % 2 == 1 { -mag } else { mag }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentence::{Chunk, PosTag, RoleTag, Word};

    fn word(w: &str) -> Word {
        Word {
            w: w.into(),
            ipa: "x".into(),
            pos: PosTag::Noun,
        }
    }

    fn sentence(id: i64, level: LevelId, n_words: usize) -> Sentence {
        Sentence {
            id,
            level,
            scene: "测试".into(),
            func: String::new(),
            pattern: String::new(),
            zh: "测试句。".into(),
            en: "test sentence".into(),
            punct: ".".into(),
            words: (0..n_words).map(|i| word(&format!("w{i}"))).collect(),
            chunks: vec![Chunk {
                r: RoleTag::Subject,
                i: (0..n_words).collect(),
            }],
            note: String::new(),
            simhash: 0,
        }
    }

    fn bank() -> PlacementBank {
        let mut vocab_pool = Vec::new();
        for band in 1..=2800u32 {
            if band % 40 == 0 {
                vocab_pool.push(VocabWord {
                    word: format!("word{band}"),
                    band,
                });
            }
        }
        let mut sentences = Vec::new();
        let mut id = 1;
        for level in LevelId::ALL {
            for _ in 0..10 {
                sentences.push(sentence(id, level, 5));
                id += 1;
            }
        }
        let mut grammar = Vec::new();
        for w in LevelId::ALL.windows(2) {
            for k in 0..4 {
                grammar.push(GrammarItem {
                    lo: w[0],
                    hi: w[1],
                    topic_zh: format!("{}级语法", w[1].as_str()),
                    prompt_zh: "题干".into(),
                    stem: "I ___ it.".into(),
                    options: vec!["a".into(), "b".into()],
                    answer: (k % 2) as u8,
                });
            }
        }
        PlacementBank {
            version: 1,
            strata: vec![300, 600, 1100, 1600, 2200, 2800],
            per_stratum: 3,
            pseudo_count: 6,
            pseudowords: (0..12).map(|i| format!("pseudo{i}")).collect(),
            vocab_pool,
            sentences,
            grammar,
        }
    }

    fn new_test(seed: u64) -> PlacementTest {
        PlacementTest::new(&bank(), seed, 1000, PlacementConfig::default()).unwrap()
    }

    /// 词汇阶段逐题作答;strategy 依 (词面, 是否伪词) 决定「认识」。
    fn run_vocab(t: &mut PlacementTest, know: impl Fn(&str) -> bool) {
        while let Some(PlacementItem::Vocab { word }) = t.next_item() {
            t.submit(PlacementAnswer::Vocab { known: know(&word) })
                .unwrap();
        }
    }

    fn answer_sentence(t: &mut PlacementTest, word_errors: u32) {
        t.submit(PlacementAnswer::Sentence {
            word_errors,
            seen_answer: false,
            dur_ms: 5000,
            wpm: 30.0,
        })
        .unwrap();
    }

    #[test]
    fn vocab_stage_has_expected_item_count() {
        let mut t = new_test(7);
        let mut n = 0;
        while let Some(PlacementItem::Vocab { .. }) = t.next_item() {
            t.submit(PlacementAnswer::Vocab { known: false }).unwrap();
            n += 1;
        }
        assert_eq!(n, 6 * 3 + 6, "6 层×3 真词 + 6 伪词");
    }

    #[test]
    fn zero_base_goes_straight_to_l1() {
        let mut t = new_test(7);
        run_vocab(&mut t, |_| false); // 全部不认识
        let r = t.result().expect("零基础直通应立即出结果");
        assert_eq!(r.level, LevelId::L1);
        assert_eq!(r.vocab_est, 0);
        assert!(!r.low_confidence);
        assert!(t.next_item().is_none());
    }

    #[test]
    fn all_yes_on_pseudowords_is_low_confidence_and_conservative() {
        let mut t = new_test(7);
        run_vocab(&mut t, |_| true); // 乱按:全部「认识」(伪词误报率 1.0)
        // f = 1.0 → est 0 → 零基础直通,但低信度已在误报率上体现:
        // est 为 0 时走直通,不进入低信度修正——直通本身已是最保守结果。
        let r = t.result().expect("误报率 1.0 时校正后 est=0,直通");
        assert_eq!(r.level, LevelId::L1);
        assert!((r.false_alarm_rate - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn strong_vocab_routes_high_and_all_pass_reaches_top() {
        let mut t = new_test(7);
        run_vocab(&mut t, |w| !w.starts_with("pseudo")); // 真词全认识,伪词全拒
        assert!(t.result().is_none(), "高档路由应进入整句阶段");
        let mut sentence_levels = Vec::new();
        while let Some(item) = t.next_item() {
            match item {
                PlacementItem::Sentence { sentence, .. } => {
                    sentence_levels.push(sentence.level);
                    answer_sentence(&mut t, 0);
                }
                PlacementItem::Grammar { .. } => {
                    t.submit(PlacementAnswer::Grammar { choice: 0 }).unwrap();
                }
                PlacementItem::Vocab { .. } => unreachable!(),
            }
        }
        let r = t.result().unwrap();
        assert_eq!(sentence_levels[0], LevelId::L4, "高档从 L4 起步");
        assert!(r.level >= LevelId::L5, "全对应到达高段,got {:?}", r.level);
        assert!(r.sentence_accuracy > 0.99);
    }

    #[test]
    fn mid_route_all_fail_sinks_to_bottom() {
        let mut t = new_test(7);
        // 认识 ≤1100 band 的词 → est 落在中档
        run_vocab(&mut t, |w| {
            w.strip_prefix("word")
                .and_then(|b| b.parse::<u32>().ok())
                .is_some_and(|b| b <= 1100)
        });
        assert!(t.result().is_none());
        while let Some(item) = t.next_item() {
            match item {
                PlacementItem::Sentence { .. } => {
                    t.submit(PlacementAnswer::Sentence {
                        word_errors: 5,
                        seen_answer: true,
                        dur_ms: 9000,
                        wpm: 5.0,
                    })
                    .unwrap();
                }
                PlacementItem::Grammar { .. } => {
                    t.submit(PlacementAnswer::Grammar { choice: 1 }).unwrap();
                }
                PlacementItem::Vocab { .. } => unreachable!(),
            }
        }
        assert_eq!(t.result().unwrap().level, LevelId::L1);
    }

    #[test]
    fn same_seed_reproduces_identical_run() {
        let script = |mut t: PlacementTest| {
            let mut trace = Vec::new();
            while let Some(item) = t.next_item() {
                match item {
                    PlacementItem::Vocab { word } => {
                        trace.push(format!("v:{word}"));
                        t.submit(PlacementAnswer::Vocab {
                            known: !word.starts_with("pseudo"),
                        })
                        .unwrap();
                    }
                    PlacementItem::Sentence { sentence, mode } => {
                        trace.push(format!("s:{}:{:?}", sentence.id, mode));
                        answer_sentence(&mut t, 2); // 全程中性
                    }
                    PlacementItem::Grammar { stem, .. } => {
                        trace.push(format!("g:{stem}"));
                        t.submit(PlacementAnswer::Grammar { choice: 0 }).unwrap();
                    }
                }
            }
            (trace, t.result().unwrap().clone())
        };
        let (trace_a, ra) = script(new_test(42));
        let (trace_b, rb) = script(new_test(42));
        assert_eq!(trace_a, trace_b, "同 seed 出题序列一致");
        assert_eq!(ra, rb);
        let (trace_c, _) = script(new_test(43));
        assert_ne!(trace_a, trace_c, "换 seed 抽题应不同");
    }

    #[test]
    fn progress_is_monotonic_and_reaches_one() {
        let mut t = new_test(9);
        let mut last = -1.0f32;
        loop {
            let p = t.progress();
            assert!(p >= last, "进度不可回退: {p} < {last}");
            last = p;
            match t.next_item() {
                None => break,
                Some(PlacementItem::Vocab { word }) => t
                    .submit(PlacementAnswer::Vocab {
                        known: !word.starts_with("pseudo"),
                    })
                    .unwrap(),
                Some(PlacementItem::Sentence { .. }) => answer_sentence(&mut t, 0),
                Some(PlacementItem::Grammar { .. }) => {
                    t.submit(PlacementAnswer::Grammar { choice: 0 }).unwrap()
                }
            }
        }
        assert!((t.progress() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn boundary_oscillation_triggers_grammar_stage() {
        let mut t = new_test(11);
        run_vocab(&mut t, |w| {
            w.strip_prefix("word")
                .and_then(|b| b.parse::<u32>().ok())
                .is_some_and(|b| b <= 1100)
        });
        // 交替通过/失败 → θ 在边界震荡,步长收敛后仍夹在两级之间
        let mut saw_grammar = false;
        let mut flip = true;
        while let Some(item) = t.next_item() {
            match item {
                PlacementItem::Sentence { .. } => {
                    answer_sentence(&mut t, if flip { 0 } else { 5 });
                    flip = !flip;
                }
                PlacementItem::Grammar { .. } => {
                    saw_grammar = true;
                    t.submit(PlacementAnswer::Grammar { choice: 0 }).unwrap();
                }
                PlacementItem::Vocab { .. } => unreachable!(),
            }
        }
        assert!(saw_grammar, "边界震荡应触发语法加测");
        assert!(t.result().is_some());
    }

    #[test]
    fn grammar_pass_takes_higher_level() {
        // 直接构造边界态:用震荡进入语法阶段,再全对
        let mut t = new_test(11);
        run_vocab(&mut t, |w| {
            w.strip_prefix("word")
                .and_then(|b| b.parse::<u32>().ok())
                .is_some_and(|b| b <= 1100)
        });
        let mut flip = true;
        let mut boundary_hi = None;
        while let Some(item) = t.next_item() {
            match item {
                PlacementItem::Sentence { .. } => {
                    answer_sentence(&mut t, if flip { 0 } else { 5 });
                    flip = !flip;
                }
                PlacementItem::Grammar { .. } => {
                    if boundary_hi.is_none() {
                        boundary_hi = t.grammar_boundary.map(|(_, hi)| hi);
                    }
                    // 全对(答案在题库里交替 0/1,按当前题作答)
                    let ans = t.grammar_items[t.grammar_pos].answer;
                    t.submit(PlacementAnswer::Grammar { choice: ans }).unwrap();
                }
                PlacementItem::Vocab { .. } => unreachable!(),
            }
        }
        let r = t.result().unwrap();
        assert_eq!(Some(r.level), boundary_hi, "语法全对取边界高级");
        assert!(r.grammar_notes.iter().any(|n| n.contains("已掌握")));
    }

    #[test]
    fn seen_answer_counts_as_failure() {
        let mut t = new_test(7);
        run_vocab(&mut t, |w| !w.starts_with("pseudo"));
        // 每题 0 错但都看了答案 → 全程降级
        while let Some(item) = t.next_item() {
            match item {
                PlacementItem::Sentence { .. } => t
                    .submit(PlacementAnswer::Sentence {
                        word_errors: 0,
                        seen_answer: true,
                        dur_ms: 3000,
                        wpm: 20.0,
                    })
                    .unwrap(),
                PlacementItem::Grammar { .. } => {
                    t.submit(PlacementAnswer::Grammar { choice: 1 }).unwrap()
                }
                PlacementItem::Vocab { .. } => unreachable!(),
            }
        }
        assert!(
            t.result().unwrap().level <= LevelId::L2,
            "看答案视为未通过,应显著下沉"
        );
    }

    #[test]
    fn listening_mode_respects_allow_flag() {
        let run = |allow: bool| {
            let mut t = PlacementTest::new(
                &bank(),
                7,
                0,
                PlacementConfig {
                    allow_listening: allow,
                },
            )
            .unwrap();
            run_vocab(&mut t, |w| !w.starts_with("pseudo"));
            let mut modes = Vec::new();
            while let Some(item) = t.next_item() {
                match item {
                    PlacementItem::Sentence { mode, .. } => {
                        modes.push(mode);
                        answer_sentence(&mut t, 0);
                    }
                    PlacementItem::Grammar { .. } => {
                        t.submit(PlacementAnswer::Grammar { choice: 0 }).unwrap()
                    }
                    PlacementItem::Vocab { .. } => unreachable!(),
                }
            }
            modes
        };
        assert!(run(true).contains(&Mode::Listening), "高段应有听打题");
        assert!(!run(false).contains(&Mode::Listening), "TTS 不可用时无听打");
    }

    #[test]
    fn mismatched_answer_kind_is_an_error() {
        let mut t = new_test(7);
        assert!(
            t.submit(PlacementAnswer::Grammar { choice: 0 }).is_err(),
            "词汇阶段提交语法答案应报错"
        );
    }

    #[test]
    fn vocab_and_theta_disagreement_steps_down() {
        // 词汇拉满(est 高)但整句全程失败 → θ 沉底;交叉校验差 ≥2 级再降一级
        // (θ=1 已是底,保守修正不越界)
        let mut t = new_test(7);
        run_vocab(&mut t, |w| !w.starts_with("pseudo"));
        while let Some(item) = t.next_item() {
            match item {
                PlacementItem::Sentence { .. } => {
                    t.submit(PlacementAnswer::Sentence {
                        word_errors: 5,
                        seen_answer: false,
                        dur_ms: 9000,
                        wpm: 5.0,
                    })
                    .unwrap();
                }
                PlacementItem::Grammar { .. } => {
                    t.submit(PlacementAnswer::Grammar { choice: 1 }).unwrap()
                }
                PlacementItem::Vocab { .. } => unreachable!(),
            }
        }
        let r = t.result().unwrap();
        assert_eq!(r.level, LevelId::L1, "保守修正不越过下界");
    }
}
