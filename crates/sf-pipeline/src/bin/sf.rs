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
    /// 按 `content/scenes.yaml` 跑**整级生产**:该级的每个场景各跑若干批,
    /// 写进内容库。
    ///
    /// 循环放在工具内部而不是外面套 shell 脚本 —— 中文场景名经 shell 传参会
    /// 被按控制台编码(Windows 上是 GBK)转一道,存进库里就是乱码
    /// (踩过:`数字与年龄` 存成 `Êý×ÖÓëÄêÁä\r`,345 句全归档到乱码场景名下)。
    /// 工具自己读 YAML、自己调用,中文一次都不出进程。
    Run {
        #[arg(long)]
        level: String,
        /// 每个场景跑几批
        #[arg(long, default_value_t = 2)]
        batches: u32,
        /// 每批向模型要几句
        #[arg(long, default_value_t = 20)]
        count: u32,
        /// 只跑这个类别
        #[arg(long)]
        category: Option<String>,
        #[arg(long, default_value = "opencode")]
        channel: String,
        #[arg(long, default_value = "opencode/hy3-free")]
        model: String,
        #[arg(long, default_value = "content")]
        content_dir: PathBuf,
        /// AI 产出落在这里,跨多次跑批累积;`sf factory build` 再把它合进
        /// content.db。**不要直接写 content.db** —— build 会删库重建。
        #[arg(long, default_value = "content/build/generated.db")]
        db: PathBuf,
        #[arg(long)]
        api_key: Option<String>,
        #[arg(long, default_value = "content/scenes.yaml")]
        scenes: PathBuf,
    },
    /// 生产前的场景词汇预扫:按 `content/scenes.yaml` 逐个场景试产一小批,
    /// 汇总每个场景缺哪些词。
    ///
    /// 为什么要有这一步:词表带是按**通用语料词频**划的,而场景要的词未必
    /// 高频 —— 实测「看医生」场景 fever/headache/sore 全不在 NGSL 内,
    /// cough(2447)/throat(2327)/stomach(2673)在 L4 判越级,一批就掉三成。
    /// 5000 句跑到一半才发现这个,已经浪费掉大量额度。先扫一遍、把缺的词
    /// 一次性补进 supplement.tsv,正式跑批才不会边跑边掉。
    ///
    /// 输出一张 Markdown 表 + 一段可直接粘进 supplement.tsv 的候选词条。
    Scan {
        #[arg(long, default_value = "content/scenes.yaml")]
        scenes: PathBuf,
        /// 每个场景试产几句(够暴露词汇即可,不必是正式批量)
        #[arg(long, default_value_t = 8)]
        count: u32,
        /// 只扫这个类别(如 医疗);缺省扫全部
        #[arg(long)]
        category: Option<String>,
        /// 只扫前 N 个场景(先小样试跑用)
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long, default_value = "opencode")]
        channel: String,
        #[arg(long, default_value = "opencode/hy3-free")]
        model: String,
        /// 报告写到这个文件(缺省只打到标准输出)
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long, default_value = "content")]
        content_dir: PathBuf,
        #[arg(long)]
        api_key: Option<String>,
    },
    /// 入库率探针:跑一批(或回放已存的产出),报出各类拒因的占比。
    ///
    /// 内容生产前先用它摸底 —— 通过率低到底是"模型不行"还是"词表缺词/
    /// 例句越级"这类自己人的问题,直方图一眼看得出。`--save` 存下原始产出,
    /// 之后改校验器可以用 `--replay` 零成本复测同一批。
    Yield {
        #[arg(long)]
        level: String,
        /// 现场生成模式:场景描述
        #[arg(long)]
        scene: Option<String>,
        #[arg(long, default_value_t = 20)]
        count: u32,
        #[arg(long, default_value = "opencode")]
        channel: String,
        #[arg(long, default_value = "opencode/hy3-free")]
        model: String,
        /// 把本次原始产出存到这个文件(供以后 --replay)
        #[arg(long)]
        save: Option<PathBuf>,
        /// 回放模式:读这些文件里的原始产出,不调模型(可多次给出)
        #[arg(long)]
        replay: Vec<PathBuf>,
        #[arg(long, default_value = "content")]
        content_dir: PathBuf,
        #[arg(long)]
        api_key: Option<String>,
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
        /// 见 `run` 的同名参数:AI 产出落 generated.db,不直接写 content.db。
        #[arg(long, default_value = "content/build/generated.db")]
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
            FactoryCmd::Run {
                level,
                batches,
                count,
                category,
                channel,
                model,
                content_dir,
                db,
                api_key,
                scenes,
            } => run_level(
                &level,
                batches,
                count,
                category.as_deref(),
                &channel,
                &model,
                &content_dir,
                &db,
                api_key,
                &scenes,
            ),
            FactoryCmd::Scan {
                scenes,
                count,
                category,
                limit,
                channel,
                model,
                out,
                content_dir,
                api_key,
            } => scan_cmd(
                &scenes,
                count,
                category.as_deref(),
                limit,
                &channel,
                &model,
                out.as_deref(),
                &content_dir,
                api_key,
            ),
            FactoryCmd::Yield {
                level,
                scene,
                count,
                channel,
                model,
                save,
                replay,
                content_dir,
                api_key,
            } => yield_cmd(
                &level,
                scene.as_deref(),
                count,
                &channel,
                &model,
                save.as_deref(),
                &replay,
                &content_dir,
                api_key,
            ),
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

/// 词表源文件,按加载顺序:`base.tsv`(NGSL)在前,`supplement.tsv`
/// (教学补充,可缺)在后 —— 同名词条以后者为准。
pub const LEXICON_FILES: [&str; 2] = ["base.tsv", "supplement.tsv"];

/// 读齐所有词表源文件,拼成一份 TSV(缺失的补充表不算错)。
fn lexicon_tsv(content_dir: &Path) -> Result<String> {
    let dir = content_dir.join("lexicon");
    let mut tsv = String::new();
    for (i, name) in LEXICON_FILES.iter().enumerate() {
        let path = dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                tsv.push_str(&text);
                tsv.push('\n');
            }
            // base.tsv 必须在;supplement.tsv 可选。
            Err(e) if i > 0 && e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }
    Ok(tsv)
}

/// 教学定级覆盖表(可缺):`lemma \t band \t 理由`,只改 band。
const LEXICON_OVERRIDES: &str = "overrides.tsv";

fn load_lexicon(content_dir: &Path) -> Result<Lexicon> {
    let mut lex = Lexicon::from_tsv(&lexicon_tsv(content_dir)?).map_err(|e| anyhow::anyhow!(e))?;
    let path = content_dir.join("lexicon").join(LEXICON_OVERRIDES);
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            lex.apply_band_overrides(&text)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    }
    Ok(lex)
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
                    let mut sentence = report.sentence.expect("Pass carries a sentence");
                    dedupe.add(sentence.en.as_str());
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
                    let mut s = report.sentence.expect("Pass carries a sentence");
                    dedupe.add(s.en.as_str());
                    if let Some(store) = store {
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
                let mut s = report.sentence.expect("Pass carries a sentence");
                dedupe.add(s.en.as_str());
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
    // 桌面端只读 content.db 的 lemma 表,所以补充词表必须在这里一并写进去,
    // 否则 CLI 校验通过的句子到了应用里会因"词表外"被拒。
    //
    // 词表与句子一起包进一个事务:逐行自动提交时整个 build 要近 6 分钟。
    // 从**解析好的** Lexicon 出,而不是重新读 TSV —— 教学定级覆盖
    // (overrides.tsv)是在 Lexicon 上应用的,照抄原始 TSV 会把覆盖丢掉,
    // 于是 CLI 校验和桌面端校验用的是两套 band。
    let lexicon = load_lexicon(content_dir)?;
    store
        .in_transaction(|store| {
            for e in lexicon.entries() {
                store.insert_lemma(
                    &e.lemma,
                    e.band,
                    e.teach_band,
                    &e.ipa_gb,
                    &e.ipa_us,
                    &e.zh_gloss,
                )?;
            }
            for s in &run.accepted {
                store.insert_sentence(s, "", rev)?;
            }
            Ok(())
        })
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // 合并 AI 生成库(可缺)。build 会删库重建,所以生成内容必须另存 ——
    // 否则每次改词表后重建,攒下来的句子就全没了。
    let generated = out.with_file_name("generated.db");
    if generated.exists() {
        let gen_store = ContentStore::open_rw(&generated)
            .map_err(|e| anyhow::anyhow!("opening {}: {e}", generated.display()))?;
        let rows = gen_store
            .all_sentences()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let mut merged = 0usize;
        store
            .in_transaction(|store| {
                for s in &rows {
                    // 种子优先:同文的以种子为准(种子是人工校对过的)
                    if store.sentence_id_by_en(&s.en)?.is_none() {
                        store.insert_sentence(s, "", rev)?;
                        merged += 1;
                    }
                }
                Ok(())
            })
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        println!("merged generated.db: {merged}/{} sentences", rows.len());
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

/// 词表分层自检:`supplement.tsv` 不得重定义 `base.tsv` 已有的词。
///
/// 补充表本意是"补 NGSL 没有的词"。一旦它悄悄给已有词换了 band,等级归属
/// 就会莫名其妙地漂移 —— 实测踩过:补充表把 `noodle` 从 1500 改成 1550,
/// 种子句 "We ordered two bowls of noodles." 当场在 L3 判越级。
fn check_lexicon_layers(content_dir: &Path) -> Result<()> {
    let dir = content_dir.join("lexicon");
    let read = |name: &str| -> Result<Vec<(String, u32)>> {
        let path = dir.join(name);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            // overrides.tsv 是三列(多一列理由),前两列取法相同
            .filter_map(|l| {
                let mut it = l.split('\t');
                let w = it.next()?.trim().to_lowercase();
                let b: u32 = it.next()?.trim().parse().ok()?;
                Some((w, b))
            })
            .collect())
    };
    let base: std::collections::HashMap<String, u32> = read("base.tsv")?.into_iter().collect();
    let mut clashes = Vec::new();
    for (w, b) in read("supplement.tsv")? {
        if let Some(&bb) = base.get(&w) {
            clashes.push(format!("{w}: supplement {b} / base {bb}"));
        }
    }
    if clashes.is_empty() {
        // 覆盖层规模:报出来,免得它悄悄长成一张没人复审的表
        let overrides = read(LEXICON_OVERRIDES)?.len();
        let sup = read("supplement.tsv")?.len();
        println!(
            "lexicon layers: ok(base {} · supplement {} · 教学定级覆盖 {})",
            base.len(),
            sup,
            overrides
        );
        Ok(())
    } else {
        for c in &clashes {
            println!("  ✕ supplement.tsv 重定义了 base.tsv 的词 — {c}");
        }
        bail!("lexicon layers: {} 处冲突", clashes.len())
    }
}

/// few-shot 示例自检:prompt 里挂着的"合格示例"必须真的能过校验器。
///
/// 踩过的坑:旧例句用了 `passport`(不在词表)、`grade`(1612)、
/// `counter`(2539),在 L1–L3 一律判越级 —— 等于拿"会被自己拒掉的句子"
/// 教模型。模型照着学,再被拒掉,入库率就是这么掉下去的。
fn check_few_shots(content_dir: &Path) -> Result<()> {
    use sf_pipeline::parse::parse_single_draft;
    use sf_pipeline::validate::{DedupeIndex, VerdictKind};

    let lexicon = load_lexicon(content_dir)?;
    let specs = load_specs(content_dir)?;
    let mut problems = Vec::new();
    let mut checked = 0usize;

    for (block, max_band) in sf_pipeline::prompt::FEW_SHOT_TIERS {
        // 该档必须在"词带正好等于 max_band"的最低等级上成立
        let spec = specs
            .values()
            .map(|(s, _)| s)
            .filter(|s| s.vocab_band > 0 && s.vocab_band <= max_band)
            .max_by_key(|s| s.vocab_band)
            .with_context(|| format!("no spec with vocab_band <= {max_band}"))?;
        let validator = Validator::new(spec, &lexicon);
        for line in block.lines() {
            let line = line.trim();
            // 只校验"合格示例"的 JSON 行;反例是故意写坏的
            if !line.starts_with('{') || !line.contains("\"words\"") {
                continue;
            }
            let draft = parse_single_draft(line)
                .map_err(|e| anyhow::anyhow!("few-shot 不是合法 JSON: {e}"))?;
            checked += 1;
            let report = validator.validate(&draft, "示例", "", &DedupeIndex::default());
            if report.verdict != VerdictKind::Pass {
                let reasons: Vec<String> = report
                    .issues
                    .iter()
                    .filter(|i| i.severity() != sf_pipeline::validate::Severity::AutoFixed)
                    .map(|i| i.zh_reason())
                    .collect();
                problems.push(format!(
                    "  ✕ [{}] 在 {} 上 {:?}: {}",
                    draft.en,
                    spec.id,
                    report.verdict,
                    reasons.join("；")
                ));
            }
        }
    }
    if problems.is_empty() {
        println!("few-shot 示例: {checked}/{checked} 通过各自档位的最低等级校验");
        Ok(())
    } else {
        for p in &problems {
            println!("{p}");
        }
        bail!("few-shot 示例有 {} 条过不了自己的校验器", problems.len())
    }
}

/// 词表 IPA 体检:每条音标都必须只用校验器接受的字符
/// ([`sf_pipeline::validate::IPA_ALLOWED`])。
///
/// 词典对账会拿词表的音标覆写模型给的音标。词表里混进一个非法字符
/// (最常见的是 `/` 斜杠、重音符号写成 ASCII 的 `'`、或者用了 `ʤ`/`ʧ`
/// 这类合并符号),生成时就判 BadIpaChars → 每遇到这个词发一次修补请求。
/// 一个错字符能持续烧额度,所以在构建期拦。
fn check_lexicon_ipa(content_dir: &Path) -> Result<()> {
    use sf_pipeline::validate::IPA_ALLOWED;

    let allowed: std::collections::HashSet<char> = IPA_ALLOWED.chars().collect();
    let mut checked = 0usize;
    let mut problems = Vec::new();
    for (name, text) in [
        (
            "base.tsv",
            std::fs::read_to_string(content_dir.join("lexicon/base.tsv")).ok(),
        ),
        (
            "supplement.tsv",
            std::fs::read_to_string(content_dir.join("lexicon/supplement.tsv")).ok(),
        ),
    ] {
        let Some(text) = text else { continue };
        for (ln, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            let (Some(lemma), Some(ipa)) = (cols.first(), cols.get(2)) else {
                continue;
            };
            let ipa = ipa.trim();
            if ipa.is_empty() {
                continue;
            }
            checked += 1;
            let bad: Vec<char> = ipa.chars().filter(|c| !allowed.contains(c)).collect();
            if !bad.is_empty() {
                problems.push(format!(
                    "  ✕ {name}:{} `{lemma}` 音标 `{ipa}` 含非法字符 {bad:?}",
                    ln + 1
                ));
            }
        }
    }
    if problems.is_empty() {
        println!("词表 IPA: {checked}/{checked} 字符合法");
        Ok(())
    } else {
        for p in &problems {
            println!("{p}");
        }
        bail!("词表 IPA: {} 条非法", problems.len())
    }
}

fn gold_run(content_dir: &Path) -> Result<()> {
    check_lexicon_layers(content_dir)?;
    check_lexicon_ipa(content_dir)?;
    check_few_shots(content_dir)?;
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
/// 按名字构造一个通道适配器(CLI 侧共用)。
/// 代理走 `SF_PROXY`,Key 走 `--api-key` 或 `SF_API_KEY`。
fn build_adapter(
    channel: &str,
    api_key: Option<String>,
) -> Result<Box<dyn sf_llm::ChannelAdapter>> {
    use sf_llm::channels::{DeepseekChannel, OllamaChannel, OpencodeChannel, ZenChannel};
    use sf_llm::meter::PriceTable;

    let key = api_key.or_else(|| std::env::var("SF_API_KEY").ok());
    let proxy = std::env::var("SF_PROXY")
        .ok()
        .filter(|p| !p.trim().is_empty());
    Ok(match channel {
        "opencode" => Box::new(OpencodeChannel::new(
            sf_llm::channels::opencode::OpencodeConfig {
                bin_override: std::env::var("SF_OPENCODE_BIN").ok().map(Into::into),
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
    })
}

/// 发一次生成请求,把整段产出取回来(探针用;工坊/gen 走流式)。
fn run_generation(
    channel: &str,
    model: &str,
    parts: sf_pipeline::prompt::PromptParts,
    api_key: Option<String>,
) -> Result<String> {
    use futures::StreamExt;
    use sf_llm::types::GenChunk;

    let adapter = build_adapter(channel, api_key)?;
    let req = sf_llm::types::GenRequest {
        model: model.to_string(),
        system: parts.system,
        user: parts.user,
        max_tokens: Some(8192),
        temperature: Some(0.7),
    };
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut stream = adapter
            .complete_stream(req)
            .await
            .map_err(|e| anyhow::anyhow!("channel error: {e} ({})", e.zh_message()))?;
        let mut text = String::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(GenChunk::Text { text: t }) => text.push_str(&t),
                Ok(GenChunk::Usage {
                    prompt_tokens,
                    completion_tokens,
                }) => eprintln!("usage: {prompt_tokens} in / {completion_tokens} out"),
                Ok(GenChunk::Done) => break,
                Ok(_) => {}
                Err(e) => bail!("stream error: {e} ({})", e.zh_message()),
            }
        }
        Ok(text)
    })
}

/// 一次探针运行的统计。按"句"计:一句有多个同类问题只算一次。
#[derive(Default)]
struct YieldStats {
    total: usize,
    stored: usize,
    parse_errors: usize,
    by_outcome: BTreeMap<&'static str, usize>,
    by_issue: BTreeMap<String, usize>,
    sole_cause: BTreeMap<String, usize>,
    over_words: BTreeMap<String, usize>,
    unknown_words: BTreeMap<String, usize>,
}

fn pct(n: usize, total: usize) -> f64 {
    if total == 0 {
        0.0
    } else {
        n as f64 * 100.0 / total as f64
    }
}

fn issue_kind(i: &sf_pipeline::validate::ValidationIssue) -> &'static str {
    use sf_pipeline::validate::ValidationIssue::*;
    match i {
        EmptyEnglish => "缺英文",
        EmptyChinese => "缺中文",
        NoWords => "缺逐词标注",
        TokenMismatch { .. } => "逐词与句子不一致",
        UnknownPosTag { .. } => "词性标签非法",
        UnknownRoleTag { .. } => "成分标签非法",
        ChunkIndexOutOfRange { .. } => "成分索引越界",
        ChunkOverlap { .. } => "成分重叠",
        ChunkGap { .. } => "成分未覆盖全句",
        BadIpaChars { .. } => "音标含非法字符",
        MissingIpa { .. } => "缺音标",
        Grammar { .. } => "语法错",
        OverLevel { .. } => "越级(词偏难)",
        UnknownWord { .. } => "词表外",
        TooLong { .. } => "句子过长",
        NearDuplicate { .. } => "与已有句重复",
        IpaReconciled { .. } => "音标已校正",
    }
}

/// 把一段原始产出喂进流水线,累计统计(与工坊同一套:流式扫描 + 校验 + 分诊)。
fn tally_yield(
    text: &str,
    validator: &Validator<'_>,
    all_specs: &[LevelSpec],
    dedupe: &mut DedupeIndex,
    st: &mut YieldStats,
) {
    use sf_pipeline::parse::StreamScanner;
    use sf_pipeline::validate::Severity;

    let mut scanner = StreamScanner::new();
    for item in scanner.push(text) {
        let Ok(d) = item else {
            st.parse_errors += 1;
            continue;
        };
        st.total += 1;
        let report = validator.validate(&d, "probe", "", dedupe);

        let mut kinds: std::collections::BTreeSet<String> = Default::default();
        for i in &report.issues {
            if i.severity() == Severity::AutoFixed {
                continue;
            }
            kinds.insert(issue_kind(i).to_string());
            match i {
                sf_pipeline::validate::ValidationIssue::UnknownWord { word } => {
                    *st.unknown_words.entry(word.to_lowercase()).or_default() += 1;
                }
                sf_pipeline::validate::ValidationIssue::OverLevel { word, band, .. } => {
                    *st.over_words
                        .entry(format!("{}({band})", word.to_lowercase()))
                        .or_default() += 1;
                }
                _ => {}
            }
        }
        for k in &kinds {
            *st.by_issue.entry(k.clone()).or_default() += 1;
        }
        if kinds.len() == 1 {
            *st.sole_cause
                .entry(kinds.iter().next().cloned().unwrap_or_default())
                .or_default() += 1;
        }

        // 分诊后的真实去向 —— 这才是用户看到的"入库率"
        let label = match triage(report, GenProfile::User, all_specs) {
            TriageOutcome::Accept { sentence } => {
                dedupe.add(sentence.en.as_str());
                st.stored += 1;
                "入库"
            }
            TriageOutcome::Relevel { sentence, .. } => {
                dedupe.add(sentence.en.as_str());
                st.stored += 1;
                "入库(改级)"
            }
            TriageOutcome::Repair { .. } => "需修补(要多发一次请求)",
            TriageOutcome::Discard {
                recoverable: Some(_),
                ..
            } => "丢弃(可捞回)",
            TriageOutcome::Discard { .. } => "丢弃",
        };
        *st.by_outcome.entry(label).or_default() += 1;
    }
}

fn report_yield(st: &YieldStats) {
    println!(
        "\n== 样本 {} 句 · 入库 {} 句({:.0}%)==",
        st.total,
        st.stored,
        pct(st.stored, st.total)
    );
    if st.parse_errors > 0 {
        println!("元素级 JSON 解析失败: {}", st.parse_errors);
    }
    println!("\n-- 去向 --");
    let mut v: Vec<_> = st.by_outcome.iter().collect();
    v.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (k, n) in v {
        println!("  {k:<24} {n:>4}  {:.0}%", pct(*n, st.total));
    }
    if !st.by_issue.is_empty() {
        println!("\n-- 问题(按句计,一句可多因)--");
        let mut v: Vec<_> = st.by_issue.iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        for (k, n) in v {
            println!("  {k:<24} {n:>4}  {:.0}%", pct(*n, st.total));
        }
        println!("\n-- 唯一原因(修掉即可救回)--");
        let mut v: Vec<_> = st.sole_cause.iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        for (k, n) in v {
            println!("  {k:<24} {n:>4}  {:.0}%", pct(*n, st.total));
        }
    }
    let dump = |title: &str, m: &BTreeMap<String, usize>| {
        if m.is_empty() {
            return;
        }
        let mut v: Vec<_> = m.iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
        let line: Vec<String> = v.iter().take(30).map(|(w, n)| format!("{w}×{n}")).collect();
        println!(
            "\n-- {title}(共 {} 个不同词)--\n  {}",
            m.len(),
            line.join(" ")
        );
    };
    dump("词表外", &st.unknown_words);
    dump("越带", &st.over_words);
}

/// `sf factory run` —— 见 [`FactoryCmd::Run`] 的说明。
#[allow(clippy::too_many_arguments)]
fn run_level(
    level: &str,
    batches: u32,
    count: u32,
    category: Option<&str>,
    channel: &str,
    model: &str,
    content_dir: &Path,
    db: &Path,
    api_key: Option<String>,
    scenes_path: &Path,
) -> Result<()> {
    let book: SceneBook = serde_yaml::from_str(
        &std::fs::read_to_string(scenes_path)
            .with_context(|| format!("reading {}", scenes_path.display()))?,
    )
    .with_context(|| format!("parsing {}", scenes_path.display()))?;

    let todo: Vec<&SceneEntry> = book
        .scenes
        .iter()
        .filter(|s| s.levels.iter().any(|l| l == level))
        .filter(|s| category.is_none_or(|c| s.category == c))
        .collect();
    if todo.is_empty() {
        bail!("{scenes_path:?} 里没有 {level} 的场景");
    }
    let target = book.targets.get(level).copied();
    println!(
        "=== {level}:{} 个场景 × {batches} 批 × {count} 句{} ===",
        todo.len(),
        target
            .map(|t| format!(" · 目标 {t} 句"))
            .unwrap_or_default()
    );

    let mut failed = 0usize;
    for (i, sc) in todo.iter().enumerate() {
        for b in 1..=batches {
            println!("--- [{}/{}] {} 批{b} ---", i + 1, todo.len(), sc.name);
            if let Err(e) = gen_cmd(
                &sc.name,
                level,
                count,
                channel,
                model,
                content_dir,
                db,
                api_key.clone(),
            ) {
                // 单批失败不该断掉整级 —— 限速、网络抖动都属常态
                println!("  ! 本批失败,继续:{e}");
                failed += 1;
            }
        }
    }
    println!("=== {level} 完成(失败批次 {failed})===");

    let store = ContentStore::open_rw(db).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "  库内句数:{}",
        store
            .sentence_count()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
    );
    Ok(())
}

/// `content/scenes.yaml` 的一条场景。
#[derive(Debug, serde::Deserialize)]
struct SceneEntry {
    id: String,
    name: String,
    category: String,
    levels: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct SceneBook {
    #[serde(default)]
    targets: BTreeMap<String, u32>,
    scenes: Vec<SceneEntry>,
}

/// 一个场景扫出来的结果。
struct SceneScan {
    id: String,
    name: String,
    category: String,
    level: LevelId,
    total: usize,
    /// 按本级原样入库的句数。
    kept: usize,
    /// 合格但被改存到更高等级的句数 —— 这一项高说明该场景在这一级
    /// 名不副实:句子没丢,但产不出**本级**的内容。
    relevelled: usize,
    unknown: Vec<String>,
    over: Vec<(String, u32)>,
}

impl SceneScan {
    fn stored(&self) -> usize {
        self.kept + self.relevelled
    }
}

/// `sf factory scan` —— 见 [`FactoryCmd::Scan`] 的说明。
#[allow(clippy::too_many_arguments)]
fn scan_cmd(
    scenes_path: &Path,
    count: u32,
    category: Option<&str>,
    limit: Option<usize>,
    channel: &str,
    model: &str,
    out: Option<&Path>,
    content_dir: &Path,
    api_key: Option<String>,
) -> Result<()> {
    use sf_pipeline::validate::{Severity, ValidationIssue};

    let book: SceneBook = serde_yaml::from_str(
        &std::fs::read_to_string(scenes_path)
            .with_context(|| format!("reading {}", scenes_path.display()))?,
    )
    .with_context(|| format!("parsing {}", scenes_path.display()))?;

    let specs = load_specs(content_dir)?;
    let lexicon = load_lexicon(content_dir)?;

    let mut todo: Vec<&SceneEntry> = book
        .scenes
        .iter()
        .filter(|s| category.is_none_or(|c| s.category == c))
        .collect();
    if let Some(n) = limit {
        todo.truncate(n);
    }

    // 覆盖面小结:清单本身能不能撑起 targets
    println!(
        "场景清单 {} —— {} 个场景",
        scenes_path.display(),
        book.scenes.len()
    );
    if !book.targets.is_empty() {
        let mut per_level: BTreeMap<&str, usize> = BTreeMap::new();
        for s in &book.scenes {
            for lv in &s.levels {
                *per_level.entry(lv.as_str()).or_default() += 1;
            }
        }
        for (lv, target) in &book.targets {
            let n = per_level.get(lv.as_str()).copied().unwrap_or(0);
            let per = if n > 0 { target.div_ceil(n as u32) } else { 0 };
            println!("  {lv}: {n} 个场景 · 目标 {target} 句 → 每场景 {per} 句");
        }
    }
    println!("待扫 {} 个场景,每个试产 {count} 句\n", todo.len());

    let mut results: Vec<SceneScan> = Vec::new();
    for (i, sc) in todo.iter().enumerate() {
        // 在该场景**最低**的目标等级上扫:词表带最紧,最容易暴露缺词。
        let level: LevelId = sc
            .levels
            .iter()
            .filter_map(|l| l.parse::<LevelId>().ok())
            .min()
            .with_context(|| format!("场景 {} 没有合法的 levels", sc.id))?;
        let (spec, _) = specs.get(&level).context("no spec for level")?;
        let parts = sf_pipeline::prompt::build_prompt(spec, &sc.name, count, &[], &[]);
        eprint!("[{}/{}] {} ({level}) ... ", i + 1, todo.len(), sc.name);

        let text = match run_generation(channel, model, parts, api_key.clone()) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("跳过:{e}");
                continue;
            }
        };

        let validator = Validator::new(spec, &lexicon);
        let mut dedupe = DedupeIndex::default();
        let mut scan = SceneScan {
            id: sc.id.clone(),
            name: sc.name.clone(),
            category: sc.category.clone(),
            level,
            total: 0,
            kept: 0,
            relevelled: 0,
            unknown: Vec::new(),
            over: Vec::new(),
        };
        let all_specs: Vec<LevelSpec> = specs.values().map(|(s, _)| s.clone()).collect();
        let mut scanner = sf_pipeline::parse::StreamScanner::new();
        for item in scanner.push(&text) {
            let Ok(d) = item else { continue };
            scan.total += 1;
            let report = validator.validate(&d, &sc.name, "", &dedupe);
            for issue in &report.issues {
                if issue.severity() == Severity::AutoFixed {
                    continue;
                }
                match issue {
                    ValidationIssue::UnknownWord { word } => {
                        let w = word.to_lowercase();
                        if !scan.unknown.contains(&w) {
                            scan.unknown.push(w);
                        }
                    }
                    ValidationIssue::OverLevel { word, band, .. } => {
                        let w = word.to_lowercase();
                        if !scan.over.iter().any(|(x, _)| *x == w) {
                            scan.over.push((w, *band));
                        }
                    }
                    _ => {}
                }
            }
            match triage(report, GenProfile::User, &all_specs) {
                TriageOutcome::Accept { sentence } => {
                    dedupe.add(sentence.en.as_str());
                    scan.kept += 1;
                }
                TriageOutcome::Relevel { sentence, .. } => {
                    dedupe.add(sentence.en.as_str());
                    scan.relevelled += 1;
                }
                _ => {}
            }
        }
        eprintln!(
            "本级 {}/{} · 改级 {} · 缺词 {} · 越带 {}",
            scan.kept,
            scan.total,
            scan.relevelled,
            scan.unknown.len(),
            scan.over.len()
        );
        results.push(scan);
    }

    let report = render_scan(&results);
    println!("{report}");
    if let Some(path) = out {
        std::fs::write(path, &report)?;
        eprintln!("报告已写到 {}", path.display());
    }
    Ok(())
}

/// 把扫描结果渲染成 Markdown(逐场景表 + 候选词条)。
fn render_scan(results: &[SceneScan]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let total: usize = results.iter().map(|r| r.total).sum();
    let kept: usize = results.iter().map(|r| r.kept).sum();
    let stored: usize = results.iter().map(SceneScan::stored).sum();

    let _ = writeln!(out, "# 场景词汇预扫报告\n");
    let _ = writeln!(
        out,
        "扫了 {} 个场景 · 试产 {total} 句 · 入库 {stored} 句({:.0}%),其中按本级入库 {kept} 句({:.0}%)\n",
        results.len(),
        pct(stored, total),
        pct(kept, total)
    );

    // 只列有问题的场景 —— 全过的场景不用占版面
    let mut problem: Vec<&SceneScan> = results
        .iter()
        .filter(|r| !r.unknown.is_empty() || !r.over.is_empty())
        .collect();
    // 先按「丢了多少 + 被推走多少」排,最该处理的排最前
    problem.sort_by_key(|r| std::cmp::Reverse(r.total - r.kept));

    let _ = writeln!(out, "## 需要补词的场景\n");
    if problem.is_empty() {
        let _ = writeln!(out, "无 —— 所有场景的词都在词表内。\n");
    } else {
        let _ = writeln!(
            out,
            "`本级` = 按扫描等级原样入库;`改级` = 合格但被推到更高等级 —— 这一列高
说明该场景在这一级产不出本级内容。
"
        );
        let _ = writeln!(
            out,
            "| 场景 | 类别 | 等级 | 本级 | 改级 | 丢 | 词表外 | 越带 |"
        );
        let _ = writeln!(
            out,
            "|------|------|------|------|------|----|--------|------|"
        );
        for r in &problem {
            let over = r
                .over
                .iter()
                .map(|(w, b)| format!("{w}({b})"))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = writeln!(
                out,
                "| {} `{}` | {} | {} | {}/{} | {} | {} | {} | {} |",
                r.name,
                r.id,
                r.category,
                r.level,
                r.kept,
                r.total,
                r.relevelled,
                r.total - r.stored(),
                if r.unknown.is_empty() {
                    "—".into()
                } else {
                    r.unknown.join(" ")
                },
                if over.is_empty() { "—".into() } else { over },
            );
        }
        let _ = writeln!(out);
    }

    // 候选词条:词表外的词按"最早需要它的等级"给 band
    let mut candidates: BTreeMap<String, (LevelId, Vec<String>)> = BTreeMap::new();
    for r in results {
        for w in &r.unknown {
            let e = candidates.entry(w.clone()).or_insert((r.level, Vec::new()));
            if r.level < e.0 {
                e.0 = r.level;
            }
            if !e.1.contains(&r.name) {
                e.1.push(r.name.clone());
            }
        }
    }
    let _ = writeln!(out, "## 候选词条(粘进 content/lexicon/supplement.tsv)\n");
    if candidates.is_empty() {
        let _ = writeln!(out, "无。\n");
    } else {
        let _ = writeln!(
            out,
            "band 给的是**保守值**:落在\"最早需要它的那一级\"的下一级。照这个值收,\n该场景在本级用到它时会被判越级、由分诊改存到上一级 —— 宁可这样,\n也不要让低等级凭空冒生词。\n\n**想让它在本级就能用,把 band 调进本级词带上限之内**(L1≤500 / L2≤1000 /\nL3≤1500 / L4≤2000 / L5≤2800 / L6 不限)。这个判断机器做不了 —— \n`pizza` 对 L2 学习者不难,`prescription` 就难。\n\n还要人工补的:**IPA(英式/美式)与中文释义**;以及把屈折形式还原成词元\n(表里收 `painkiller`,`painkillers` 会自动按后缀还原命中)。\n"
        );
        let _ = writeln!(out, "```tsv");
        for (w, (lv, scenes)) in &candidates {
            let _ = writeln!(
                out,
                "{w}\t{}\t\t\t\t# {} —— {}",
                suggested_band(*lv),
                lv,
                scenes.join(" / ")
            );
        }
        let _ = writeln!(out, "```\n");
    }

    // 越带词:不用补表,分诊会改级,但值得知道哪些场景会被推高
    let mut over_all: BTreeMap<String, (u32, Vec<String>)> = BTreeMap::new();
    for r in results {
        for (w, b) in &r.over {
            let e = over_all.entry(w.clone()).or_insert((*b, Vec::new()));
            if !e.1.contains(&r.name) {
                e.1.push(r.name.clone());
            }
        }
    }
    if !over_all.is_empty() {
        let _ = writeln!(out, "## 越带词(不必补表,分诊会改存到合适等级)\n");
        let mut v: Vec<_> = over_all.iter().collect();
        v.sort_by_key(|(_, (b, _))| std::cmp::Reverse(*b));
        for (w, (b, scenes)) in v {
            let _ = writeln!(out, "- `{w}`({b}) —— {}", scenes.join(" / "));
        }
        let _ = writeln!(out);
    }
    out
}

/// 给新词建议的 band:落在"最早需要它的那一级"的下一级。
fn suggested_band(level: LevelId) -> u32 {
    match level {
        LevelId::L1 => 600,
        LevelId::L2 => 1100,
        LevelId::L3 => 1600,
        LevelId::L4 => 2100,
        LevelId::L5 => 2900,
        LevelId::L6 => 2900,
    }
}

/// `sf factory yield` —— 见 [`FactoryCmd::Yield`] 的说明。
#[allow(clippy::too_many_arguments)]
fn yield_cmd(
    level: &str,
    scene: Option<&str>,
    count: u32,
    channel: &str,
    model: &str,
    save: Option<&Path>,
    replay: &[PathBuf],
    content_dir: &Path,
    api_key: Option<String>,
) -> Result<()> {
    let level: LevelId = level.parse().map_err(|e: String| anyhow::anyhow!(e))?;
    let specs = load_specs(content_dir)?;
    let (spec, _) = specs.get(&level).context("no spec for level")?;
    let all_specs: Vec<LevelSpec> = specs.values().map(|(s, _)| s.clone()).collect();
    let lexicon = load_lexicon(content_dir)?;
    let validator = Validator::new(spec, &lexicon);
    let mut dedupe = DedupeIndex::default();
    let mut st = YieldStats::default();

    if !replay.is_empty() {
        for path in replay {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            tally_yield(&text, &validator, &all_specs, &mut dedupe, &mut st);
        }
        report_yield(&st);
        return Ok(());
    }

    let scene = scene.context("现场模式需要 --scene(回放模式用 --replay)")?;
    let parts = sf_pipeline::prompt::build_prompt(spec, scene, count, &[], &[]);
    eprintln!(
        "prompt: system {} 字符 / user {} 字符",
        parts.system.chars().count(),
        parts.user.chars().count()
    );
    let text = run_generation(channel, model, parts, api_key)?;
    if let Some(path) = save {
        std::fs::write(path, &text)?;
        eprintln!("原始产出已存到 {}", path.display());
    }
    tally_yield(&text, &validator, &all_specs, &mut dedupe, &mut st);
    report_yield(&st);
    Ok(())
}

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
    // 出厂库查重看**全库**,不是本场景。
    //
    // 踩过的坑:先前跟着工坊改成了"场景内",结果同一句被存进 6 个场景 ——
    // 实测 516 句里 124 句(24%)是跨场景的完全同文,`What is your name?`
    // 存了 6 份。工坊那个策略是给个人库设计的(它另有 exists_by_en 全局
    // 兜底),出厂库是成品,用户按等级浏览时整级句子一眼看得见,不能重。
    //
    // 全局阈值安全性有实测支撑:L1 的 378 条不同文本两两比较(71253 对),
    // 相似度 ≥0.65 的只有 12 对,且全部是真近重
    // (`I am a teacher.` / `Yes, I am a teacher.`),零假阳性。
    let mut dedupe = DedupeIndex::new(
        store
            .all_sentences_en()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?,
    );
    // 种子句也要避开 —— 它们最终会和生成内容一起进 content.db。
    // (generated.db 与 content.db 分开,所以这里得显式把种子加进来。)
    for sentence in &run_seeds(content_dir)?.accepted {
        dedupe.add(sentence.en.as_str());
    }

    let adapter = build_adapter(channel, api_key)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let avoid = dedupe.recent(8);
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
        // 修补队列:流结束后统一发,避免和主流式请求抢通道
        let mut pending_repairs: Vec<(sf_core::Sentence, Vec<String>)> = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(GenChunk::Text { text }) => {
                    for draft in scanner.push(&text) {
                        match draft {
                            Ok(d) => {
                                let report = validator.validate(&d, scene, "", &dedupe);
                                match triage(report, GenProfile::Factory, &all_specs) {
                                    TriageOutcome::Accept { sentence }
                                    | TriageOutcome::Relevel { sentence, .. } => {
                                        // 完全同文兜底:近重阈值之外再挡一次,
                                        // 保证成品库里不会出现两条一模一样的句子
                                        // (工坊的 exists_by_en 对应物)。
                                        if store
                                            .sentence_id_by_en(&sentence.en)
                                            .map_err(|e| anyhow::anyhow!(e.to_string()))?
                                            .is_some()
                                        {
                                            println!("  ✕ 已有完全相同的句子:{}", sentence.en);
                                            discarded += 1;
                                            continue;
                                        }
                                        dedupe.add(sentence.en.as_str());
                                        store
                                            .insert_sentence(&sentence, "", 1)
                                            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                                        accepted += 1;
                                        println!("  ✓ {}", sentence.en);
                                    }
                                    TriageOutcome::Repair { sentence, issues } => {
                                        // 排队,流结束后统一发修补调用(仅传差异,§7.4)。
                                        // 桌面工坊早就这么做了,CLI 这边一直是 TODO ——
                                        // 于是音标缺失、语法小错这类"改一下就能用"的句子
                                        // 在出厂生产里被白白丢掉。
                                        println!("  ⟳ {} (待修补)", sentence.en);
                                        pending_repairs.push((
                                            sentence,
                                            issues.iter().map(|i| i.zh_reason()).collect(),
                                        ));
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
        // ---- 修补循环:每句一次,仅传差异;修不好才算丢 ----
        for (broken, reasons) in pending_repairs {
            match repair_one(&*adapter, model, &broken, &reasons, &validator, &dedupe).await {
                Some(fixed) => {
                    dedupe.add(fixed.en.as_str());
                    store
                        .insert_sentence(&fixed, "", 1)
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                    accepted += 1;
                    println!("  ✓ {} (已修补)", fixed.en);
                }
                None => {
                    println!("  ✕ 修补未过:{}", broken.en);
                    discarded += 1;
                }
            }
        }
        println!("accepted {accepted} · discarded {discarded}");
        Ok(())
    })
}

/// 单次修补调用(§7.4 仅传差异):让模型只改这几处问题,重新校验,
/// 干净通过才收。与桌面工坊 `workshop::run_repair` 同一套做法。
async fn repair_one(
    adapter: &dyn sf_llm::ChannelAdapter,
    model: &str,
    broken: &sf_core::Sentence,
    reasons: &[String],
    validator: &Validator<'_>,
    dedupe: &DedupeIndex,
) -> Option<sf_core::Sentence> {
    use futures::StreamExt;
    use sf_llm::types::GenChunk;

    let parts = sf_pipeline::prompt::build_repair_prompt(&broken.en, reasons);
    let req = sf_llm::types::GenRequest {
        model: model.to_string(),
        system: parts.system,
        user: parts.user,
        max_tokens: Some(2048),
        // 修补要的是"照着改",不是再创作 —— 温度压低
        temperature: Some(0.2),
    };
    let mut stream = adapter.complete_stream(req).await.ok()?;
    let mut text = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(GenChunk::Text { text: t }) => text.push_str(&t),
            Ok(GenChunk::Done) => break,
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    let draft = sf_pipeline::parse::parse_single_draft(&text).ok()?;
    let report = validator.validate(&draft, &broken.scene, &broken.func, dedupe);
    match report.verdict {
        sf_pipeline::validate::VerdictKind::Pass => report.sentence,
        _ => None,
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01B3);
    }
    h
}
