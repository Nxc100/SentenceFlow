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
use sf_llm::queue::{GenJob, JobParams, JobState};
use sf_llm::types::{ChannelError, ChannelId, GenChunk, GenRequest};
use sf_pipeline::parse::StreamScanner;
use sf_pipeline::prompt::build_prompt;
use sf_pipeline::triage::{GenProfile, TriageOutcome, triage};
use sf_pipeline::validate::{DedupeIndex, Validator};
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

fn emit<T: Serialize + Clone>(app: &AppHandle, event: &str, payload: T) {
    let _ = app.emit(event, payload);
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
    let spec = state.spec_for(level)?.clone();
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

        let batch_size = job.batch_size(batch_idx);
        let avoid: Vec<u64> = dedupe.recent(16).collect();
        let parts = build_prompt(&spec, &job.params.scene, batch_size, &avoid);
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

        let validator = Validator::new(&spec, &state.lexicon);
        let mut scanner = StreamScanner::new();
        let mut accepted_in_batch = 0u32;
        let mut batch_failed = false;
        // 修补队列:流结束后统一发修补调用(仅传差异,§7.4)。
        let mut pending_repairs: Vec<(sf_core::Sentence, Vec<String>)> = Vec::new();

        while let Some(chunk) = stream.next().await {
            if state.gen_cancelled() {
                drop(stream);
                job.finish_batch(batch_idx, accepted_in_batch);
                job.pause();
                break 'batches;
            }
            match chunk {
                Ok(GenChunk::Text { text }) => {
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
                                let report = validator.validate(&d, &job.params.scene, "", &dedupe);
                                let hash = report.simhash;
                                match triage(report, GenProfile::User, &all_specs) {
                                    TriageOutcome::Accept { mut sentence } => {
                                        dedupe.add(hash);
                                        let content = state.content.lock().expect("content lock");
                                        if let Some(user) = &content.user {
                                            let rid = user.insert_sentence(&sentence, "", 1)?;
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
                                        reason: format!("JSON 解析失败: {e}"),
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
                    tokio::time::sleep(std::time::Duration::from_secs(decision.wait_secs)).await;
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
                    dedupe.add(fixed.simhash);
                    {
                        let content = state.content.lock().expect("content lock");
                        if let Some(user) = &content.user {
                            let rid = user.insert_sentence(&fixed, "", 1)?;
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

    {
        let progress = state.progress.lock().expect("progress lock");
        progress.save_job(&job)?;
    }
    emit(
        &app,
        "workshop://done",
        DoneEvent {
            job_id,
            state: job.state.clone(),
            produced: job.produced,
            discarded: discarded_total,
            summary: format!("{} 通过 · {} 丢弃", job.produced, discarded_total),
        },
    );
    Ok(job_id)
}
