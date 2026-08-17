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

/// Parameters of one workshop job (§4.4 场景生成流).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobParams {
    pub scene: String,
    pub level: String,
    pub total_sentences: u32,
    pub microbatch: u32,
    pub channel: String,
    pub model: String,
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

    /// Sentences to *ask the model for* in batch `idx`. Planned batches follow
    /// the original split; top-up batches ask for the current shortfall plus
    /// 50% headroom (the discard rate is unknown), clamped to the microbatch.
    pub fn request_size(&self, idx: usize) -> u32 {
        if idx < self.planned_batches() {
            self.batch_size(idx)
        } else {
            let want = self.shortfall();
            (want + want.div_ceil(2)).clamp(2, self.params.microbatch.max(2))
        }
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
        }
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
        // 补足批按缺口 + 50% 超量索要,夹在 microbatch 内:8+4=12 → 10。
        assert_eq!(job.request_size(1), 10);

        job.start_batch(1);
        job.finish_batch(1, 7);
        assert_eq!(job.shortfall(), 1);
        job.push_topup_batch();
        // 缺 1 句时至少要 2 句,留丢弃余量。
        assert_eq!(job.request_size(2), 2);
        job.start_batch(2);
        job.finish_batch(2, 1);
        assert_eq!(job.shortfall(), 0);
        assert_eq!(job.state, JobState::Completed);
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
