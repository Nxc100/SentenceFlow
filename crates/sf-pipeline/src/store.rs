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
    rev      INTEGER NOT NULL DEFAULT 1,
    -- 场景练习包 id(《场景练习模块-实现方案》§3.2);'' = 普通等级句。
    -- 场景句不参与等级 SRS 队列,取句路径统一过滤 pack = ''。
    pack     TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_sentence_level ON sentence(level);
CREATE INDEX IF NOT EXISTS idx_sentence_scene ON sentence(level, scene);
-- idx_sentence_pack 由 migrate_pack_column 建:老库执行 SCHEMA 时 pack 列
-- 尚不存在,放这里会让整个 batch 失败。
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
        store.migrate_pack_column()?;
        store.set_meta("origin", origin)?;
        store.set_meta("rev", &rev.to_string())?;
        Ok(store)
    }

    /// Open an existing database read-only (the shipped content.db).
    ///
    /// 只读库不做迁移:出厂 content.db 由 `sf factory build` 重建,
    /// 天然带最新 schema。旧内容包缺列时 [`Self::has_pack_column`] 为 false,
    /// 查询层自动退化(见 [`Self::pack_filter`])。
    pub fn open_readonly(path: &Path) -> Result<Self> {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        Ok(Self { conn })
    }

    /// Open read-write without recreating meta (user_content.db at runtime).
    /// 老用户库缺 `pack` 列时就地做加列迁移(additive,行为完全兼容)。
    pub fn open_rw(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        let store = Self { conn };
        store.migrate_pack_column()?;
        Ok(store)
    }

    /// 表是否已有 `pack` 列(旧库/旧内容包可能没有)。
    fn has_pack_column(&self) -> bool {
        self.conn
            .prepare("SELECT pack FROM sentence LIMIT 1")
            .is_ok()
    }

    /// 幂等迁移:缺列则 `ALTER TABLE … ADD COLUMN`(SQLite 加列是 O(1)
    /// 元数据操作),随后建索引。新库走 SCHEMA 建表后只补索引。
    fn migrate_pack_column(&self) -> Result<()> {
        if !self.has_pack_column() {
            self.conn
                .execute_batch("ALTER TABLE sentence ADD COLUMN pack TEXT NOT NULL DEFAULT ''")?;
        }
        self.conn
            .execute_batch("CREATE INDEX IF NOT EXISTS idx_sentence_pack ON sentence(pack)")?;
        Ok(())
    }

    /// 等级取句路径的防污染条件:有 pack 列时排除场景句,没有则恒真
    /// (旧内容包不含场景句,条件无意义)。
    fn pack_filter(&self, prefix: &str) -> &'static str {
        if self.has_pack_column() {
            match prefix {
                "AND" => "AND pack = ''",
                _ => "WHERE pack = ''",
            }
        } else {
            match prefix {
                "AND" => "",
                _ => "",
            }
        }
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
        self.insert_sentence_in_pack(s, attribution, rev, "")
    }

    /// Insert into a scenario pack(`pack` 非空 = 场景练习句,不进等级队列)。
    pub fn insert_sentence_in_pack(
        &self,
        s: &Sentence,
        attribution: &str,
        rev: u32,
        pack: &str,
    ) -> Result<i64> {
        let words = serde_json::to_string(&s.words).map_err(|e| StoreError::Data(e.to_string()))?;
        let chunks =
            serde_json::to_string(&s.chunks).map_err(|e| StoreError::Data(e.to_string()))?;
        self.conn.execute(
            "INSERT INTO sentence
               (level, scene, func, pattern, zh, en, punct, words, chunks, note,
                simhash, license, attribution, rev, pack)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
                pack,
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

    /// 某等级的练习句(**不含场景练习句** —— 防污染,方案 §3.2)。
    pub fn sentences_by_level(&self, level: LevelId) -> Result<Vec<Sentence>> {
        let sql = format!(
            "SELECT * FROM sentence WHERE level = ?1 {} ORDER BY id",
            self.pack_filter("AND")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![level.as_str()], Self::row_to_sentence)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 一个场景包的全部句子,按对话顺序(写入顺序 = id 序)。
    pub fn sentences_by_pack(&self, pack: &str) -> Result<Vec<Sentence>> {
        if !self.has_pack_column() {
            return Ok(Vec::new());
        }
        let mut stmt = self
            .conn
            .prepare("SELECT * FROM sentence WHERE pack = ?1 ORDER BY id")?;
        let rows = stmt.query_map(params![pack], Self::row_to_sentence)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 库内全部场景包 id 及其句数(用户库据此即席构造「我的场景包」)。
    pub fn pack_counts(&self) -> Result<Vec<(String, u32)>> {
        if !self.has_pack_column() {
            return Ok(Vec::new());
        }
        let mut stmt = self.conn.prepare(
            "SELECT pack, COUNT(*) FROM sentence WHERE pack <> '' GROUP BY pack ORDER BY MIN(id)",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u32>(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 删除整个场景包(用户库)。
    pub fn delete_pack(&self, pack: &str) -> Result<u32> {
        if !self.has_pack_column() || pack.is_empty() {
            return Ok(0);
        }
        let n = self
            .conn
            .execute("DELETE FROM sentence WHERE pack = ?1", params![pack])?;
        Ok(n as u32)
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

    /// Distinct scenes per level (library grouping, §4.3);场景练习句不计入。
    pub fn scenes(&self, level: LevelId) -> Result<Vec<String>> {
        let sql = format!(
            "SELECT DISTINCT scene FROM sentence WHERE level = ?1 {} ORDER BY scene",
            self.pack_filter("AND")
        );
        let mut stmt = self.conn.prepare(&sql)?;
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

    /// 场景包内容:出厂包在前,用户同名包续在其后(id 已按库偏移)。
    pub fn sentences_by_pack(&self, pack: &str) -> Result<Vec<Sentence>> {
        let mut out = self.factory.sentences_by_pack(pack)?;
        if let Some(user) = &self.user {
            out.extend(user.sentences_by_pack(pack)?.into_iter().map(|mut s| {
                s.id += Self::USER_ID_OFFSET;
                s
            }));
        }
        Ok(out)
    }

    /// 两库的场景包句数统计:`(pack, count, from_user)`。
    pub fn pack_counts(&self) -> Result<Vec<(String, u32, bool)>> {
        let mut out: Vec<(String, u32, bool)> = self
            .factory
            .pack_counts()?
            .into_iter()
            .map(|(p, n)| (p, n, false))
            .collect();
        if let Some(user) = &self.user {
            out.extend(user.pack_counts()?.into_iter().map(|(p, n)| (p, n, true)));
        }
        Ok(out)
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
    fn scenario_pack_sentences_never_leak_into_level_queries() {
        let path = temp_db("packs");
        let store = ContentStore::create(&path, "factory", 1).unwrap();
        store
            .insert_sentence(&sample(LevelId::L1, "level sentence"), "", 1)
            .unwrap();
        store
            .insert_sentence_in_pack(&sample(LevelId::L1, "pack line 1"), "", 1, "cafe-order")
            .unwrap();
        store
            .insert_sentence_in_pack(&sample(LevelId::L1, "pack line 2"), "", 1, "cafe-order")
            .unwrap();

        // 等级路径完全看不到场景句(防污染红线)
        let level = store.sentences_by_level(LevelId::L1).unwrap();
        assert_eq!(level.len(), 1, "等级取句必须排除场景句");
        assert_eq!(level[0].en, "level sentence");

        // 场景路径按写入顺序返回整包
        let pack = store.sentences_by_pack("cafe-order").unwrap();
        assert_eq!(
            pack.iter().map(|s| s.en.as_str()).collect::<Vec<_>>(),
            vec!["pack line 1", "pack line 2"],
            "对话顺序 = 写入顺序"
        );
        assert_eq!(store.pack_counts().unwrap(), vec![("cafe-order".into(), 2)]);
        // 句总数仍包含场景句(内容包体量统计)
        assert_eq!(store.sentence_count().unwrap(), 3);

        assert_eq!(store.delete_pack("cafe-order").unwrap(), 2);
        assert!(store.sentences_by_pack("cafe-order").unwrap().is_empty());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn legacy_db_without_pack_column_migrates_on_open() {
        let path = temp_db("migrate");
        // 造一个"旧版"库:显式建不含 pack 列的表
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 CREATE TABLE sentence (
                    id INTEGER PRIMARY KEY, level TEXT NOT NULL, scene TEXT NOT NULL DEFAULT '',
                    func TEXT NOT NULL DEFAULT '', pattern TEXT NOT NULL DEFAULT '',
                    zh TEXT NOT NULL, en TEXT NOT NULL, punct TEXT NOT NULL DEFAULT '',
                    words TEXT NOT NULL, chunks TEXT NOT NULL, note TEXT NOT NULL DEFAULT '',
                    simhash INTEGER NOT NULL DEFAULT 0, license TEXT NOT NULL DEFAULT 'proprietary',
                    attribution TEXT NOT NULL DEFAULT '', rev INTEGER NOT NULL DEFAULT 1);
                 INSERT INTO sentence(level, zh, en, words, chunks)
                   VALUES ('L1', '旧句。', 'legacy row', '[]', '[]');",
            )
            .unwrap();
        }
        // 打开即迁移,老数据保留且默认 pack=''
        let store = ContentStore::open_rw(&path).unwrap();
        assert!(store.has_pack_column(), "打开后应已加列");
        let level = store.sentences_by_level(LevelId::L1).unwrap();
        assert_eq!(level.len(), 1);
        assert_eq!(level[0].en, "legacy row");
        // 迁移后可正常写场景句
        store
            .insert_sentence_in_pack(&sample(LevelId::L2, "new pack line"), "", 1, "hotel")
            .unwrap();
        assert_eq!(store.sentences_by_pack("hotel").unwrap().len(), 1);
        assert_eq!(store.sentences_by_level(LevelId::L2).unwrap().len(), 0);
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
