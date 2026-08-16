//! progress.db — learning progress store (spec §7.7).
//!
//! Tables: `srs`, `log`, `kv`, `spend`, `gen_queue`, `bench`, plus
//! `favorite` (收藏, §4.3 — a small schema addition over the doc's list,
//! recorded here). The wrong-book (错题本) is *derived* from `srs`
//! (`err ≥ 2 OR marked_unfamiliar`), not a table of its own.

use rusqlite::{Connection, params};
use sf_core::srs::{Mode, SrsState};
use sf_core::stats::{ErrorTag, LogResult, LogRow};
use sf_llm::queue::GenJob;
use std::path::Path;

use crate::error::{CmdError, CmdResult};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS srs (
    sentence_id       INTEGER PRIMARY KEY,
    box               INTEGER NOT NULL,
    progress          REAL    NOT NULL DEFAULT 0,
    due_at            INTEGER NOT NULL,
    err               INTEGER NOT NULL DEFAULT 0,
    last_mode         TEXT,
    last_at           INTEGER NOT NULL,
    seen_answer       INTEGER NOT NULL DEFAULT 0,
    marked_unfamiliar INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_srs_due ON srs(due_at);
CREATE TABLE IF NOT EXISTS log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    ts          INTEGER NOT NULL,
    sentence_id INTEGER NOT NULL,
    mode        TEXT    NOT NULL,
    result      TEXT    NOT NULL,
    dur_ms      INTEGER NOT NULL DEFAULT 0,
    errors      INTEGER NOT NULL DEFAULT 0,
    wpm         REAL    NOT NULL DEFAULT 0,
    seen_answer INTEGER NOT NULL DEFAULT 0,
    error_tags  TEXT    NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS idx_log_ts ON log(ts);
CREATE TABLE IF NOT EXISTS kv (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS spend (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    ts         INTEGER NOT NULL,
    provider   TEXT    NOT NULL,
    tokens_in  INTEGER NOT NULL DEFAULT 0,
    tokens_out INTEGER NOT NULL DEFAULT 0,
    cost_est   REAL    NOT NULL DEFAULT 0,
    requests   INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE IF NOT EXISTS gen_queue (
    job_id   INTEGER PRIMARY KEY,
    payload  TEXT    NOT NULL,
    state    TEXT    NOT NULL,
    produced INTEGER NOT NULL DEFAULT 0,
    ts       INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS bench (
    model            TEXT PRIMARY KEY,
    score            REAL    NOT NULL,
    latency          INTEGER NOT NULL,
    list_fingerprint TEXT    NOT NULL,
    ts               INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS favorite (
    sentence_id INTEGER PRIMARY KEY,
    added_at    INTEGER NOT NULL
);
"#;

pub struct ProgressDb {
    conn: Connection,
}

impl ProgressDb {
    pub fn open(path: &Path) -> CmdResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> CmdResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    // ------------------------------------------------------------- srs

    pub fn get_srs(&self, sentence_id: i64) -> CmdResult<Option<SrsState>> {
        let mut stmt = self.conn.prepare(
            "SELECT box, progress, due_at, err, last_mode, last_at, seen_answer,
                    marked_unfamiliar
             FROM srs WHERE sentence_id = ?1",
        )?;
        let mut rows = stmt.query_map(params![sentence_id], row_to_srs)?;
        Ok(rows.next().transpose()?)
    }

    pub fn all_srs(&self) -> CmdResult<Vec<(i64, SrsState)>> {
        let mut stmt = self.conn.prepare(
            "SELECT sentence_id, box, progress, due_at, err, last_mode, last_at,
                    seen_answer, marked_unfamiliar
             FROM srs",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row_to_srs_at(row, 1)?))
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn upsert_srs(&self, sentence_id: i64, s: &SrsState) -> CmdResult<()> {
        let mode = s.last_mode.map(mode_str);
        self.conn.execute(
            "INSERT INTO srs (sentence_id, box, progress, due_at, err, last_mode,
                              last_at, seen_answer, marked_unfamiliar)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(sentence_id) DO UPDATE SET
               box = excluded.box, progress = excluded.progress,
               due_at = excluded.due_at, err = excluded.err,
               last_mode = excluded.last_mode, last_at = excluded.last_at,
               seen_answer = excluded.seen_answer,
               marked_unfamiliar = excluded.marked_unfamiliar",
            params![
                sentence_id,
                s.box_idx,
                s.progress as f64,
                s.due_at,
                s.err,
                mode,
                s.last_at,
                s.seen_answer,
                s.marked_unfamiliar,
            ],
        )?;
        Ok(())
    }

    /// 错题本 (§4.3): err ≥2 或标不熟悉.
    pub fn wrongbook_ids(&self) -> CmdResult<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT sentence_id FROM srs
             WHERE err >= 2 OR marked_unfamiliar = 1
             ORDER BY last_at DESC",
        )?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Ids the SRS has never seen — candidates for 新句 (§4.5).
    pub fn seen_ids(&self) -> CmdResult<std::collections::HashSet<i64>> {
        let mut stmt = self.conn.prepare("SELECT sentence_id FROM srs")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    // ------------------------------------------------------------- log

    pub fn insert_log(&self, row: &LogRow) -> CmdResult<()> {
        self.conn.execute(
            "INSERT INTO log (ts, sentence_id, mode, result, dur_ms, errors, wpm,
                              seen_answer, error_tags)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                row.ts,
                row.sentence_id,
                mode_str(row.mode),
                match row.result {
                    LogResult::Correct => "correct",
                    LogResult::Wrong => "wrong",
                },
                row.dur_ms,
                row.errors,
                f64::from(row.wpm),
                row.seen_answer,
                serde_json::to_string(&row.error_tags)?,
            ],
        )?;
        Ok(())
    }

    pub fn all_logs(&self) -> CmdResult<Vec<LogRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT ts, sentence_id, mode, result, dur_ms, errors, wpm,
                    seen_answer, error_tags
             FROM log ORDER BY ts",
        )?;
        let rows = stmt.query_map([], |row| {
            let mode: String = row.get(2)?;
            let result: String = row.get(3)?;
            let tags: String = row.get(8)?;
            Ok(LogRow {
                ts: row.get(0)?,
                sentence_id: row.get(1)?,
                mode: parse_mode(&mode),
                result: if result == "correct" {
                    LogResult::Correct
                } else {
                    LogResult::Wrong
                },
                dur_ms: row.get(4)?,
                errors: row.get(5)?,
                wpm: row.get::<_, f64>(6)? as f32,
                seen_answer: row.get(7)?,
                error_tags: serde_json::from_str::<Vec<ErrorTag>>(&tags).unwrap_or_default(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Attempts logged today (体验模式每日 5 句上限, §4.6).
    pub fn attempts_since(&self, since_ts: i64) -> CmdResult<u32> {
        Ok(self.conn.query_row(
            "SELECT COUNT(DISTINCT sentence_id) FROM log WHERE ts >= ?1",
            params![since_ts],
            |r| r.get(0),
        )?)
    }

    // ------------------------------------------------------------- kv

    pub fn kv_get(&self, key: &str) -> CmdResult<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT value FROM kv WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    pub fn kv_set(&self, key: &str, value: &str) -> CmdResult<()> {
        self.conn.execute(
            "INSERT INTO kv(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ------------------------------------------------------------- favorite

    pub fn favorite_add(&self, sentence_id: i64, now: i64) -> CmdResult<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO favorite(sentence_id, added_at) VALUES (?1, ?2)",
            params![sentence_id, now],
        )?;
        Ok(())
    }

    pub fn favorite_remove(&self, sentence_id: i64) -> CmdResult<()> {
        self.conn.execute(
            "DELETE FROM favorite WHERE sentence_id = ?1",
            params![sentence_id],
        )?;
        Ok(())
    }

    pub fn favorite_ids(&self) -> CmdResult<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT sentence_id FROM favorite ORDER BY added_at DESC")?;
        let rows = stmt.query_map([], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ------------------------------------------------------------- spend

    pub fn spend_add(
        &self,
        ts: i64,
        provider: &str,
        tokens_in: u64,
        tokens_out: u64,
        cost_est: f64,
    ) -> CmdResult<()> {
        self.conn.execute(
            "INSERT INTO spend (ts, provider, tokens_in, tokens_out, cost_est)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, provider, tokens_in as i64, tokens_out as i64, cost_est],
        )?;
        Ok(())
    }

    /// (requests, cost) since a timestamp — CostBar 今日 n 次 / 月度提醒 (§4.7).
    pub fn spend_since(&self, since_ts: i64) -> CmdResult<(u32, f64)> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(SUM(requests), 0), COALESCE(SUM(cost_est), 0)
             FROM spend WHERE ts >= ?1",
            params![since_ts],
            |r| Ok((r.get::<_, i64>(0)? as u32, r.get(1)?)),
        )?)
    }

    // ------------------------------------------------------------- gen_queue

    pub fn save_job(&self, job: &GenJob) -> CmdResult<()> {
        self.conn.execute(
            "INSERT INTO gen_queue (job_id, payload, state, produced, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(job_id) DO UPDATE SET
               payload = excluded.payload, state = excluded.state,
               produced = excluded.produced",
            params![
                job.job_id as i64,
                serde_json::to_string(job)?,
                serde_json::to_string(&job.state)?,
                job.produced,
                job.created_at,
            ],
        )?;
        Ok(())
    }

    pub fn load_jobs(&self) -> CmdResult<Vec<GenJob>> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload FROM gen_queue ORDER BY ts DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut jobs = Vec::new();
        for payload in rows {
            jobs.push(serde_json::from_str(&payload?)?);
        }
        Ok(jobs)
    }

    pub fn load_job(&self, job_id: u64) -> CmdResult<Option<GenJob>> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload FROM gen_queue WHERE job_id = ?1")?;
        let mut rows = stmt.query(params![job_id as i64])?;
        match rows.next()? {
            Some(r) => {
                let payload: String = r.get(0)?;
                Ok(Some(serde_json::from_str(&payload)?))
            }
            None => Ok(None),
        }
    }

    // ------------------------------------------------------------- bench

    pub fn save_bench(
        &self,
        model: &str,
        score: f64,
        latency_ms: i64,
        fingerprint: &str,
        ts: i64,
    ) -> CmdResult<()> {
        self.conn.execute(
            "INSERT INTO bench (model, score, latency, list_fingerprint, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(model) DO UPDATE SET
               score = excluded.score, latency = excluded.latency,
               list_fingerprint = excluded.list_fingerprint, ts = excluded.ts",
            params![model, score, latency_ms, fingerprint, ts],
        )?;
        Ok(())
    }

    /// (model, score) best-first for a given list fingerprint.
    pub fn bench_ranking(&self, fingerprint: &str) -> CmdResult<Vec<(String, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT model, score FROM bench WHERE list_fingerprint = ?1
             ORDER BY score DESC",
        )?;
        let rows = stmt.query_map(params![fingerprint], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
}

fn mode_str(m: Mode) -> &'static str {
    match m {
        Mode::Typing => "typing",
        Mode::Reorder => "reorder",
        Mode::Listening => "listening",
        Mode::Dictation => "dictation",
    }
}

fn parse_mode(s: &str) -> Mode {
    match s {
        "reorder" => Mode::Reorder,
        "listening" => Mode::Listening,
        "dictation" => Mode::Dictation,
        _ => Mode::Typing,
    }
}

fn row_to_srs(row: &rusqlite::Row<'_>) -> rusqlite::Result<SrsState> {
    row_to_srs_at(row, 0)
}

fn row_to_srs_at(row: &rusqlite::Row<'_>, base: usize) -> rusqlite::Result<SrsState> {
    let mode: Option<String> = row.get(base + 4)?;
    Ok(SrsState {
        box_idx: row.get(base)?,
        progress: row.get::<_, f64>(base + 1)? as f32,
        due_at: row.get(base + 2)?,
        err: row.get(base + 3)?,
        last_mode: mode.as_deref().map(parse_mode),
        last_at: row.get(base + 5)?,
        seen_answer: row.get(base + 6)?,
        marked_unfamiliar: row.get(base + 7)?,
    })
}

impl From<CmdError> for rusqlite::Error {
    fn from(e: CmdError) -> Self {
        rusqlite::Error::InvalidParameterName(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srs_roundtrip_and_wrongbook() {
        let db = ProgressDb::open_in_memory().unwrap();
        let mut s = SrsState::new(1000);
        s.err = 2;
        s.last_mode = Some(Mode::Listening);
        db.upsert_srs(7, &s).unwrap();
        let got = db.get_srs(7).unwrap().unwrap();
        assert_eq!(got.err, 2);
        assert_eq!(got.last_mode, Some(Mode::Listening));
        assert_eq!(db.wrongbook_ids().unwrap(), vec![7]);
        assert!(db.seen_ids().unwrap().contains(&7));
    }

    #[test]
    fn log_roundtrip() {
        let db = ProgressDb::open_in_memory().unwrap();
        db.insert_log(&LogRow {
            ts: 1,
            sentence_id: 5,
            mode: Mode::Typing,
            result: LogResult::Correct,
            dur_ms: 4000,
            errors: 0,
            wpm: 42.5,
            seen_answer: false,
            error_tags: vec![],
        })
        .unwrap();
        let logs = db.all_logs().unwrap();
        assert_eq!(logs.len(), 1);
        assert!((logs[0].wpm - 42.5).abs() < 1e-6);
        assert_eq!(db.attempts_since(0).unwrap(), 1);
    }

    #[test]
    fn kv_favorites_spend() {
        let db = ProgressDb::open_in_memory().unwrap();
        db.kv_set("settings", "{}").unwrap();
        assert_eq!(db.kv_get("settings").unwrap().unwrap(), "{}");
        db.favorite_add(3, 100).unwrap();
        db.favorite_add(3, 100).unwrap(); // idempotent
        assert_eq!(db.favorite_ids().unwrap(), vec![3]);
        db.favorite_remove(3).unwrap();
        assert!(db.favorite_ids().unwrap().is_empty());
        db.spend_add(50, "deepseek", 1000, 400, 0.01).unwrap();
        let (req, cost) = db.spend_since(0).unwrap();
        assert_eq!(req, 1);
        assert!((cost - 0.01).abs() < 1e-9);
    }

    #[test]
    fn gen_queue_roundtrip() {
        let db = ProgressDb::open_in_memory().unwrap();
        let job = GenJob::new(
            9,
            sf_llm::queue::JobParams {
                scene: "机场".into(),
                level: "L3".into(),
                total_sentences: 30,
                microbatch: 20,
                channel: "opencode".into(),
                model: "opencode/x".into(),
            },
            123,
        );
        db.save_job(&job).unwrap();
        let jobs = db.load_jobs().unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0], job);
        assert_eq!(db.load_job(9).unwrap().unwrap().params.scene, "机场");
    }

    #[test]
    fn bench_ranking_orders() {
        let db = ProgressDb::open_in_memory().unwrap();
        db.save_bench("a", 60.0, 4000, "fp1", 1).unwrap();
        db.save_bench("b", 90.0, 3000, "fp1", 1).unwrap();
        db.save_bench("c", 99.0, 3000, "fp2", 1).unwrap();
        let ranking = db.bench_ranking("fp1").unwrap();
        assert_eq!(ranking[0].0, "b");
        assert_eq!(ranking.len(), 2);
    }
}
