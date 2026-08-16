//! Generation task queue (spec §7.5 `gen_queue`): micro-batches, resumable
//! jobs, half-finished output banked early (半成品先入库).
//!
//! This is the pure state machine; persistence (progress.db) and execution
//! (adapters) live in the app layer. Every transition returns a new state so
//! the app can persist after each step — a crash or 断网 loses at most the
//! in-flight batch, and [续跑] resumes from the next pending one (§6.3).

use serde::{Deserialize, Serialize};

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
