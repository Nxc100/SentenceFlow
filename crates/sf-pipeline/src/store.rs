//! SQLite content store — content.db / user_content.db (spec §7.7).
//!
//! Both databases share one schema; `meta.origin` distinguishes `factory`
//! from `user`. The practice engine reads them through a unified
//! [`ContentIndex`] and is oblivious to origin (§4.3).
//!
//! Schema note (deviation from §7.7, recorded): the doc lists `note_tpl` /
//! `note_slots` columns plus a `note_tpl` table; note templating is a
//! factory-side token-saving device, so the store keeps a single denormalized
//! `note` column and templating stays inside the generation prompt layer.

use rusqlite::{Connection, OpenFlags, params};
use sf_core::sentence::{LevelId, Sentence};
use sf_core::spec::LevelSpec;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("data error: {0}")]
    Data(String),
}

type Result<T> = std::result::Result<T, StoreError>;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS sentence (
    id       INTEGER PRIMARY KEY,
    level    TEXT NOT NULL,
    scene    TEXT NOT NULL DEFAULT '',
    func     TEXT NOT NULL DEFAULT '',
    pattern  TEXT NOT NULL DEFAULT '',
    zh       TEXT NOT NULL,
    en       TEXT NOT NULL,
    punct    TEXT NOT NULL DEFAULT '',
    words    TEXT NOT NULL,           -- JSON [Word]
    chunks   TEXT NOT NULL,           -- JSON [Chunk]
    note     TEXT NOT NULL DEFAULT '',
    simhash  INTEGER NOT NULL DEFAULT 0,
    license  TEXT NOT NULL DEFAULT 'proprietary',
    attribution TEXT NOT NULL DEFAULT '',
    rev      INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_sentence_level ON sentence(level);
CREATE INDEX IF NOT EXISTS idx_sentence_scene ON sentence(level, scene);
CREATE TABLE IF NOT EXISTS lemma (
    lemma    TEXT PRIMARY KEY,
    band     INTEGER NOT NULL,
    ipa_gb   TEXT NOT NULL DEFAULT '',
    ipa_us   TEXT NOT NULL DEFAULT '',
    zh_gloss TEXT NOT NULL DEFAULT '',
    audio    TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS level_spec (
    id   TEXT PRIMARY KEY,
    yaml TEXT NOT NULL
);
"#;

pub struct ContentStore {
    conn: Connection,
}

impl ContentStore {
    /// Create (or open read-write) a content database and ensure the schema.
    pub fn create(path: &Path, origin: &str, rev: u32) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        let store = Self { conn };
        store.set_meta("origin", origin)?;
        store.set_meta("rev", &rev.to_string())?;
        Ok(store)
    }

    /// Open an existing database read-only (the shipped content.db).
    pub fn open_readonly(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self { conn })
    }

    /// Open read-write without recreating meta (user_content.db at runtime).
    pub fn open_rw(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_meta(&self, key: &str) -> Result<Option<String>> {
        let mut stmt = self.conn.prepare("SELECT value FROM meta WHERE key = ?1")?;
        let mut rows = stmt.query(params![key])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// Insert a validated sentence; returns its row id.
    pub fn insert_sentence(&self, s: &Sentence, attribution: &str, rev: u32) -> Result<i64> {
        let words = serde_json::to_string(&s.words).map_err(|e| StoreError::Data(e.to_string()))?;
        let chunks =
            serde_json::to_string(&s.chunks).map_err(|e| StoreError::Data(e.to_string()))?;
        self.conn.execute(
            "INSERT INTO sentence
               (level, scene, func, pattern, zh, en, punct, words, chunks, note,
                simhash, license, attribution, rev)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                s.level.as_str(),
                s.scene,
                s.func,
                s.pattern,
                s.zh,
                s.en,
                s.punct,
                words,
                chunks,
                s.note,
                s.simhash as i64,
                if attribution.is_empty() {
                    "proprietary"
                } else {
                    "CC BY"
                },
                attribution,
                rev,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn insert_lemma(
        &self,
        lemma: &str,
        band: u32,
        ipa_gb: &str,
        ipa_us: &str,
        zh_gloss: &str,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO lemma(lemma, band, ipa_gb, ipa_us, zh_gloss)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(lemma) DO UPDATE SET
               band = excluded.band, ipa_gb = excluded.ipa_gb,
               ipa_us = excluded.ipa_us, zh_gloss = excluded.zh_gloss",
            params![lemma, band, ipa_gb, ipa_us, zh_gloss],
        )?;
        Ok(())
    }

    pub fn insert_level_spec(&self, spec: &LevelSpec, yaml: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO level_spec(id, yaml) VALUES (?1, ?2)
             ON CONFLICT(id) DO UPDATE SET yaml = excluded.yaml",
            params![spec.id.as_str(), yaml],
        )?;
        Ok(())
    }

    pub fn load_level_specs(&self) -> Result<Vec<LevelSpec>> {
        let mut stmt = self
            .conn
            .prepare("SELECT yaml FROM level_spec ORDER BY id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut specs = Vec::new();
        for yaml in rows {
            let yaml = yaml?;
            specs.push(LevelSpec::from_yaml(&yaml).map_err(|e| StoreError::Data(e.to_string()))?);
        }
        Ok(specs)
    }

    fn row_to_sentence(row: &rusqlite::Row<'_>) -> rusqlite::Result<Sentence> {
        let level_str: String = row.get("level")?;
        let words_json: String = row.get("words")?;
        let chunks_json: String = row.get("chunks")?;
        let to_sql_err = |e: String| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            )
        };
        Ok(Sentence {
            id: row.get("id")?,
            level: level_str.parse::<LevelId>().map_err(to_sql_err)?,
            scene: row.get("scene")?,
            func: row.get("func")?,
            pattern: row.get("pattern")?,
            zh: row.get("zh")?,
            en: row.get("en")?,
            punct: row.get("punct")?,
            words: serde_json::from_str(&words_json).map_err(|e| to_sql_err(e.to_string()))?,
            chunks: serde_json::from_str(&chunks_json).map_err(|e| to_sql_err(e.to_string()))?,
            note: row.get("note")?,
            simhash: row.get::<_, i64>("simhash")? as u64,
        })
    }

    pub fn sentences_by_level(&self, level: LevelId) -> Result<Vec<Sentence>> {
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM sentence WHERE level = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![level.as_str()], Self::row_to_sentence)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn sentence_by_id(&self, id: i64) -> Result<Option<Sentence>> {
        let mut stmt = self.conn.prepare("SELECT * FROM sentence WHERE id = ?1")?;
        let mut rows = stmt.query_map(params![id], Self::row_to_sentence)?;
        Ok(rows.next().transpose()?)
    }

    /// Exact-text lookup (trial-progress import matches by `en`, §7.9).
    pub fn sentence_id_by_en(&self, en: &str) -> Result<Option<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM sentence WHERE en = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![en.trim()])?;
        Ok(rows.next()?.map(|r| r.get(0)).transpose()?)
    }

    /// Delete one sentence (user library only — the caller enforces that).
    pub fn delete_sentence(&self, id: i64) -> Result<()> {
        self.conn
            .execute("DELETE FROM sentence WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn all_simhashes(&self) -> Result<Vec<u64>> {
        let mut stmt = self.conn.prepare("SELECT simhash FROM sentence")?;
        let rows = stmt.query_map([], |r| r.get::<_, i64>(0))?;
        Ok(rows
            .map(|r| r.map(|v| v as u64))
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn sentence_count(&self) -> Result<u32> {
        Ok(self
            .conn
            .query_row("SELECT COUNT(*) FROM sentence", [], |r| r.get(0))?)
    }

    /// Distinct scenes per level (library grouping, §4.3).
    pub fn scenes(&self, level: LevelId) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT scene FROM sentence WHERE level = ?1 ORDER BY scene")?;
        let rows = stmt.query_map(params![level.as_str()], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// Dump the lemma table as validator-ready TSV (client-side lexicon reuse).
    pub fn lemma_tsv(&self) -> Result<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT lemma, band, ipa_gb, ipa_us, zh_gloss FROM lemma")?;
        let rows = stmt.query_map([], |r| {
            Ok(format!(
                "{}\t{}\t{}\t{}\t{}",
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?
            ))
        })?;
        let mut out = String::new();
        for line in rows {
            out.push_str(&line?);
            out.push('\n');
        }
        Ok(out)
    }
}

/// Unified read view over factory + user content (§4.3 ContentIndex).
pub struct ContentIndex {
    pub factory: ContentStore,
    pub user: Option<ContentStore>,
}

impl ContentIndex {
    /// User sentence ids are offset into a disjoint range so both stores can
    /// be addressed through one id space. Factory content stays < this bound.
    pub const USER_ID_OFFSET: i64 = 1_000_000_000;

    pub fn sentences_by_level(&self, level: LevelId) -> Result<Vec<Sentence>> {
        let mut out = self.factory.sentences_by_level(level)?;
        if let Some(user) = &self.user {
            out.extend(user.sentences_by_level(level)?.into_iter().map(|mut s| {
                s.id += Self::USER_ID_OFFSET;
                s
            }));
        }
        Ok(out)
    }

    pub fn sentence_by_id(&self, id: i64) -> Result<Option<Sentence>> {
        if id >= Self::USER_ID_OFFSET {
            match &self.user {
                Some(user) => Ok(user
                    .sentence_by_id(id - Self::USER_ID_OFFSET)?
                    .map(|mut s| {
                        s.id = id;
                        s
                    })),
                None => Ok(None),
            }
        } else {
            self.factory.sentence_by_id(id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::sentence::{Chunk, PosTag, RoleTag, Word};

    fn sample(level: LevelId, en: &str) -> Sentence {
        Sentence {
            id: 0,
            level,
            scene: "问候".into(),
            func: "打招呼".into(),
            pattern: "主+系+表".into(),
            zh: "我很好。".into(),
            en: en.into(),
            punct: ".".into(),
            words: vec![
                Word {
                    w: "I".into(),
                    ipa: "aɪ".into(),
                    pos: PosTag::Pronoun,
                },
                Word {
                    w: "am".into(),
                    ipa: "æm".into(),
                    pos: PosTag::Auxiliary,
                },
                Word {
                    w: "fine".into(),
                    ipa: "faɪn".into(),
                    pos: PosTag::Adjective,
                },
            ],
            chunks: vec![
                Chunk {
                    r: RoleTag::Subject,
                    i: vec![0],
                },
                Chunk {
                    r: RoleTag::Linking,
                    i: vec![1],
                },
                Chunk {
                    r: RoleTag::Complement,
                    i: vec![2],
                },
            ],
            note: "note".into(),
            simhash: 42,
        }
    }

    fn temp_db(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("sf-store-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn roundtrip_sentence() {
        let path = temp_db("roundtrip");
        let store = ContentStore::create(&path, "factory", 1).unwrap();
        let id = store
            .insert_sentence(&sample(LevelId::L1, "I am fine."), "", 1)
            .unwrap();
        let got = store.sentence_by_id(id).unwrap().unwrap();
        assert_eq!(got.en, "I am fine.");
        assert_eq!(got.words[2].pos, PosTag::Adjective);
        assert_eq!(got.simhash, 42);
        assert_eq!(store.sentence_count().unwrap(), 1);
        assert_eq!(store.get_meta("origin").unwrap().unwrap(), "factory");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn level_query_and_scenes() {
        let path = temp_db("levels");
        let store = ContentStore::create(&path, "factory", 1).unwrap();
        store
            .insert_sentence(&sample(LevelId::L1, "a"), "", 1)
            .unwrap();
        store
            .insert_sentence(&sample(LevelId::L2, "b"), "", 1)
            .unwrap();
        assert_eq!(store.sentences_by_level(LevelId::L1).unwrap().len(), 1);
        assert_eq!(store.scenes(LevelId::L1).unwrap(), vec!["问候"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn content_index_merges_and_offsets_user_ids() {
        let fpath = temp_db("idx-factory");
        let upath = temp_db("idx-user");
        let factory = ContentStore::create(&fpath, "factory", 1).unwrap();
        let user = ContentStore::create(&upath, "user", 1).unwrap();
        factory
            .insert_sentence(&sample(LevelId::L1, "factory sentence"), "", 1)
            .unwrap();
        let uid = user
            .insert_sentence(&sample(LevelId::L1, "user sentence"), "", 1)
            .unwrap();
        let idx = ContentIndex {
            factory,
            user: Some(user),
        };
        let all = idx.sentences_by_level(LevelId::L1).unwrap();
        assert_eq!(all.len(), 2);
        let user_row = idx
            .sentence_by_id(ContentIndex::USER_ID_OFFSET + uid)
            .unwrap()
            .unwrap();
        assert_eq!(user_row.en, "user sentence");
        let _ = std::fs::remove_file(&fpath);
        let _ = std::fs::remove_file(&upath);
    }

    #[test]
    fn lemma_tsv_roundtrips_into_lexicon() {
        let path = temp_db("lemma");
        let store = ContentStore::create(&path, "factory", 1).unwrap();
        store.insert_lemma("be", 1, "bi", "bi", "是").unwrap();
        store.insert_lemma("go", 40, "ɡəʊ", "ɡoʊ", "去").unwrap();
        let tsv = store.lemma_tsv().unwrap();
        let lex = crate::lexicon::Lexicon::from_tsv(&tsv).unwrap();
        assert_eq!(lex.band_of("went"), Some(40));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn spec_snapshot_roundtrip() {
        let yaml = r#"
id: L1
cefr: "A1"
vocab_band: 500
max_words: 8
grammar_whitelist: [be_present]
can_do: ["打招呼"]
practice:
  flow: reorder_then_typing
  review_listening_ratio: 0.0
  dictation_min_box: 0
  hints: { ipa: always, first_letter: true, zh_hideable: false }
  judge: { strict: false }
  srs:
    daily_new_default: 10
    daily_new_range: [5, 50]
    review_cap: 60
    box_intervals_days: [1, 2, 4, 7]
    box5_recheck_days: 12
    listening_weight: 1.5
"#;
        let path = temp_db("spec");
        let store = ContentStore::create(&path, "factory", 1).unwrap();
        let spec = LevelSpec::from_yaml(yaml).unwrap();
        store.insert_level_spec(&spec, yaml).unwrap();
        let specs = store.load_level_specs().unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0], spec);
        let _ = std::fs::remove_file(&path);
    }
}
