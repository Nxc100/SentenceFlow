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
CREATE TABLE IF NOT EXISTS chat_thread (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    mode        TEXT NOT NULL,             -- free | roleplay | agent
    title       TEXT NOT NULL DEFAULT '',
    role_id     TEXT NOT NULL DEFAULT '',  -- 角色扮演的角色卡 id
    role_system TEXT NOT NULL DEFAULT '',  -- 角色卡的人设描述(发送时组入 system)
    oc_session  TEXT NOT NULL DEFAULT '',  -- opencode 服务端会话 id
    workdir     TEXT NOT NULL DEFAULT '',  -- 智能体工作目录
    channel     TEXT NOT NULL DEFAULT '',  -- 本会话固定通道(空 = 跟随设置)
    model       TEXT NOT NULL DEFAULT '',  -- 本会话固定模型(空 = 跟随设置)
    model_label TEXT NOT NULL DEFAULT '',  -- 模型展示名(界面不露原始 id)
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_message (
    id        INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL,
    role      TEXT NOT NULL,               -- user | assistant
    text      TEXT NOT NULL,
    fix_json  TEXT NOT NULL DEFAULT '',    -- 纠错卡 JSON(无纠错为空)
    ts        INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_chat_message_thread ON chat_message(thread_id);
"#;

pub struct ProgressDb {
    conn: Connection,
}

impl ProgressDb {
    pub fn open(path: &Path) -> CmdResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        let db = Self { conn };
        db.migrate_chat_model_columns()?;
        Ok(db)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> CmdResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        let db = Self { conn };
        db.migrate_chat_model_columns()?;
        Ok(db)
    }

    /// 幂等加列:v0.1.0 发出去的库里 `chat_thread` 没有每会话模型三列
    /// (`CREATE TABLE IF NOT EXISTS` 不会补列)。SQLite 加列是 O(1) 元数据
    /// 操作,缺哪列补哪列 —— 与 sf-pipeline 的 `pack` 列迁移同一套路。
    fn migrate_chat_model_columns(&self) -> CmdResult<()> {
        for (col, ddl) in [
            (
                "channel",
                "ALTER TABLE chat_thread ADD COLUMN channel TEXT NOT NULL DEFAULT ''",
            ),
            (
                "model",
                "ALTER TABLE chat_thread ADD COLUMN model TEXT NOT NULL DEFAULT ''",
            ),
            (
                "model_label",
                "ALTER TABLE chat_thread ADD COLUMN model_label TEXT NOT NULL DEFAULT ''",
            ),
        ] {
            let present = self
                .conn
                .prepare(&format!("SELECT {col} FROM chat_thread LIMIT 1"))
                .is_ok();
            if !present {
                self.conn.execute_batch(ddl)?;
            }
        }
        Ok(())
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

    /// 练习过的句子 id 集合(场景包「已练」标记;场景句不写 srs,
    /// 所以不能用 [`Self::seen_ids`] 判断)。
    pub fn logged_sentence_ids(&self) -> CmdResult<std::collections::HashSet<i64>> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT sentence_id FROM log")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

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

    // ------------------------------------------------------------- chat (AI 聊天模块)

    #[allow(clippy::too_many_arguments)]
    pub fn chat_thread_create(
        &self,
        mode: &str,
        title: &str,
        role_id: &str,
        role_system: &str,
        workdir: &str,
        now: i64,
    ) -> CmdResult<i64> {
        self.conn.execute(
            "INSERT INTO chat_thread (mode, title, role_id, role_system, workdir,
                                      created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![mode, title, role_id, role_system, workdir, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn chat_threads(&self) -> CmdResult<Vec<ChatThreadRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mode, title, role_id, role_system, oc_session, workdir,
                    channel, model, model_label, updated_at
             FROM chat_thread ORDER BY updated_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], row_to_chat_thread)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn chat_thread_get(&self, id: i64) -> CmdResult<Option<ChatThreadRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mode, title, role_id, role_system, oc_session, workdir,
                    channel, model, model_label, updated_at
             FROM chat_thread WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], row_to_chat_thread)?;
        Ok(rows.next().transpose()?)
    }

    /// 本会话固定通道/模型(三者同时为空 = 跟随全局设置)。
    pub fn chat_thread_set_model(
        &self,
        id: i64,
        channel: &str,
        model: &str,
        model_label: &str,
    ) -> CmdResult<()> {
        self.conn.execute(
            "UPDATE chat_thread SET channel = ?2, model = ?3, model_label = ?4
             WHERE id = ?1",
            params![id, channel, model, model_label],
        )?;
        Ok(())
    }

    pub fn chat_thread_set_session(&self, id: i64, session: &str) -> CmdResult<()> {
        self.conn.execute(
            "UPDATE chat_thread SET oc_session = ?2 WHERE id = ?1",
            params![id, session],
        )?;
        Ok(())
    }

    pub fn chat_thread_touch(&self, id: i64, now: i64) -> CmdResult<()> {
        self.conn.execute(
            "UPDATE chat_thread SET updated_at = ?2 WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    /// Delete a thread and its messages.
    pub fn chat_thread_delete(&self, id: i64) -> CmdResult<()> {
        self.conn
            .execute("DELETE FROM chat_message WHERE thread_id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM chat_thread WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn chat_messages(&self, thread_id: i64) -> CmdResult<Vec<ChatMessageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, text, fix_json, ts FROM chat_message
             WHERE thread_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map(params![thread_id], row_to_chat_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Last `limit` messages in chronological order (回放窗口).
    pub fn chat_recent_messages(
        &self,
        thread_id: i64,
        limit: u32,
    ) -> CmdResult<Vec<ChatMessageRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, role, text, fix_json, ts FROM (
                 SELECT id, role, text, fix_json, ts FROM chat_message
                 WHERE thread_id = ?1 ORDER BY id DESC LIMIT ?2
             ) ORDER BY id",
        )?;
        let rows = stmt.query_map(params![thread_id, limit], row_to_chat_message)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn chat_message_add(
        &self,
        thread_id: i64,
        role: &str,
        text: &str,
        fix_json: &str,
        ts: i64,
    ) -> CmdResult<i64> {
        self.conn.execute(
            "INSERT INTO chat_message (thread_id, role, text, fix_json, ts)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![thread_id, role, text, fix_json, ts],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn chat_message_count(&self, thread_id: i64) -> CmdResult<u32> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM chat_message WHERE thread_id = ?1",
            params![thread_id],
            |r| r.get(0),
        )?)
    }
}

/// One chat thread (会话) row.
#[derive(Debug, Clone)]
pub struct ChatThreadRow {
    pub id: i64,
    pub mode: String,
    pub title: String,
    pub role_id: String,
    pub role_system: String,
    pub oc_session: String,
    pub workdir: String,
    /// 本会话固定通道(空 = 跟随全局设置)
    pub channel: String,
    /// 本会话固定模型(空 = 跟随全局设置)
    pub model: String,
    pub model_label: String,
    pub updated_at: i64,
}

/// One chat message row.
#[derive(Debug, Clone)]
pub struct ChatMessageRow {
    pub id: i64,
    pub role: String,
    pub text: String,
    pub fix_json: String,
    pub ts: i64,
}

fn row_to_chat_thread(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatThreadRow> {
    Ok(ChatThreadRow {
        id: r.get(0)?,
        mode: r.get(1)?,
        title: r.get(2)?,
        role_id: r.get(3)?,
        role_system: r.get(4)?,
        oc_session: r.get(5)?,
        workdir: r.get(6)?,
        channel: r.get(7)?,
        model: r.get(8)?,
        model_label: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

fn row_to_chat_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChatMessageRow> {
    Ok(ChatMessageRow {
        id: r.get(0)?,
        role: r.get(1)?,
        text: r.get(2)?,
        fix_json: r.get(3)?,
        ts: r.get(4)?,
    })
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
                mode: sf_llm::queue::GenMode::Level,
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

    #[test]
    fn chat_thread_and_messages_roundtrip() {
        let db = ProgressDb::open_in_memory().unwrap();
        let t1 = db
            .chat_thread_create("free", "聊聊周末", "", "", "", 100)
            .unwrap();
        let t2 = db
            .chat_thread_create("roleplay", "面试官", "interviewer", "You are…", "", 200)
            .unwrap();
        // list ordered by updated_at desc
        let threads = db.chat_threads().unwrap();
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0].id, t2);
        assert_eq!(threads[0].role_id, "interviewer");

        db.chat_message_add(t1, "user", "Hello", "", 101).unwrap();
        db.chat_message_add(t1, "assistant", "Hi!", r#"{"better":"x"}"#, 102)
            .unwrap();
        db.chat_thread_touch(t1, 300).unwrap();
        assert_eq!(db.chat_threads().unwrap()[0].id, t1);
        assert_eq!(db.chat_message_count(t1).unwrap(), 2);
        let msgs = db.chat_messages(t1).unwrap();
        assert_eq!(msgs[0].text, "Hello");
        assert_eq!(msgs[1].fix_json, r#"{"better":"x"}"#);

        db.chat_thread_set_session(t1, "ses_x").unwrap();
        assert_eq!(db.chat_thread_get(t1).unwrap().unwrap().oc_session, "ses_x");

        // recent window keeps chronological order of the tail
        for i in 0..5 {
            db.chat_message_add(t1, "user", &format!("m{i}"), "", 110 + i)
                .unwrap();
        }
        let recent = db.chat_recent_messages(t1, 3).unwrap();
        assert_eq!(
            recent.iter().map(|m| m.text.as_str()).collect::<Vec<_>>(),
            vec!["m2", "m3", "m4"]
        );

        // delete cascades to messages
        db.chat_thread_delete(t1).unwrap();
        assert_eq!(db.chat_message_count(t1).unwrap(), 0);
        assert!(db.chat_thread_get(t1).unwrap().is_none());
    }

    #[test]
    fn chat_thread_model_override_roundtrip() {
        let db = ProgressDb::open_in_memory().unwrap();
        let id = db.chat_thread_create("free", "t", "", "", "", 1).unwrap();
        // 默认跟随设置:三列皆空
        let t = db.chat_thread_get(id).unwrap().unwrap();
        assert_eq!((t.channel.as_str(), t.model.as_str()), ("", ""));

        db.chat_thread_set_model(id, "opencode", "opencode/hy3-free", "hy3-free")
            .unwrap();
        let t = db.chat_thread_get(id).unwrap().unwrap();
        assert_eq!(t.channel, "opencode");
        assert_eq!(t.model, "opencode/hy3-free");
        assert_eq!(t.model_label, "hy3-free");

        // 改回跟随设置
        db.chat_thread_set_model(id, "", "", "").unwrap();
        assert_eq!(db.chat_thread_get(id).unwrap().unwrap().model, "");
    }

    /// v0.1.0 已发出的库里 chat_thread 没有模型三列 —— 打开时必须自动补列,
    /// 且旧数据完好(真机升级路径)。
    #[test]
    fn old_chat_thread_table_gains_model_columns() {
        let dir = std::env::temp_dir().join(format!("sf-migrate-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("progress.db");
        let _ = std::fs::remove_file(&path);
        {
            // 模拟旧库:老版本的建表语句(无 channel/model/model_label)
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE chat_thread (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     mode TEXT NOT NULL, title TEXT NOT NULL DEFAULT '',
                     role_id TEXT NOT NULL DEFAULT '', role_system TEXT NOT NULL DEFAULT '',
                     oc_session TEXT NOT NULL DEFAULT '', workdir TEXT NOT NULL DEFAULT '',
                     created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
                 INSERT INTO chat_thread (mode, title, oc_session, created_at, updated_at)
                 VALUES ('free', '老会话', 'ses_old', 10, 10);",
            )
            .unwrap();
        }
        let db = ProgressDb::open(&path).unwrap();
        let threads = db.chat_threads().unwrap();
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].title, "老会话");
        assert_eq!(threads[0].oc_session, "ses_old");
        assert_eq!(threads[0].model, ""); // 补列后默认跟随设置
        db.chat_thread_set_model(threads[0].id, "zen", "opencode/x", "x")
            .unwrap();
        assert_eq!(db.chat_threads().unwrap()[0].model, "opencode/x");
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
