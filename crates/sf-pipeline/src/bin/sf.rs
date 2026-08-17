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
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Factory { cmd } => match cmd {
            FactoryCmd::Build {
                content_dir,
                out,
                rev,
            } => build(&content_dir, &out, rev),
            FactoryCmd::Validate { content_dir } => validate_seeds(&content_dir).map(|_| ()),
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
