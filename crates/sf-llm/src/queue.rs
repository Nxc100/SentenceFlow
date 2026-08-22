//! Generation task queue (spec §7.5 `gen_queue`): micro-batches, resumable
//! jobs, half-finished output banked early (半成品先入库).
//!
//! This is the pure state machine; persistence (progress.db) and execution
//! (adapters) live in the app layer. Every transition returns a new state so
//! the app can persist after each step — a crash or 断网 loses at most the
//! in-flight batch, and [续跑] resumes from the next pending one (§6.3).

use serde::{Deserialize, Serialize};

/// 拿满机制:补足批的最大追加次数。规划批全部跑完仍未达到用户指定句数时,
/// 逐批追加补足批直到拿满或触顶;上限防止在词表覆盖不了的场景
/// (如 L1 写快餐食物词)无限烧额度。
pub const MAX_TOPUP_BATCHES: usize = 4;

/// 还没有实测样本时假定的通过率。
///
/// 2026-08-23 修完词表/prompt/查重之后重测:同一批模型产出的入库率从
/// 78.75% 升到 100%,打包应用里真跑一次是 12/13(唯一的丢弃是真·完全同文
/// 重复)。残余损失主要来自"这个场景写得差不多了"的重复,而不是格式或词表。
/// 所以先验取 0.9 —— 取低了每批都白要一成句子,取高了不过是多跑一个补足批。
const ASSUMED_ACCEPT_RATE: f64 = 0.90;

/// 通过率的可信下限:再低也不按更低的值放大请求量。
/// 0.6 对应最多 1.67× 的超额请求 —— 20 句的微批最多要到 34 句,
/// 按实测每句 ~150 token 算仍远在 `max_tokens = 8192` 之内,不会被截断。
const MIN_ACCEPT_RATE: f64 = 0.60;

/// 估算通过率前至少要有的样本量(句)。太少的样本会把一两次意外放大。
const MIN_RATE_SAMPLE: u32 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchState {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    /// Stopped by the user or an interruption; resumable.
    Paused,
    Completed,
    /// Cancelled for good (kept for history).
    Cancelled,
}

/// 生成模式(《场景练习模块-实现方案》§3.4)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenMode {
    /// 按等级生成练习句(默认;老任务反序列化后即此值)。
    #[default]
    Level,
    /// 场景对话:双角色真实对话,不受词表带约束。
    Scenario,
}

/// Parameters of one workshop job (§4.4 场景生成流).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobParams {
    pub scene: String,
    /// 场景对话模式下仅作参考难度记录(不约束生成/校验)。
    pub level: String,
    pub total_sentences: u32,
    pub microbatch: u32,
    pub channel: String,
    pub model: String,
    #[serde(default)]
    pub mode: GenMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenJob {
    pub job_id: u64,
    pub params: JobParams,
    pub state: JobState,
    /// One entry per micro-batch (进度点 ●●●○○, §5.5).
    pub batches: Vec<BatchState>,
    /// Sentences accepted so far (已过校验先行入库, §6.3).
    pub produced: u32,
    /// 本任务累计**向模型要过**多少句。与 `produced` 一起给出实测通过率,
    /// 让后续批次按需超额请求(见 [`GenJob::request_size`])。
    /// 老任务的持久化记录没有这个字段,反序列化后为 0 = 尚无样本。
    #[serde(default)]
    pub requested: u32,
    pub created_at: i64,
}

impl GenJob {
    /// Split `total` sentences into micro-batches of `microbatch` (last batch
    /// takes the remainder).
    pub fn new(job_id: u64, params: JobParams, now: i64) -> Self {
        let n = params.total_sentences.div_ceil(params.microbatch.max(1)) as usize;
        Self {
            job_id,
            params,
            state: JobState::Running,
            batches: vec![BatchState::Pending; n.max(1)],
            produced: 0,
            requested: 0,
            created_at: now,
        }
    }

    /// Sentences requested by batch `idx`.
    pub fn batch_size(&self, idx: usize) -> u32 {
        let mb = self.params.microbatch.max(1);
        let full = self.params.total_sentences / mb;
        if idx < full as usize {
            mb
        } else {
            let rem = self.params.total_sentences % mb;
            if rem == 0 { mb } else { rem }
        }
    }

    /// Batches planned by the original `total/microbatch` split; batches at or
    /// beyond this index are top-up batches appended by [`push_topup_batch`].
    ///
    /// [`push_topup_batch`]: Self::push_topup_batch
    pub fn planned_batches(&self) -> usize {
        (self
            .params
            .total_sentences
            .div_ceil(self.params.microbatch.max(1)) as usize)
            .max(1)
    }

    /// How many sentences are still missing to reach the user's target.
    pub fn shortfall(&self) -> u32 {
        self.params.total_sentences.saturating_sub(self.produced)
    }

    /// Top-up batches appended so far.
    pub fn topup_count(&self) -> usize {
        self.batches.len().saturating_sub(self.planned_batches())
    }

    /// 拿满:append one top-up batch to cover the shortfall. A job that had
    /// already completed goes back to Running so [`next_pending`] hands the
    /// new batch out.
    ///
    /// [`next_pending`]: Self::next_pending
    pub fn push_topup_batch(&mut self) {
        self.batches.push(BatchState::Pending);
        if self.state == JobState::Completed {
            self.state = JobState::Running;
        }
    }

    /// 本任务到目前为止的实测通过率(入库句数 ÷ 要过的句数)。
    /// 样本不足时用先验值 [`ASSUMED_ACCEPT_RATE`]。
    pub fn accept_rate(&self) -> f64 {
        if self.requested < MIN_RATE_SAMPLE {
            return ASSUMED_ACCEPT_RATE;
        }
        (self.produced as f64 / self.requested as f64).clamp(MIN_ACCEPT_RATE, 1.0)
    }

    /// 记下这一批向模型要了多少句(通过率的分母)。
    pub fn record_request(&mut self, asked: u32) {
        self.requested = self.requested.saturating_add(asked);
    }

    /// Sentences to *ask the model for* in batch `idx`。
    ///
    /// 一律按"还差多少 ÷ 实测通过率"要,并以"一个微批的量 ÷ 通过率"封顶:
    /// 微批是**交付**粒度(进度点),要多少才能交付这么多得看通过率。
    ///
    /// 为什么要改:原来规划批一句不多要,注定差一截,于是几乎每个任务都要
    /// 额外跑补足批 —— 而补足批要重发整个 system prompt(约 2500 字符),
    /// 补一次的开销比多要几句大得多。按通过率一次要够,常见情况下 20 句的
    /// 任务从"2 次请求"降到"1 次"。
    ///
    /// 反过来也不会浪费:通过率高时公式自然退化成"还差多少要多少",
    /// 已经拿满就不再要(返回 0,调用方据此收尾)。
    pub fn request_size(&self, idx: usize) -> u32 {
        let _ = idx; // 规划批与补足批用同一套规则,自我校正
        let remaining = self.shortfall();
        if remaining == 0 {
            return 0;
        }
        let rate = self.accept_rate();
        let inflate = |n: u32| ((n as f64 / rate).ceil() as u32).max(n);
        let microbatch = self.params.microbatch.max(1);
        inflate(remaining).min(inflate(microbatch)).max(2)
    }

    /// Next batch to run, if the job is runnable.
    pub fn next_pending(&self) -> Option<usize> {
        if self.state != JobState::Running {
            return None;
        }
        self.batches.iter().position(|b| *b == BatchState::Pending)
    }

    pub fn start_batch(&mut self, idx: usize) {
        if self.batches.get(idx) == Some(&BatchState::Pending) {
            self.batches[idx] = BatchState::Running;
        }
    }

    /// Record a finished batch. `accepted` = sentences that passed validation
    /// and were stored.
    pub fn finish_batch(&mut self, idx: usize, accepted: u32) {
        if let Some(b) = self.batches.get_mut(idx) {
            *b = BatchState::Done;
            self.produced += accepted;
        }
        if self
            .batches
            .iter()
            .all(|b| matches!(b, BatchState::Done | BatchState::Failed))
        {
            self.state = JobState::Completed;
        }
    }

    /// A batch failed (network cut, budget stop mid-batch…). The job pauses;
    /// the batch returns to Pending so [续跑] retries it.
    pub fn fail_batch(&mut self, idx: usize) {
        if let Some(b) = self.batches.get_mut(idx) {
            *b = BatchState::Pending;
        }
        self.state = JobState::Paused;
    }

    /// User pressed [停止]: 即刻停表且保留已产出 (§6.3). In-flight batch goes
    /// back to Pending.
    pub fn pause(&mut self) {
        for b in &mut self.batches {
            if *b == BatchState::Running {
                *b = BatchState::Pending;
            }
        }
        if self.state == JobState::Running {
            self.state = JobState::Paused;
        }
    }

    /// [续跑] (§6.3 断点续跑).
    pub fn resume(&mut self) {
        if self.state == JobState::Paused {
            self.state = JobState::Running;
        }
    }

    pub fn cancel(&mut self) {
        self.pause();
        self.state = JobState::Cancelled;
    }

    pub fn done_batches(&self) -> usize {
        self.batches
            .iter()
            .filter(|b| **b == BatchState::Done)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(total: u32, mb: u32) -> JobParams {
        JobParams {
            scene: "机场值机".into(),
            level: "L3".into(),
            total_sentences: total,
            microbatch: mb,
            channel: "opencode".into(),
            model: "opencode/deepseek-v4-flash".into(),
            mode: GenMode::Level,
        }
    }

    #[test]
    fn legacy_job_json_without_mode_defaults_to_level() {
        // 断点续跑会反序列化旧任务:缺 mode 字段必须回落到按等级生成
        let legacy = r#"{"scene":"机场值机","level":"L3","total_sentences":10,
            "microbatch":10,"channel":"opencode","model":"m"}"#;
        let p: JobParams = serde_json::from_str(legacy).unwrap();
        assert_eq!(p.mode, GenMode::Level);
    }

    #[test]
    fn splits_into_microbatches() {
        let job = GenJob::new(1, params(30, 20), 0);
        assert_eq!(job.batches.len(), 2);
        assert_eq!(job.batch_size(0), 20);
        assert_eq!(job.batch_size(1), 10);
    }

    #[test]
    fn exact_division_has_full_batches() {
        let job = GenJob::new(1, params(40, 20), 0);
        assert_eq!(job.batches.len(), 2);
        assert_eq!(job.batch_size(1), 20);
    }

    #[test]
    fn happy_path_completes() {
        let mut job = GenJob::new(1, params(30, 20), 0);
        while let Some(idx) = job.next_pending() {
            job.start_batch(idx);
            job.finish_batch(idx, job.batch_size(idx) - 2); // some discards
        }
        assert_eq!(job.state, JobState::Completed);
        assert_eq!(job.produced, 26);
    }

    #[test]
    fn failure_pauses_and_batch_is_retryable() {
        let mut job = GenJob::new(1, params(30, 10), 0);
        let idx = job.next_pending().unwrap();
        job.start_batch(idx);
        job.fail_batch(idx);
        assert_eq!(job.state, JobState::Paused);
        assert_eq!(job.next_pending(), None); // paused jobs don't hand out work
        job.resume();
        assert_eq!(job.next_pending(), Some(idx)); // same batch retries
    }

    #[test]
    fn pause_returns_running_batch_to_pending() {
        let mut job = GenJob::new(1, params(30, 10), 0);
        let idx = job.next_pending().unwrap();
        job.start_batch(idx);
        job.pause();
        assert_eq!(job.batches[idx], BatchState::Pending);
        job.resume();
        assert_eq!(job.next_pending(), Some(idx));
    }

    #[test]
    fn topup_batch_reopens_completed_job_until_target_met() {
        // 10 句 1 批,只通过 2 句 → 缺口 8,追加补足批继续拿满。
        let mut job = GenJob::new(1, params(10, 10), 0);
        job.start_batch(0);
        job.finish_batch(0, 2);
        assert_eq!(job.state, JobState::Completed);
        assert_eq!(job.shortfall(), 8);
        assert_eq!(job.planned_batches(), 1);

        job.push_topup_batch();
        assert_eq!(job.state, JobState::Running);
        assert_eq!(job.topup_count(), 1);
        assert_eq!(job.next_pending(), Some(1));
        // 这个用例没调 record_request,样本为 0 → 用先验 0.9:
        // 缺 8 句要 ceil(8/0.9)=9,不超过一个微批的量 ceil(10/0.9)=12。
        assert_eq!(job.request_size(1), 9);

        job.start_batch(1);
        job.finish_batch(1, 7);
        assert_eq!(job.shortfall(), 1);
        job.push_topup_batch();
        // 缺 1 句时至少要 2 句,留丢弃余量。
        assert_eq!(job.request_size(2), 2);
        job.start_batch(2);
        job.finish_batch(2, 1);
        assert_eq!(job.shortfall(), 0);
        // 拿满之后不再索要 —— 调用方据此收尾,不会白跑一批。
        assert_eq!(job.request_size(3), 0);
        assert_eq!(job.state, JobState::Completed);
    }

    /// 规划批就按通过率超额索要:20 句的任务在 0.8 的先验下一次要 25 句,
    /// 常见情况下一个请求就拿满,不必再跑补足批(补足批要重发整个
    /// system prompt,比多要几句贵得多)。
    #[test]
    fn planned_batch_over_asks_by_accept_rate() {
        let job = GenJob::new(1, params(20, 20), 0);
        assert_eq!(job.accept_rate(), 0.90, "无样本时用先验");
        assert_eq!(job.request_size(0), 23); // ceil(20 / 0.9)
    }

    /// 有了实测样本就按实测走;通过率高时自然退化成"还差多少要多少",
    /// 不会无谓超额。
    #[test]
    fn request_size_follows_measured_rate() {
        let mut job = GenJob::new(1, params(40, 20), 0);
        job.record_request(20);
        job.start_batch(0);
        job.finish_batch(0, 20); // 20/20 = 100%
        assert_eq!(job.accept_rate(), 1.0);
        assert_eq!(job.request_size(1), 20, "全过时不多要一句");

        let mut job = GenJob::new(2, params(40, 20), 0);
        job.record_request(20);
        job.start_batch(0);
        job.finish_batch(0, 10); // 10/20 = 50% → 夹到下限 0.6
        assert_eq!(job.accept_rate(), 0.60);
        // 还缺 30 句,但一批只按"交付一个微批"索要:ceil(20/0.6)=34。
        assert_eq!(job.request_size(1), 34);
    }

    /// 样本太少不做估算,免得一两次意外把请求量带偏。
    #[test]
    fn tiny_sample_falls_back_to_prior() {
        let mut job = GenJob::new(1, params(20, 20), 0);
        job.record_request(4);
        job.finish_batch(0, 0);
        assert_eq!(job.accept_rate(), 0.90);
    }

    #[test]
    fn topup_batch_survives_pause_resume() {
        let mut job = GenJob::new(1, params(10, 10), 0);
        job.start_batch(0);
        job.finish_batch(0, 3);
        job.push_topup_batch();
        let idx = job.next_pending().unwrap();
        job.start_batch(idx);
        job.pause();
        assert_eq!(job.state, JobState::Paused);
        job.resume();
        assert_eq!(job.next_pending(), Some(idx), "补足批断点续跑");
        assert_eq!(job.topup_count(), 1);
    }

    #[test]
    fn produced_survives_interruption() {
        let mut job = GenJob::new(1, params(30, 10), 0);
        job.start_batch(0);
        job.finish_batch(0, 9);
        job.start_batch(1);
        job.fail_batch(1);
        assert_eq!(job.produced, 9, "已过校验句先行入库");
        assert_eq!(job.done_batches(), 1);
    }
}
