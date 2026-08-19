//! `sf` — vendor-side factory CLI (spec §8).
//!
//! ```text
//! sf factory build   --content-dir content --out content/build/content.db
//! sf factory validate --content-dir content
//! sf factory gen     --scene "机场值机" --level L3 --count 20 --channel ollama --model qwen3
//! sf gold run        --content-dir content
//! sf export trial    --content-dir content --out apps/web-trial/src/data/trial-content.json
//! ```

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use sf_core::sentence::{LevelId, Sentence};
use sf_core::spec::LevelSpec;
use sf_pipeline::lexicon::Lexicon;
use sf_pipeline::seed::SeedFile;
use sf_pipeline::store::ContentStore;
use sf_pipeline::triage::{GenProfile, TriageOutcome, triage};
use sf_pipeline::validate::{DedupeIndex, Validator, VerdictKind};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "sf", about = "SentenceFlow factory pipeline CLI (vendor side)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Factory content production.
    Factory {
        #[command(subcommand)]
        cmd: FactoryCmd,
    },
    /// Gold-set regression (§8 质量体系).
    Gold {
        #[command(subcommand)]
        cmd: GoldCmd,
    },
    /// Export derived artifacts.
    Export {
        #[command(subcommand)]
        cmd: ExportCmd,
    },
}

#[derive(Subcommand)]
enum FactoryCmd {
    /// Build content.db from seed files (validated through the pipeline).
    Build {
        #[arg(long, default_value = "content")]
        content_dir: PathBuf,
        #[arg(long, default_value = "content/build/content.db")]
        out: PathBuf,
        /// Content-pack revision stamped into meta.
        #[arg(long, default_value_t = 1)]
        rev: u32,
    },
    /// Validate all seed files without writing anything.
    Validate {
        #[arg(long, default_value = "content")]
        content_dir: PathBuf,
    },
    /// Generate sentences over an AI channel into a content database.
    Gen {
        #[arg(long)]
        scene: String,
        #[arg(long)]
        level: String,
        #[arg(long, default_value_t = 20)]
        count: u32,
        /// opencode | deepseek | zen | ollama
        #[arg(long)]
        channel: String,
        #[arg(long)]
        model: String,
        #[arg(long, default_value = "content")]
        content_dir: PathBuf,
        #[arg(long, default_value = "content/build/content.db")]
        db: PathBuf,
        /// API key for deepseek/zen (or env SF_API_KEY).
        #[arg(long)]
        api_key: Option<String>,
    },
}

#[derive(Subcommand)]
enum GoldCmd {
    /// Run the gold set (currently: all seed files) through the validator.
    Run {
        #[arg(long, default_value = "content")]
        content_dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum ExportCmd {
    /// Emit the web-trial content JSON from seed files (L1–L2 各一节, §7.9).
    Trial {
        #[arg(long, default_value = "content")]
        content_dir: PathBuf,
        #[arg(long, default_value = "apps/web-trial/src/data/trial-content.json")]
        out: PathBuf,
        #[arg(long, default_value = "L1,L2")]
        levels: String,
        #[arg(long, default_value_t = 20)]
        per_level: u32,
    },
    /// 把某个已生成的场景包(用户库)导出成出厂 YAML 素材,人工审校后
    /// 放进 content/scenario/(内容生产流水线,方案 §3.3)。
    Scenario {
        /// 用户库路径(桌面端:%APPDATA%/app.sentenceflow.desktop/user_content.db)
        #[arg(long)]
        db: PathBuf,
        /// 包 id(= 生成任务的场景名)
        #[arg(long)]
        pack: String,
        /// 输出文件
        #[arg(long)]
        out: PathBuf,
        /// 出厂包 id(kebab-case,如 cafe-order)
        #[arg(long)]
        id: String,
        #[arg(long)]
        category: String,
        #[arg(long, default_value = "")]
        intro: String,
        #[arg(long, default_value = "L3")]
        reference_level: String,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Factory { cmd } => match cmd {
            FactoryCmd::Build {
                content_dir,
                out,
                rev,
            } => build(&content_dir, &out, rev),
            FactoryCmd::Validate { content_dir } => {
                validate_seeds(&content_dir)?;
                run_placement(&content_dir)?;
                run_scenario_packs(&content_dir, None, 1).map(|_| ())
            }
            FactoryCmd::Gen {
                scene,
                level,
                count,
                channel,
                model,
                content_dir,
                db,
                api_key,
            } => gen_cmd(
                &scene,
                &level,
                count,
                &channel,
                &model,
                &content_dir,
                &db,
                api_key,
            ),
        },
        Cmd::Gold { cmd } => match cmd {
            GoldCmd::Run { content_dir } => gold_run(&content_dir),
        },
        Cmd::Export { cmd } => match cmd {
            ExportCmd::Trial {
                content_dir,
                out,
                levels,
                per_level,
            } => export_trial(&content_dir, &out, &levels, per_level),
            ExportCmd::Scenario {
                db,
                pack,
                out,
                id,
                category,
                intro,
                reference_level,
            } => export_scenario(&db, &pack, &out, &id, &category, &intro, &reference_level),
        },
    }
}

// ---------------------------------------------------------------- loading

fn load_specs(content_dir: &Path) -> Result<BTreeMap<LevelId, (LevelSpec, String)>> {
    let dir = content_dir.join("specs");
    let mut specs = BTreeMap::new();
    for entry in std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        let yaml = std::fs::read_to_string(&path)?;
        let spec =
            LevelSpec::from_yaml(&yaml).with_context(|| format!("parsing {}", path.display()))?;
        specs.insert(spec.id, (spec, yaml));
    }
    if specs.is_empty() {
        bail!("no level specs found in {}", dir.display());
    }
    Ok(specs)
}

fn load_lexicon(content_dir: &Path) -> Result<Lexicon> {
    let path = content_dir.join("lexicon").join("base.tsv");
    let tsv =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Lexicon::from_tsv(&tsv).map_err(|e| anyhow::anyhow!(e))
}

fn seed_files(content_dir: &Path) -> Result<Vec<PathBuf>> {
    let dir = content_dir.join("seed");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .collect();
    files.sort();
    Ok(files)
}

// ---------------------------------------------------------------- validate

struct SeedRun {
    accepted: Vec<Sentence>,
    problems: Vec<String>,
}

/// Validate all seeds; returns accepted sentences and human-readable problems.
fn run_seeds(content_dir: &Path) -> Result<SeedRun> {
    let specs = load_specs(content_dir)?;
    let lexicon = load_lexicon(content_dir)?;
    let mut dedupe = DedupeIndex::default();
    let mut accepted = Vec::new();
    let mut problems = Vec::new();

    for file in seed_files(content_dir)? {
        let yaml = std::fs::read_to_string(&file)?;
        let seed =
            SeedFile::from_yaml(&yaml).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        let (spec, _) = specs
            .get(&seed.level)
            .with_context(|| format!("{}: no spec for {}", file.display(), seed.level))?;
        let validator = Validator::new(spec, &lexicon);
        for (idx, s) in seed.sentences.iter().enumerate() {
            let report = validator.validate(&s.to_draft(), &s.scene, &s.func, &dedupe);
            match report.verdict {
                VerdictKind::Pass => {
                    dedupe.add(report.simhash);
                    let mut sentence = report.sentence.expect("Pass carries a sentence");
                    sentence.note = s.note.clone();
                    accepted.push(sentence);
                }
                _ => {
                    let reasons: Vec<String> = report
                        .issues
                        .iter()
                        .filter(|i| i.severity() != sf_pipeline::validate::Severity::AutoFixed)
                        .map(|i| i.zh_reason())
                        .collect();
                    problems.push(format!(
                        "{}#{} [{}] {:?}: {}",
                        file.file_name().and_then(|n| n.to_str()).unwrap_or("?"),
                        idx + 1,
                        s.en,
                        report.verdict,
                        reasons.join("；")
                    ));
                }
            }
        }
    }
    Ok(SeedRun { accepted, problems })
}

fn validate_seeds(content_dir: &Path) -> Result<SeedRun> {
    let run = run_seeds(content_dir)?;
    println!("seed sentences accepted: {}", run.accepted.len());
    if !run.problems.is_empty() {
        println!("problems ({}):", run.problems.len());
        for p in &run.problems {
            println!("  ✕ {p}");
        }
        bail!("{} seed sentence(s) failed validation", run.problems.len());
    }
    Ok(run)
}

// ------------------------------------------------------- scenario export

/// 把用户库里的一个场景包导出为出厂 YAML(内容生产:工坊生成 → 导出 →
/// 人工审校 → 放进 content/scenario/ → factory build 强校验)。
fn export_scenario(
    db: &Path,
    pack: &str,
    out: &Path,
    id: &str,
    category: &str,
    intro: &str,
    reference_level: &str,
) -> Result<()> {
    let store = ContentStore::open_readonly(db)
        .map_err(|e| anyhow::anyhow!("opening {}: {e}", db.display()))?;
    let lines = store
        .sentences_by_pack(pack)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    if lines.is_empty() {
        bail!("pack「{pack}」not found in {}", db.display());
    }

    let mut yaml = String::new();
    yaml.push_str(&format!(
        "# 场景练习包 — {pack}(由 `sf export scenario` 从生成结果导出,已人工审校)\n\
         pack: {id}\nname: \"{pack}\"\ncategory: \"{category}\"\n"
    ));
    if !intro.is_empty() {
        yaml.push_str(&format!("intro: \"{intro}\"\n"));
    }
    yaml.push_str(&format!("reference_level: {reference_level}\ndialogue:\n"));

    for (i, s) in lines.iter().enumerate() {
        // speaker 存在 func 列;缺失时按顺序 A/B 交替兜底
        let speaker = if s.func == "A" || s.func == "B" {
            s.func.clone()
        } else if i % 2 == 0 {
            "A".into()
        } else {
            "B".into()
        };
        let en = format!("{}{}", s.en.trim_end_matches(&s.punct), s.punct);
        yaml.push_str(&format!(
            "  - speaker: {speaker}\n    en: \"{}\"\n    zh: \"{}\"\n",
            en.replace('"', "\\\""),
            s.zh.replace('"', "\\\"")
        ));
        if !s.pattern.is_empty() {
            yaml.push_str(&format!("    pattern: \"{}\"\n", s.pattern));
        }
        if !s.note.is_empty() {
            yaml.push_str(&format!("    note: \"{}\"\n", s.note.replace('"', "\\\"")));
        }
        yaml.push_str("    words:\n");
        for w in &s.words {
            yaml.push_str(&format!(
                "      - {{ w: \"{}\", ipa: \"{}\", pos: \"{}\" }}\n",
                w.w,
                w.ipa,
                serde_json::to_value(w.pos)?.as_str().unwrap_or("n")
            ));
        }
        yaml.push_str("    chunks:\n");
        for c in &s.chunks {
            let idx: Vec<String> = c.i.iter().map(|n| n.to_string()).collect();
            yaml.push_str(&format!(
                "      - {{ r: \"{}\", i: [{}] }}\n",
                serde_json::to_value(c.r)?.as_str().unwrap_or("advl"),
                idx.join(", ")
            ));
        }
        yaml.push('\n');
    }

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(out, yaml)?;
    println!(
        "scenario pack exported: {} ({} lines) — 请人工审校后再 factory build",
        out.display(),
        lines.len()
    );
    Ok(())
}

// ---------------------------------------------------------------- scenario

/// 出厂场景包文件(`content/scenario/*.yaml`,方案 §3.3)。
#[derive(serde::Deserialize)]
struct ScenarioFile {
    pack: String,
    name: String,
    category: String,
    #[serde(default)]
    intro: String,
    #[serde(default)]
    reference_level: Option<LevelId>,
    dialogue: Vec<sf_pipeline::seed::SeedSentence>,
}

/// 写入 content.db `meta["scenario_packs"]` 的包元信息。
#[derive(serde::Serialize)]
struct ScenarioPackMeta {
    pack: String,
    name: String,
    category: String,
    intro: String,
    reference_level: Option<LevelId>,
}

/// 每个出厂场景包的最小对话轮数(太短不成对话)。
const SCENARIO_MIN_TURNS: usize = 6;

/// 校验并写入全部出厂场景包;返回包元信息(空目录 → 空表)。
///
/// 与等级种子的差别只有一处:**校验用放开词表带的规格**(场景对话
/// 不分等级,方案 §1/§3.3),结构/成分/音标/查重照旧强校验。
fn run_scenario_packs(
    content_dir: &Path,
    store: Option<&ContentStore>,
    rev: u32,
) -> Result<Vec<ScenarioPackMeta>> {
    let dir = content_dir.join("scenario");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let specs = load_specs(content_dir)?;
    let lexicon = load_lexicon(content_dir)?;
    // 校验规格以 L6 为底(句长上限 20),词表判定整体关闭:场景对话的
    // 词汇取材于真实生活(latte/checkout…),不受 NGSL 词表约束。
    let open_spec = {
        let (l6, _) = specs
            .get(&LevelId::L6)
            .context("scenario packs need the L6 spec as validation base")?;
        l6.clone()
    };
    let validator = Validator::new_open_vocabulary(&open_spec, &lexicon);

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("yaml"))
        .collect();
    files.sort();

    let mut problems: Vec<String> = Vec::new();
    let mut metas: Vec<ScenarioPackMeta> = Vec::new();
    let mut seen_packs: BTreeMap<String, String> = BTreeMap::new();

    for file in files {
        let yaml = std::fs::read_to_string(&file)?;
        let sf: ScenarioFile =
            serde_yaml::from_str(&yaml).map_err(|e| anyhow::anyhow!("{}: {e}", file.display()))?;
        let fname = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        if let Some(prev) = seen_packs.insert(sf.pack.clone(), fname.clone()) {
            problems.push(format!("{fname}: pack id「{}」与 {prev} 重复", sf.pack));
        }
        if sf.dialogue.len() < SCENARIO_MIN_TURNS {
            problems.push(format!(
                "{fname}: 只有 {} 轮,少于 {SCENARIO_MIN_TURNS} 轮下限",
                sf.dialogue.len()
            ));
        }
        // 包内查重(跨包/跨库允许重复:不同场景常有相同短句)
        let mut dedupe = DedupeIndex::default();
        for (idx, line) in sf.dialogue.iter().enumerate() {
            let speaker = line.speaker.trim().to_uppercase();
            if speaker != "A" && speaker != "B" {
                problems.push(format!("{fname}#{}: speaker 必须是 A 或 B", idx + 1));
                continue;
            }
            let report = validator.validate(&line.to_draft(), &sf.name, &speaker, &dedupe);
            match report.verdict {
                VerdictKind::Pass => {
                    dedupe.add(report.simhash);
                    if let Some(store) = store {
                        let mut s = report.sentence.expect("Pass carries a sentence");
                        s.note = line.note.clone();
                        s.level = sf.reference_level.unwrap_or(LevelId::L3);
                        store
                            .insert_sentence_in_pack(&s, "", rev, &sf.pack)
                            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    }
                }
                verdict => {
                    let reasons: Vec<String> = report
                        .issues
                        .iter()
                        .filter(|i| i.severity() != sf_pipeline::validate::Severity::AutoFixed)
                        .map(|i| i.zh_reason())
                        .collect();
                    problems.push(format!(
                        "{fname}#{} [{}] {verdict:?}: {}",
                        idx + 1,
                        line.en,
                        reasons.join("；")
                    ));
                }
            }
        }
        metas.push(ScenarioPackMeta {
            pack: sf.pack,
            name: sf.name,
            category: sf.category,
            intro: sf.intro,
            reference_level: sf.reference_level,
        });
    }

    if !problems.is_empty() {
        println!("scenario problems ({}):", problems.len());
        for p in &problems {
            println!("  ✕ {p}");
        }
        bail!("{} scenario line(s) failed validation", problems.len());
    }
    Ok(metas)
}

// ---------------------------------------------------------------- placement

/// 定级题库文件(content/placement/placement.yaml,《定级测试实现方案》§3.2)。
#[derive(serde::Deserialize)]
struct PlacementFile {
    version: u32,
    vocab: PlacementVocab,
    sentences: Vec<PlacementSeed>,
    grammar: Vec<PlacementGrammar>,
}

#[derive(serde::Deserialize)]
struct PlacementVocab {
    strata: Vec<u32>,
    per_stratum: u32,
    pseudo_count: u32,
    pseudowords: Vec<String>,
}

#[derive(serde::Deserialize)]
struct PlacementSeed {
    level: LevelId,
    #[serde(flatten)]
    seed: sf_pipeline::seed::SeedSentence,
}

#[derive(serde::Deserialize)]
struct PlacementGrammar {
    lo: LevelId,
    hi: LevelId,
    topic_zh: String,
    prompt_zh: String,
    stem: String,
    options: Vec<String>,
    answer: u8,
}

/// 每级最少定级句数(阶梯自适应要有足够抽题余量)。
const PLACEMENT_MIN_PER_LEVEL: usize = 6;
/// 每个相邻边界最少语法题数(与 sf-core 的加测题数对齐)。
const PLACEMENT_MIN_GRAMMAR: usize = 4;

/// 校验定级题库并装配为 [`sf_core::PlacementBank`](sf_core::placement::PlacementBank)
/// (`vocab_pool` 留空,客户端运行时从 lemma 表装配)。题库文件缺失时返回
/// `None`(向后兼容);校验不过则构建失败——题目质量红线在构建期把守。
fn run_placement(content_dir: &Path) -> Result<Option<sf_core::PlacementBank>> {
    let path = content_dir.join("placement").join("placement.yaml");
    if !path.exists() {
        return Ok(None);
    }
    let file: PlacementFile = serde_yaml::from_str(&std::fs::read_to_string(&path)?)
        .with_context(|| format!("parsing {}", path.display()))?;
    let specs = load_specs(content_dir)?;
    let lexicon = load_lexicon(content_dir)?;
    let mut problems: Vec<String> = Vec::new();

    // 词汇配置红线
    if file.vocab.strata.is_empty() || file.vocab.strata.windows(2).any(|w| w[0] >= w[1]) {
        problems.push("vocab.strata 必须非空且严格升序".into());
    }
    if file.vocab.per_stratum == 0 {
        problems.push("vocab.per_stratum 必须 > 0".into());
    }
    if file.vocab.pseudowords.len() < file.vocab.pseudo_count as usize {
        problems.push("伪词数量少于 pseudo_count".into());
    }
    for p in &file.vocab.pseudowords {
        if lexicon.lookup(p).is_some() {
            problems.push(format!("伪词「{p}」是词表内真词,必须更换"));
        }
        if !(3..=9).contains(&p.len()) || !p.chars().all(|c| c.is_ascii_lowercase()) {
            problems.push(format!("伪词「{p}」需为 3–9 个小写字母"));
        }
    }

    // 定级句:按各自等级的 spec 走完整校验管线;题库内部互不重复
    // (独立 dedupe——定级句先于练习内容出现,与出厂句库重叠可接受)。
    let mut dedupe = DedupeIndex::default();
    let mut sentences: Vec<Sentence> = Vec::new();
    let mut per_level: BTreeMap<LevelId, usize> = BTreeMap::new();
    for (idx, ps) in file.sentences.iter().enumerate() {
        let Some((spec, _)) = specs.get(&ps.level) else {
            problems.push(format!("placement#{}: 无 {} 级 spec", idx + 1, ps.level));
            continue;
        };
        let validator = Validator::new(spec, &lexicon);
        let report = validator.validate(&ps.seed.to_draft(), &ps.seed.scene, "", &dedupe);
        match report.verdict {
            VerdictKind::Pass => {
                dedupe.add(report.simhash);
                let mut s = report.sentence.expect("Pass carries a sentence");
                s.id = idx as i64 + 1;
                s.note = ps.seed.note.clone();
                *per_level.entry(ps.level).or_default() += 1;
                sentences.push(s);
            }
            verdict => {
                let reasons: Vec<String> = report
                    .issues
                    .iter()
                    .filter(|i| i.severity() != sf_pipeline::validate::Severity::AutoFixed)
                    .map(|i| i.zh_reason())
                    .collect();
                problems.push(format!(
                    "placement#{} [{}] {verdict:?}: {}",
                    idx + 1,
                    ps.seed.en,
                    reasons.join("；")
                ));
            }
        }
    }
    for level in LevelId::ALL {
        let n = per_level.get(&level).copied().unwrap_or(0);
        if n < PLACEMENT_MIN_PER_LEVEL {
            problems.push(format!(
                "{level} 级定级句仅 {n} 句,少于 {PLACEMENT_MIN_PER_LEVEL} 句下限"
            ));
        }
    }

    // 语法题:二选一、答案有效、边界相邻、每个边界题量达标
    for (i, g) in file.grammar.iter().enumerate() {
        if g.options.len() != 2 {
            problems.push(format!("grammar#{}: 必须二选一", i + 1));
        }
        if (g.answer as usize) >= g.options.len() {
            problems.push(format!("grammar#{}: answer 越界", i + 1));
        }
        if g.hi as i32 - g.lo as i32 != 1 {
            problems.push(format!(
                "grammar#{}: 边界必须相邻({}-{})",
                i + 1,
                g.lo,
                g.hi
            ));
        }
        if !g.stem.contains("___") {
            problems.push(format!("grammar#{}: 挖空句缺少 ___", i + 1));
        }
    }
    for w in LevelId::ALL.windows(2) {
        let n = file
            .grammar
            .iter()
            .filter(|g| g.lo == w[0] && g.hi == w[1])
            .count();
        if n < PLACEMENT_MIN_GRAMMAR {
            problems.push(format!(
                "边界 {}-{} 语法题仅 {n} 题,少于 {PLACEMENT_MIN_GRAMMAR} 题下限",
                w[0], w[1]
            ));
        }
    }

    if !problems.is_empty() {
        println!("placement problems ({}):", problems.len());
        for p in &problems {
            println!("  ✕ {p}");
        }
        bail!("{} placement item(s) failed validation", problems.len());
    }

    Ok(Some(sf_core::PlacementBank {
        version: file.version,
        strata: file.vocab.strata,
        per_stratum: file.vocab.per_stratum,
        pseudo_count: file.vocab.pseudo_count,
        pseudowords: file.vocab.pseudowords,
        vocab_pool: Vec::new(),
        sentences,
        grammar: file
            .grammar
            .into_iter()
            .map(|g| sf_core::GrammarItem {
                lo: g.lo,
                hi: g.hi,
                topic_zh: g.topic_zh,
                prompt_zh: g.prompt_zh,
                stem: g.stem,
                options: g.options,
                answer: g.answer,
            })
            .collect(),
    }))
}

// ---------------------------------------------------------------- build

fn build(content_dir: &Path, out: &Path, rev: u32) -> Result<()> {
    let run = validate_seeds(content_dir)?;
    let specs = load_specs(content_dir)?;

    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let _ = std::fs::remove_file(out);
    let store = ContentStore::create(out, "factory", rev)
        .map_err(|e| anyhow::anyhow!("creating {}: {e}", out.display()))?;

    // Spec snapshot: 内容与行为同版 (§7.7).
    let mut spec_concat = String::new();
    for (spec, yaml) in specs.values() {
        store
            .insert_level_spec(spec, yaml)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        spec_concat.push_str(yaml);
    }
    store
        .set_meta(
            "spec_hash",
            &format!("{:016x}", fnv1a64(spec_concat.as_bytes())),
        )
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // Lemma table doubles as the client-side dictionary (§7.7).
    let lex_path = content_dir.join("lexicon").join("base.tsv");
    for line in std::fs::read_to_string(&lex_path)?.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        store
            .insert_lemma(
                cols[0],
                cols[1].parse().unwrap_or(0),
                cols.get(2).unwrap_or(&""),
                cols.get(3).unwrap_or(&""),
                cols.get(4).unwrap_or(&""),
            )
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }

    for s in &run.accepted {
        store
            .insert_sentence(s, "", rev)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }

    // 出厂场景包(《场景练习模块-实现方案》§3.3):句子按对话顺序入
    // sentence 表(带 pack),包元信息进 meta["scenario_packs"]。
    let packs = run_scenario_packs(content_dir, Some(&store), rev)?;
    if !packs.is_empty() {
        store
            .set_meta("scenario_packs", &serde_json::to_string(&packs)?)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        println!("scenario packs embedded: {}", packs.len());
    }

    // 定级题库(方案 §3.2):校验后整体打进 meta["placement"]。
    if let Some(bank) = run_placement(content_dir)? {
        store
            .set_meta("placement", &serde_json::to_string(&bank)?)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        store
            .set_meta("placement_rev", &bank.version.to_string())
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        println!(
            "placement bank embedded: {} sentences, {} grammar items, {} pseudowords",
            bank.sentences.len(),
            bank.grammar.len(),
            bank.pseudowords.len()
        );
    }

    println!(
        "content.db written: {} ({} sentences, rev {rev})",
        out.display(),
        run.accepted.len()
    );
    Ok(())
}

// ---------------------------------------------------------------- gold

fn gold_run(content_dir: &Path) -> Result<()> {
    // Gold = seeds for now; the harness (pass-rate report + non-zero exit on
    // regression) is what W1's 出口标准 needs.
    let run = run_seeds(content_dir)?;
    let total = run.accepted.len() + run.problems.len();
    let rate = if total > 0 {
        run.accepted.len() as f64 / total as f64 * 100.0
    } else {
        0.0
    };
    println!(
        "gold regression: {}/{} pass ({rate:.1}%)",
        run.accepted.len(),
        total
    );
    for p in &run.problems {
        println!("  ✕ {p}");
    }
    if !run.problems.is_empty() {
        bail!("gold regression failed");
    }
    Ok(())
}

// ---------------------------------------------------------------- export

fn export_trial(content_dir: &Path, out: &Path, levels: &str, per_level: u32) -> Result<()> {
    let run = validate_seeds(content_dir)?;
    let wanted: Vec<LevelId> = levels
        .split(',')
        .map(|s| s.trim().parse::<LevelId>().map_err(|e| anyhow::anyhow!(e)))
        .collect::<Result<_>>()?;

    #[derive(serde::Serialize)]
    struct Section {
        level: LevelId,
        title: String,
        /// 该级 LevelSpec — 试用版练习行为的数据源(§4.9 三端共用).
        spec: LevelSpec,
        sentences: Vec<Sentence>,
    }
    #[derive(serde::Serialize)]
    struct TrialContent {
        sections: Vec<Section>,
    }

    let specs = load_specs(content_dir)?;
    let mut sections = Vec::new();
    for level in wanted {
        let mut sentences: Vec<Sentence> = run
            .accepted
            .iter()
            .filter(|s| s.level == level)
            .take(per_level as usize)
            .cloned()
            .collect();
        // Trial ids are positional; keep them stable and non-zero.
        for (i, s) in sentences.iter_mut().enumerate() {
            s.id = i as i64 + 1;
        }
        if sentences.is_empty() {
            bail!("no seed sentences for {level}");
        }
        let (spec, _) = specs.get(&level).context("missing spec")?;
        sections.push(Section {
            level,
            title: format!("{level} 体验节"),
            spec: spec.clone(),
            sentences,
        });
    }
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        out,
        serde_json::to_string_pretty(&TrialContent { sections })?,
    )?;
    println!("trial content written: {}", out.display());
    Ok(())
}

// ---------------------------------------------------------------- gen

#[allow(clippy::too_many_arguments)]
fn gen_cmd(
    scene: &str,
    level: &str,
    count: u32,
    channel: &str,
    model: &str,
    content_dir: &Path,
    db: &Path,
    api_key: Option<String>,
) -> Result<()> {
    use futures::StreamExt;
    use sf_llm::ChannelAdapter;
    use sf_llm::channels::{DeepseekChannel, OllamaChannel, OpencodeChannel, ZenChannel};
    use sf_llm::meter::PriceTable;
    use sf_llm::types::GenChunk;
    use sf_pipeline::parse::StreamScanner;
    use sf_pipeline::prompt::build_prompt;

    let level: LevelId = level.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let specs = load_specs(content_dir)?;
    let (spec, _) = specs.get(&level).context("no spec for level")?;
    let all_specs: Vec<LevelSpec> = specs.values().map(|(s, _)| s.clone()).collect();
    let lexicon = load_lexicon(content_dir)?;

    let store = if db.exists() {
        ContentStore::open_rw(db).map_err(|e| anyhow::anyhow!(e.to_string()))?
    } else {
        if let Some(parent) = db.parent() {
            std::fs::create_dir_all(parent)?;
        }
        ContentStore::create(db, "factory", 1).map_err(|e| anyhow::anyhow!(e.to_string()))?
    };
    let mut dedupe = DedupeIndex::new(
        store
            .all_simhashes()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?,
    );

    let key = api_key.or_else(|| std::env::var("SF_API_KEY").ok());
    // CLI 的 AI 代理经 SF_PROXY 环境变量(桌面端对应设置项 proxy_url)。
    let proxy = std::env::var("SF_PROXY")
        .ok()
        .filter(|p| !p.trim().is_empty());
    let adapter: Box<dyn ChannelAdapter> = match channel {
        "opencode" => Box::new(OpencodeChannel::new(
            sf_llm::channels::opencode::OpencodeConfig {
                bin_override: None,
                sandbox_dir: std::env::temp_dir().join("sf-agent-sandbox"),
                known_bad_versions: vec![],
                rpm_estimate: 10,
                proxy_url: proxy,
            },
        )),
        "deepseek" => Box::new(DeepseekChannel::new(
            key.context("--api-key or SF_API_KEY required for deepseek")?
                .into(),
            PriceTable {
                prompt_per_m: 2.0,
                completion_per_m: 8.0,
            },
            proxy,
        )),
        "zen" => Box::new(ZenChannel::new(
            key.context("--api-key or SF_API_KEY required for zen")?
                .into(),
            10,
            proxy,
        )),
        "ollama" => Box::new(OllamaChannel::default()),
        other => bail!("unknown channel: {other}"),
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let avoid: Vec<u64> = dedupe.recent(16).collect();
        let parts = build_prompt(spec, scene, count, &avoid, &[]);
        let req = sf_llm::types::GenRequest {
            model: model.to_string(),
            system: parts.system,
            user: parts.user,
            max_tokens: Some(8192),
            temperature: Some(0.7),
        };
        let mut stream = adapter
            .complete_stream(req)
            .await
            .map_err(|e| anyhow::anyhow!("channel error: {e} ({})", e.zh_message()))?;
        let mut scanner = StreamScanner::new();
        let validator = Validator::new(spec, &lexicon);
        let mut accepted = 0u32;
        let mut discarded = 0u32;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(GenChunk::Text { text }) => {
                    for draft in scanner.push(&text) {
                        match draft {
                            Ok(d) => {
                                let report = validator.validate(&d, scene, "", &dedupe);
                                let hash = report.simhash;
                                match triage(report, GenProfile::Factory, &all_specs) {
                                    TriageOutcome::Accept { sentence }
                                    | TriageOutcome::Relevel { sentence, .. } => {
                                        dedupe.add(hash);
                                        store
                                            .insert_sentence(&sentence, "", 1)
                                            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                                        accepted += 1;
                                        println!("  ✓ {}", sentence.en);
                                    }
                                    TriageOutcome::Repair { sentence, issues } => {
                                        // Factory-run repair loop is W6 work;
                                        // for now bank the sentence with its
                                        // issues logged.
                                        println!(
                                            "  ⟳ {} (needs repair: {} issues)",
                                            sentence.en,
                                            issues.len()
                                        );
                                        discarded += 1;
                                    }
                                    TriageOutcome::Discard { reason, .. } => {
                                        println!("  ✕ discarded: {reason}");
                                        discarded += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                println!("  ✕ broken JSON element: {e}");
                                discarded += 1;
                            }
                        }
                    }
                }
                Ok(GenChunk::Usage {
                    prompt_tokens,
                    completion_tokens,
                }) => {
                    println!("usage: {prompt_tokens} in / {completion_tokens} out");
                }
                Ok(GenChunk::Done) => break,
                Ok(_) => {}
                Err(e) => bail!("stream error: {e} ({})", e.zh_message()),
            }
        }
        println!("accepted {accepted} · discarded {discarded}");
        Ok(())
    })
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}
