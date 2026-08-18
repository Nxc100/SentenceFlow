//! 生成工坊 runner (spec §4.4, §6.3): micro-batched streaming generation with
//! live validation badges, budget/rate guard rails, resumable jobs.
//!
//! Frontend contract (Tauri events, all payloads JSON):
//! * `workshop://card`     — one sentence card update
//! * `workshop://progress` — job/batch progress (进度点)
//! * `workshop://backoff`  — rate-limit countdown (免费额度限速…)
//! * `workshop://meter`    — CostBar live numbers
//! * `workshop://done`     — job finished/paused with summary
//! * `workshop://error`    — inline error card (Key 失效等, 非全局弹窗)

use crate::channels::{effective_prices, make_adapter};
use crate::error::{CmdError, CmdResult};
use crate::state::{AppState, now_unix};
use futures::StreamExt;
use serde::Serialize;
use sf_core::sentence::LevelId;
use sf_llm::backoff::{BackoffPolicy, BackoffState};
use sf_llm::estimate::estimate_tokens;
use sf_llm::meter::{BudgetVerdict, MoneyMeter};
use sf_llm::queue::{GenJob, GenMode, JobParams, JobState, MAX_TOPUP_BATCHES};
use sf_llm::types::{ChannelError, ChannelId, GenChunk, GenRequest};
use sf_pipeline::parse::StreamScanner;
use sf_pipeline::prompt::build_prompt;
use sf_pipeline::triage::{GenProfile, TriageOutcome, triage};
use sf_pipeline::validate::{DedupeIndex, ValidationIssue, Validator};
use std::collections::BTreeSet;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CardEvent {
    /// 校验通过,已入库 (✓).
    Accepted {
        job_id: u64,
        sentence: sf_core::Sentence,
    },
    /// 修补中 (⟳).
    Repairing { job_id: u64, en: String },
    /// 丢弃,可展开原因,可捞回 (✕).
    Discarded {
        job_id: u64,
        en: String,
        reason: String,
        recoverable: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    pub job_id: u64,
    pub batches: Vec<sf_llm::queue::BatchState>,
    pub produced: u32,
    pub state: JobState,
}

#[derive(Debug, Clone, Serialize)]
pub struct MeterEvent {
    pub job_id: u64,
    /// 付费通道: 当前累计费用;免费通道: None.
    pub cost_cny: Option<f64>,
    pub today_requests: u32,
    pub warning: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoneEvent {
    pub job_id: u64,
    pub state: JobState,
    pub produced: u32,
    pub discarded: u32,
    pub summary: String,
}

/// 生成过程活跃信号(`workshop://activity`,节流 ≤稍多于每 400ms 一条):
/// 慢通道上第一批可能跑 1–2 分钟,没有它界面完全静止,用户以为卡死。
/// `phase`:`connect` 连接通道 / `streaming` 正在产出(`n` = 已接收字符数)
/// / `repairing` 修补中(`n` = 待修补句数)。
#[derive(Debug, Clone, Serialize)]
pub struct ActivityEvent {
    pub job_id: u64,
    pub phase: String,
    pub n: u64,
    /// 当前批次(1 起)与总批数,给「第 x/y 批」文案。
    pub batch: usize,
    pub batches: usize,
}

/// streaming 活跃信号的最小间隔(事件风暴保护)。
const ACTIVITY_INTERVAL: std::time::Duration = std::time::Duration::from_millis(400);

fn emit<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
    let _ = app.emit(event, payload);
}

/// 精确查重护栏:入库前跨出厂库 + 用户库再查一次完全同文。simhash 近重
/// (阈值 16)已挡住绝大多数重复,这里保证"每个场景中的句子不重复"的
/// 硬承诺不受阈值校准影响。
pub(crate) fn exists_by_en(content: &sf_pipeline::store::ContentIndex, en: &str) -> bool {
    content
        .factory
        .sentence_id_by_en(en)
        .ok()
        .flatten()
        .is_some()
        || content
            .user
            .as_ref()
            .and_then(|u| u.sentence_id_by_en(en).ok().flatten())
            .is_some()
}

/// Single-shot repair call (§7.4 修补调用,仅传差异): asks the model to re-emit
/// one object, revalidates, returns the sentence only on a clean pass.
async fn run_repair(
    state: &Arc<AppState>,
    channel: ChannelId,
    model: &str,
    broken: &sf_core::Sentence,
    reasons: &[String],
    validator: &Validator<'_>,
    dedupe: &DedupeIndex,
) -> Option<sf_core::Sentence> {
    let parts = sf_pipeline::prompt::build_repair_prompt(&broken.en, reasons);
    let req = GenRequest {
        model: model.to_string(),
        system: parts.system,
        user: parts.user,
        max_tokens: Some(2048),
        temperature: Some(0.2),
    };
    let adapter = make_adapter(state, channel, None).ok()?;
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

/// Create a new job (persisted) and run it until done/paused/cancelled.
pub async fn run_new_job(
    app: AppHandle,
    state: Arc<AppState>,
    params: JobParams,
) -> CmdResult<u64> {
    let now = now_unix();
    let job_id = now as u64;
    let job = GenJob::new(job_id, params, now);
    {
        let progress = state.progress.lock().expect("progress lock");
        progress.save_job(&job)?;
    }
    run_job(app, state, job).await
}

/// Resume a paused job ([续跑], §6.3).
pub async fn resume_job(app: AppHandle, state: Arc<AppState>, job_id: u64) -> CmdResult<u64> {
    let mut job = {
        let progress = state.progress.lock().expect("progress lock");
        progress
            .load_job(job_id)?
            .ok_or_else(|| CmdError::new("workshop", "任务不存在"))?
    };
    job.resume();
    run_job(app, state, job).await
}

async fn run_job(app: AppHandle, state: Arc<AppState>, mut job: GenJob) -> CmdResult<u64> {
    state.take_gen_cancel(); // clear any stale cancel flag
    let job_id = job.job_id;
    let level: LevelId = job
        .params
        .level
        .parse()
        .map_err(|e: String| CmdError::new("workshop", e))?;
    let scenario = job.params.mode == GenMode::Scenario;
    // 场景模式:整批句子归入以任务场景命名的场景包(等级模式为空串)
    let pack = if scenario {
        job.params.scene.trim().to_string()
    } else {
        String::new()
    };
    // 场景对话不受词表带/句长阶梯约束(方案 §3.4):以最高等级 spec 为底
    // 克隆一份"放开带宽"的校验规格,结构/成分/音标/查重照旧强校验。
    let spec = if scenario {
        state.spec_for(LevelId::L6)?.clone() // 句长上限 20,词表判定见下
    } else {
        state.spec_for(level)?.clone()
    };
    let all_specs: Vec<_> = state.specs.values().cloned().collect();
    let channel: ChannelId =
        serde_json::from_value(serde_json::Value::String(job.params.channel.clone()))
            .map_err(|_| CmdError::new("workshop", format!("未知通道 {}", job.params.channel)))?;

    // Dedupe against everything already in the user library + this job.
    let mut dedupe = {
        let content = state.content.lock().expect("content lock");
        let mut hashes = content.factory.all_simhashes()?;
        if let Some(user) = &content.user {
            hashes.extend(user.all_simhashes()?);
        }
        DedupeIndex::new(hashes)
    };

    // Money metering only where money can burn (§5.5 双形态).
    let mut money = match channel {
        ChannelId::Deepseek => {
            let budget = state
                .settings
                .lock()
                .expect("settings lock")
                .ai
                .per_run_budget_cny;
            Some(MoneyMeter::new(effective_prices(&state), budget, 0))
        }
        _ => None,
    };
    let mut backoff = BackoffState::default();
    let backoff_policy = BackoffPolicy::default();
    let mut discarded_total = 0u32;
    // 失败样本回灌(§8 运行时形态):本任务中被词表校验拒绝过的单词,
    // 回灌进后续批次的 prompt 显式禁用,提高补足批通过率。
    let mut banned_words: BTreeSet<String> = BTreeSet::new();

    'job: loop {
        'batches: while let Some(batch_idx) = job.next_pending() {
            if state.gen_cancelled() {
                job.pause();
                break;
            }
            job.start_batch(batch_idx);
            emit(
                &app,
                "workshop://progress",
                ProgressEvent {
                    job_id,
                    batches: job.batches.clone(),
                    produced: job.produced,
                    state: job.state.clone(),
                },
            );

            let batch_size = job.request_size(batch_idx);
            let avoid: Vec<u64> = dedupe.recent(16).collect();
            let banned: Vec<String> = banned_words.iter().cloned().collect();
            let parts = if scenario {
                sf_pipeline::prompt::build_scenario_prompt(
                    &job.params.scene,
                    batch_size,
                    &avoid,
                    &banned,
                )
            } else {
                build_prompt(&spec, &job.params.scene, batch_size, &avoid, &banned)
            };
            if let Some(m) = &mut money {
                m.est_prompt_tokens = estimate_tokens(&parts.system) + estimate_tokens(&parts.user);
            }
            let req = GenRequest {
                model: job.params.model.clone(),
                system: parts.system,
                user: parts.user,
                max_tokens: Some(8192),
                temperature: Some(0.7),
            };

            let (batch_no, batch_total) = (batch_idx + 1, job.batches.len());
            let activity = |phase: &str, n: u64| {
                emit(
                    &app,
                    "workshop://activity",
                    ActivityEvent {
                        job_id,
                        phase: phase.into(),
                        n,
                        batch: batch_no,
                        batches: batch_total,
                    },
                );
            };

            activity("connect", 0);
            let adapter = make_adapter(&state, channel, None)?;
            let mut stream = match adapter.complete_stream(req).await {
                Ok(s) => {
                    backoff.on_success();
                    s
                }
                Err(ChannelError::RateLimited { retry_after_secs }) => {
                    let decision = backoff.on_rate_limited(&backoff_policy, Some(retry_after_secs));
                    emit(
                        &app,
                        "workshop://backoff",
                        serde_json::json!({
                            "job_id": job_id,
                            "wait_secs": decision.wait_secs,
                            "suggest_switch": decision.suggest_channel_switch,
                        }),
                    );
                    job.fail_batch(batch_idx);
                    tokio::time::sleep(std::time::Duration::from_secs(decision.wait_secs)).await;
                    job.resume();
                    continue 'batches;
                }
                Err(e) => {
                    emit(
                        &app,
                        "workshop://error",
                        serde_json::json!({
                            "job_id": job_id, "message": e.zh_message(),
                        }),
                    );
                    job.fail_batch(batch_idx);
                    break 'batches;
                }
            };

            let validator = if scenario {
                Validator::new_open_vocabulary(&spec, &state.lexicon)
            } else {
                Validator::new(&spec, &state.lexicon)
            };
            let mut scanner = StreamScanner::new();
            let mut accepted_in_batch = 0u32;
            let mut batch_failed = false;
            // 修补队列:流结束后统一发修补调用(仅传差异,§7.4)。
            let mut pending_repairs: Vec<(sf_core::Sentence, Vec<String>)> = Vec::new();
            // 活跃信号:已接收字符数,节流上报
            let mut stream_chars = 0u64;
            let mut last_activity = std::time::Instant::now();

            while let Some(chunk) = stream.next().await {
                if state.gen_cancelled() {
                    drop(stream);
                    job.finish_batch(batch_idx, accepted_in_batch);
                    job.pause();
                    break 'batches;
                }
                match chunk {
                    Ok(GenChunk::Text { text }) => {
                        stream_chars += text.chars().count() as u64;
                        if last_activity.elapsed() >= ACTIVITY_INTERVAL {
                            activity("streaming", stream_chars);
                            last_activity = std::time::Instant::now();
                        }
                        if let Some(m) = &mut money {
                            let verdict = m.add_completion_tokens(estimate_tokens(&text));
                            emit(
                                &app,
                                "workshop://meter",
                                MeterEvent {
                                    job_id,
                                    cost_cny: Some(m.current_cost()),
                                    today_requests: 0,
                                    warning: verdict != BudgetVerdict::Ok,
                                },
                            );
                            if verdict == BudgetVerdict::Exhausted {
                                // 触顶硬拦截,优雅收尾:保留已产出 (§6.3).
                                drop(stream);
                                job.finish_batch(batch_idx, accepted_in_batch);
                                job.pause();
                                emit(
                                    &app,
                                    "workshop://error",
                                    serde_json::json!({
                                        "job_id": job_id,
                                        "message": "已达单次预算上限,已入库完成部分",
                                        "budget_stop": true,
                                    }),
                                );
                                break 'batches;
                            }
                        }
                        for draft in scanner.push(&text) {
                            match draft {
                                Ok(d) => {
                                    // 场景对话:说话人存进 func 列(A/B 轮替)
                                    let speaker = d.speaker.trim().to_uppercase();
                                    let report = validator.validate(
                                        &d,
                                        &job.params.scene,
                                        &speaker,
                                        &dedupe,
                                    );
                                    for issue in &report.issues {
                                        if let ValidationIssue::OverLevel { word, .. }
                                        | ValidationIssue::UnknownWord { word } = issue
                                        {
                                            banned_words.insert(word.to_lowercase());
                                        }
                                    }
                                    let hash = report.simhash;
                                    match triage(report, GenProfile::User, &all_specs) {
                                        TriageOutcome::Accept { mut sentence } => {
                                            let content =
                                                state.content.lock().expect("content lock");
                                            // 精确查重护栏:simhash 近重之外再挡一次
                                            // 完全同文(保证场景内不重复)。
                                            if exists_by_en(&content, &sentence.en) {
                                                drop(content);
                                                discarded_total += 1;
                                                emit(
                                                    &app,
                                                    "workshop://card",
                                                    CardEvent::Discarded {
                                                        job_id,
                                                        en: sentence.en.clone(),
                                                        reason: "与句库已有句完全相同".into(),
                                                        recoverable: false,
                                                    },
                                                );
                                                continue;
                                            }
                                            dedupe.add(hash);
                                            if let Some(user) = &content.user {
                                                let rid = user.insert_sentence_in_pack(
                                                    &sentence, "", 1, &pack,
                                                )?;
                                                sentence.id =
                                                    sf_pipeline::store::ContentIndex::USER_ID_OFFSET
                                                        + rid;
                                            }
                                            drop(content);
                                            accepted_in_batch += 1;
                                            emit(
                                                &app,
                                                "workshop://card",
                                                CardEvent::Accepted { job_id, sentence },
                                            );
                                        }
                                        TriageOutcome::Repair { sentence, issues } => {
                                            emit(
                                                &app,
                                                "workshop://card",
                                                CardEvent::Repairing {
                                                    job_id,
                                                    en: sentence.en.clone(),
                                                },
                                            );
                                            let reasons =
                                                issues.iter().map(|i| i.zh_reason()).collect();
                                            pending_repairs.push((sentence, reasons));
                                        }
                                        TriageOutcome::Relevel { sentence, .. }
                                        | TriageOutcome::Discard {
                                            recoverable: Some(sentence),
                                            ..
                                        } => {
                                            discarded_total += 1;
                                            emit(
                                                &app,
                                                "workshop://card",
                                                CardEvent::Discarded {
                                                    job_id,
                                                    en: sentence.en.clone(),
                                                    reason: "超出当前等级或与已有句重复".into(),
                                                    recoverable: true,
                                                },
                                            );
                                        }
                                        TriageOutcome::Discard {
                                            recoverable: None,
                                            reason,
                                        } => {
                                            discarded_total += 1;
                                            emit(
                                                &app,
                                                "workshop://card",
                                                CardEvent::Discarded {
                                                    job_id,
                                                    en: String::new(),
                                                    reason,
                                                    recoverable: false,
                                                },
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    discarded_total += 1;
                                    emit(
                                        &app,
                                        "workshop://card",
                                        CardEvent::Discarded {
                                            job_id,
                                            en: String::new(),
                                            reason: format!(
                                                "AI 返回的格式有误,已跳过这条(不影响其他句子)。技术细节:{e}"
                                            ),
                                            recoverable: false,
                                        },
                                    );
                                }
                            }
                        }
                    }
                    Ok(GenChunk::Usage {
                        prompt_tokens,
                        completion_tokens,
                    }) => {
                        if let Some(m) = &mut money {
                            m.report_usage(prompt_tokens, completion_tokens);
                        }
                        let progress = state.progress.lock().expect("progress lock");
                        progress.spend_add(
                            now_unix(),
                            &job.params.channel,
                            prompt_tokens,
                            completion_tokens,
                            money.as_ref().map(|m| m.current_cost()).unwrap_or(0.0),
                        )?;
                    }
                    Ok(GenChunk::Done) => break,
                    Err(ChannelError::RateLimited { retry_after_secs }) => {
                        let decision =
                            backoff.on_rate_limited(&backoff_policy, Some(retry_after_secs));
                        emit(
                            &app,
                            "workshop://backoff",
                            serde_json::json!({
                                "job_id": job_id,
                                "wait_secs": decision.wait_secs,
                                "suggest_switch": decision.suggest_channel_switch,
                            }),
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(decision.wait_secs))
                            .await;
                    }
                    Err(e) => {
                        emit(
                            &app,
                            "workshop://error",
                            serde_json::json!({
                                "job_id": job_id, "message": e.zh_message(),
                            }),
                        );
                        batch_failed = true;
                        break;
                    }
                }
            }

            if batch_failed {
                // Keep what landed, return the batch for retry (§6.3 断点续跑).
                job.finish_batch(batch_idx, accepted_in_batch);
                job.pause();
                break 'batches;
            }

            // 修补调用:一句一发,单次尝试;修不好 → 丢弃可捞回 (§7.4).
            if !pending_repairs.is_empty() {
                activity("repairing", pending_repairs.len() as u64);
            }
            for (broken, reasons) in pending_repairs.drain(..) {
                if state.gen_cancelled() {
                    break;
                }
                match run_repair(
                    &state,
                    channel,
                    &job.params.model,
                    &broken,
                    &reasons,
                    &validator,
                    &dedupe,
                )
                .await
                {
                    Some(mut fixed) => {
                        {
                            let content = state.content.lock().expect("content lock");
                            if exists_by_en(&content, &fixed.en) {
                                drop(content);
                                discarded_total += 1;
                                emit(
                                    &app,
                                    "workshop://card",
                                    CardEvent::Discarded {
                                        job_id,
                                        en: fixed.en.clone(),
                                        reason: "与句库已有句完全相同".into(),
                                        recoverable: false,
                                    },
                                );
                                continue;
                            }
                            dedupe.add(fixed.simhash);
                            if let Some(user) = &content.user {
                                let rid = user.insert_sentence_in_pack(&fixed, "", 1, &pack)?;
                                fixed.id = sf_pipeline::store::ContentIndex::USER_ID_OFFSET + rid;
                            }
                        }
                        accepted_in_batch += 1;
                        emit(
                            &app,
                            "workshop://card",
                            CardEvent::Accepted {
                                job_id,
                                sentence: fixed,
                            },
                        );
                    }
                    None => {
                        discarded_total += 1;
                        emit(
                            &app,
                            "workshop://card",
                            CardEvent::Discarded {
                                job_id,
                                en: broken.en.clone(),
                                reason: format!("修补未通过:{}", reasons.join("；")),
                                recoverable: true,
                            },
                        );
                    }
                }
            }

            job.finish_batch(batch_idx, accepted_in_batch);
            {
                let progress = state.progress.lock().expect("progress lock");
                progress.save_job(&job)?;
            }
            emit(
                &app,
                "workshop://progress",
                ProgressEvent {
                    job_id,
                    batches: job.batches.clone(),
                    produced: job.produced,
                    state: job.state.clone(),
                },
            );
        }

        // 拿满机制:规划批全部完成但未达到用户指定句数 → 追加补足批继续,
        // 有限次触顶后诚实收尾(不达标场景见 done summary 的引导文案)。
        if job.state == JobState::Completed
            && job.shortfall() > 0
            && job.topup_count() < MAX_TOPUP_BATCHES
            && !state.gen_cancelled()
        {
            job.push_topup_batch();
            let progress = state.progress.lock().expect("progress lock");
            progress.save_job(&job)?;
            continue 'job;
        }
        break 'job;
    }

    {
        let progress = state.progress.lock().expect("progress lock");
        progress.save_job(&job)?;
    }
    // 拿满未遂时诚实收尾:说明原因并给出可行动的建议(§6.1 不甩锅给用户)。
    let summary = if job.state == JobState::Completed && job.shortfall() > 0 {
        format!(
            "目标 {} · 通过 {} · 丢弃 {}。已自动补生成 {} 轮仍未凑满——这个场景的常用说法超出了当前等级范围,建议提高等级或换个更日常的场景描述",
            job.params.total_sentences,
            job.produced,
            discarded_total,
            job.topup_count()
        )
    } else if scenario {
        format!(
            "{} 句对话已生成 · {} 丢弃 · 去「场景」页开练",
            job.produced, discarded_total
        )
    } else {
        format!("{} 通过 · {} 丢弃", job.produced, discarded_total)
    };
    emit(
        &app,
        "workshop://done",
        DoneEvent {
            job_id,
            state: job.state.clone(),
            produced: job.produced,
            discarded: discarded_total,
            summary,
        },
    );
    Ok(job_id)
}
