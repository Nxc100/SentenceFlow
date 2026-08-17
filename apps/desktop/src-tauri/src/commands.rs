//! Tauri command surface — thin wrappers: rusqlite loads, sf-core decides,
//! rusqlite saves (spec §7.3).

use crate::channels;
use crate::error::{CmdError, CmdResult};
use crate::licensing::{self, LicenseState};
use crate::progress::ProgressDb;
use crate::settings::Settings;
use crate::state::{AppState, now_unix};
use crate::workshop;
use serde::{Deserialize, Serialize};
use sf_core::sentence::{LevelId, Sentence};
use sf_core::spec::LevelSpec;
use sf_core::srs::{Mode, Outcome, SrsState};
use sf_core::stats::{ErrorTag, LogResult, LogRow, StatsSummary, day_index};
use sf_core::{JudgePolicy, Session, SessionItem, Verdict};
use sf_llm::queue::{GenJob, JobParams};
use sf_llm::types::{ChannelId, ChannelStatus};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

type S<'a> = State<'a, Arc<AppState>>;

// ------------------------------------------------------------- bootstrap

#[derive(Debug, Serialize)]
pub struct Bootstrap {
    pub specs: Vec<LevelSpec>,
    pub license: LicenseState,
    pub settings: Settings,
    pub content_rev: Option<String>,
    pub sentence_count: u32,
}

#[tauri::command]
pub fn bootstrap(state: S<'_>) -> CmdResult<Bootstrap> {
    let license = licensing::current_state(&state.paths, now_unix())?;
    let settings = state.settings.lock().expect("settings lock").clone();
    let content = state.content.lock().expect("content lock");
    Ok(Bootstrap {
        specs: state.specs.values().cloned().collect(),
        license,
        settings,
        content_rev: content.factory.get_meta("rev")?,
        sentence_count: content.factory.sentence_count()?,
    })
}

// ------------------------------------------------------------- settings

#[tauri::command]
pub fn get_settings(state: S<'_>) -> CmdResult<Settings> {
    Ok(state.settings.lock().expect("settings lock").clone())
}

#[tauri::command]
pub fn set_settings(state: S<'_>, settings: Settings) -> CmdResult<()> {
    state.save_settings(&settings)?;
    *state.settings.lock().expect("settings lock") = settings;
    Ok(())
}

// ------------------------------------------------------------- license

#[tauri::command]
pub fn get_license_state(state: S<'_>) -> CmdResult<LicenseState> {
    licensing::current_state(&state.paths, now_unix())
}

#[tauri::command]
pub fn activate_license(state: S<'_>, sflic: String) -> CmdResult<LicenseState> {
    licensing::activate(&state.paths, &sflic)?;
    licensing::current_state(&state.paths, now_unix())
}

#[tauri::command]
pub fn export_license(state: S<'_>) -> CmdResult<String> {
    licensing::export_license(&state.paths)
}

// ------------------------------------------------------------- content

#[tauri::command]
pub fn list_scenes(state: S<'_>, level: LevelId) -> CmdResult<Vec<String>> {
    let content = state.content.lock().expect("content lock");
    Ok(content.factory.scenes(level)?)
}

#[tauri::command]
pub fn list_sentences(
    state: S<'_>,
    level: LevelId,
    scene: Option<String>,
) -> CmdResult<Vec<Sentence>> {
    let content = state.content.lock().expect("content lock");
    let mut all = content.sentences_by_level(level)?;
    if let Some(scene) = scene {
        all.retain(|s| s.scene == scene);
    }
    Ok(all)
}

#[tauri::command]
pub fn get_sentence(state: S<'_>, id: i64) -> CmdResult<Option<Sentence>> {
    let content = state.content.lock().expect("content lock");
    content.sentence_by_id(id).map_err(Into::into)
}

#[tauri::command]
pub fn delete_user_sentence(state: S<'_>, id: i64) -> CmdResult<()> {
    use sf_pipeline::store::ContentIndex;
    if id < ContentIndex::USER_ID_OFFSET {
        return Err(CmdError::new("content", "出厂句库不可删除"));
    }
    let content = state.content.lock().expect("content lock");
    let Some(user) = &content.user else {
        return Err(CmdError::new("content", "用户句库未初始化"));
    };
    user.delete_sentence(id - ContentIndex::USER_ID_OFFSET)?;
    Ok(())
}

/// 我的句集导入:"中文 Tab 英文"双列文本 (§4.3). Words/chunks are left empty —
/// imported sentences practise as typing-only until 解析 is generated for
/// them via the workshop.
#[tauri::command]
pub fn import_tab_sentences(state: S<'_>, level: LevelId, text: String) -> CmdResult<u32> {
    let content = state.content.lock().expect("content lock");
    let Some(user) = &content.user else {
        return Err(CmdError::new("content", "用户句库未初始化"));
    };
    let mut n = 0u32;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some((zh, en)) = line.split_once('\t') else {
            continue;
        };
        let (tokens, punct) = sf_pipeline::validate::tokenize_en(en.trim());
        if tokens.is_empty() {
            continue;
        }
        let sentence = Sentence {
            id: 0,
            level,
            scene: "导入".into(),
            func: String::new(),
            pattern: String::new(),
            zh: zh.trim().to_string(),
            en: en.trim().to_string(),
            punct,
            words: vec![],
            chunks: vec![],
            note: String::new(),
            simhash: sf_pipeline::simhash::simhash64(en),
        };
        user.insert_sentence(&sentence, "", 1)?;
        n += 1;
    }
    Ok(n)
}

// ------------------------------------------------------------- practice

#[derive(Debug, Serialize)]
pub struct TodayOverview {
    pub due_count: u32,
    pub new_available: u32,
    pub streak_days: u32,
    pub practiced_today: u32,
}

#[tauri::command]
pub fn today_overview(
    state: S<'_>,
    level: LevelId,
    tz_offset_secs: i32,
) -> CmdResult<TodayOverview> {
    let now = now_unix();
    let progress = state.progress.lock().expect("progress lock");
    let all_srs = progress.all_srs()?;
    let seen = progress.seen_ids()?;
    let logs = progress.all_logs()?;
    drop(progress);
    let content = state.content.lock().expect("content lock");
    let pool: Vec<i64> = content
        .sentences_by_level(level)?
        .iter()
        .map(|s| s.id)
        .filter(|id| !seen.contains(id))
        .collect();
    drop(content);
    let today = day_index(now, tz_offset_secs);
    let practiced_today = logs
        .iter()
        .filter(|l| day_index(l.ts, tz_offset_secs) == today)
        .count() as u32;
    let stats = sf_core::fold_stats(logs, tz_offset_secs);
    Ok(TodayOverview {
        due_count: all_srs.iter().filter(|(_, s)| s.is_due(now)).count() as u32,
        new_available: pool.len() as u32,
        streak_days: stats.streak_days,
        practiced_today,
    })
}

#[tauri::command]
pub fn start_session(state: S<'_>, level: LevelId, tz_offset_secs: i32) -> CmdResult<Session> {
    let now = now_unix();
    let spec = state.spec_for(level)?;
    let (daily_new, reorder_override) = {
        let settings = state.settings.lock().expect("settings lock");
        (settings.practice.daily_new, settings.practice.reorder_first)
    };
    let progress = state.progress.lock().expect("progress lock");
    let due = progress.all_srs()?;
    let seen = progress.seen_ids()?;
    drop(progress);
    let content = state.content.lock().expect("content lock");
    let pool: Vec<i64> = content
        .sentences_by_level(level)?
        .iter()
        .map(|s| s.id)
        .filter(|id| !seen.contains(id))
        .collect();
    drop(content);
    // Deterministic per local day: re-entering keeps the same queue (§4.5).
    let seed = day_index(now, tz_offset_secs) as u64;
    let mut session = sf_core::build_session(&due, &pool, spec, now, seed, daily_new);
    if let Some(force) = reorder_override {
        for item in &mut session.items {
            if !item.is_review {
                item.reorder_first = force;
            }
        }
    }
    Ok(session)
}

/// Ad-hoc session over an explicit id list (错题本一键重练 / 收藏 / 我的句集).
#[tauri::command]
pub fn start_custom_session(ids: Vec<i64>) -> CmdResult<Session> {
    Ok(Session {
        items: ids
            .into_iter()
            .map(|sentence_id| SessionItem {
                sentence_id,
                mode: Mode::Typing,
                is_review: true,
                reorder_first: false,
            })
            .collect(),
        overflow_reviews: 0,
    })
}

#[derive(Debug, Deserialize)]
pub struct AttemptReport {
    pub sentence_id: i64,
    pub mode: Mode,
    pub outcome: Outcome,
    pub dur_ms: u32,
    pub errors: u32,
    pub wpm: f32,
    pub error_tags: Vec<ErrorTag>,
    pub tz_offset_secs: i32,
}

#[derive(Debug, Serialize)]
pub struct AttemptAck {
    pub srs: SrsState,
    /// 体验模式剩余额度 (None = 不受限).
    pub lapsed_remaining: Option<u32>,
}

#[tauri::command]
pub fn submit_attempt(state: S<'_>, report: AttemptReport) -> CmdResult<AttemptAck> {
    let now = now_unix();
    // 体验模式每日 5 句硬上限 (§4.6) — checked before recording new work.
    let license = licensing::current_state(&state.paths, now)?;
    let mut lapsed_remaining = None;
    if let LicenseState::Lapsed { daily_limit, .. } = &license {
        let day_start =
            (day_index(now, report.tz_offset_secs)) * 86_400 - i64::from(report.tz_offset_secs);
        let progress = state.progress.lock().expect("progress lock");
        let used = progress.attempts_since(day_start)?;
        let already_counted = progress.get_srs(report.sentence_id)?.is_some_and(|s| {
            day_index(s.last_at, report.tz_offset_secs) == day_index(now, report.tz_offset_secs)
        });
        if used >= *daily_limit && !already_counted {
            return Err(CmdError::new("trial_limit", "体验模式每日 5 句已用完"));
        }
        lapsed_remaining = Some(daily_limit.saturating_sub(used));
    }

    let sentence_level = {
        let content = state.content.lock().expect("content lock");
        content
            .sentence_by_id(report.sentence_id)?
            .map(|s| s.level)
            .ok_or_else(|| CmdError::new("content", "句子不存在"))?
    };
    let spec = state.spec_for(sentence_level)?;

    let progress = state.progress.lock().expect("progress lock");
    let prev = progress
        .get_srs(report.sentence_id)?
        .unwrap_or_else(|| SrsState::new(now));
    let next = sf_core::apply_outcome(prev, report.outcome, report.mode, spec, now);
    progress.upsert_srs(report.sentence_id, &next)?;
    let (result, seen_answer) = match report.outcome {
        Outcome::Correct { seen_answer } => (LogResult::Correct, seen_answer),
        Outcome::Wrong | Outcome::MarkUnfamiliar => (LogResult::Wrong, false),
        Outcome::MarkMastered => (LogResult::Correct, false),
    };
    progress.insert_log(&LogRow {
        ts: now,
        sentence_id: report.sentence_id,
        mode: report.mode,
        result,
        dur_ms: report.dur_ms,
        errors: report.errors,
        wpm: report.wpm,
        seen_answer,
        error_tags: report.error_tags,
    })?;
    Ok(AttemptAck {
        srs: next,
        lapsed_remaining,
    })
}

#[tauri::command]
pub fn judge_text(state: S<'_>, sentence_id: i64, input: String) -> CmdResult<Verdict> {
    let content = state.content.lock().expect("content lock");
    let sentence = content
        .sentence_by_id(sentence_id)?
        .ok_or_else(|| CmdError::new("content", "句子不存在"))?;
    drop(content);
    let targets = sentence.target_words();
    let target_refs: Vec<&str> = if targets.is_empty() {
        // Imported sentences without annotations: judge against tokenization.
        return Ok(sf_core::judge(
            &input,
            &sf_pipeline::validate::tokenize_en(&sentence.en)
                .0
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            &JudgePolicy::default(),
        ));
    } else {
        targets
    };
    Ok(sf_core::judge(
        &input,
        &target_refs,
        &JudgePolicy::default(),
    ))
}

// ------------------------------------------------------------- collections

#[tauri::command]
pub fn wrongbook(state: S<'_>) -> CmdResult<Vec<i64>> {
    let progress = state.progress.lock().expect("progress lock");
    progress.wrongbook_ids()
}

#[tauri::command]
pub fn favorites(state: S<'_>) -> CmdResult<Vec<i64>> {
    let progress = state.progress.lock().expect("progress lock");
    progress.favorite_ids()
}

#[tauri::command]
pub fn favorite_toggle(state: S<'_>, sentence_id: i64, on: bool) -> CmdResult<()> {
    let progress = state.progress.lock().expect("progress lock");
    if on {
        progress.favorite_add(sentence_id, now_unix())
    } else {
        progress.favorite_remove(sentence_id)
    }
}

// ------------------------------------------------------------- stats

#[tauri::command]
pub fn get_stats(state: S<'_>, tz_offset_secs: i32) -> CmdResult<StatsSummary> {
    let progress = state.progress.lock().expect("progress lock");
    let logs = progress.all_logs()?;
    drop(progress);
    Ok(sf_core::fold_stats(logs, tz_offset_secs))
}

// ------------------------------------------------------------- trial import

#[derive(Debug, Deserialize)]
pub struct TrialExport {
    pub version: u32,
    pub items: Vec<TrialItem>,
}

#[derive(Debug, Deserialize)]
pub struct TrialItem {
    pub en: String,
    pub srs: SrsState,
}

/// Import web-trial progress (§7.9 试用进度带得走). Matching is by exact `en`
/// text against the factory library; unmatched rows are skipped and counted.
#[tauri::command]
pub fn import_trial_progress(state: S<'_>, export: TrialExport) -> CmdResult<(u32, u32)> {
    if export.version != 1 {
        return Err(CmdError::new("import", "试用导出文件版本过新"));
    }
    // Resolve ids first (content lock), then merge (progress lock) — never
    // hold both locks at once.
    let resolved: Vec<(Option<i64>, SrsState)> = {
        let content = state.content.lock().expect("content lock");
        export
            .items
            .iter()
            .map(|item| Ok((content.factory.sentence_id_by_en(&item.en)?, item.srs)))
            .collect::<CmdResult<_>>()?
    };
    let progress = state.progress.lock().expect("progress lock");
    let mut merged = 0u32;
    let mut skipped = 0u32;
    for (id, srs) in resolved {
        match id {
            Some(id) => {
                // 按 last_at 取新 (§4.8 数据恢复口径).
                let keep_incoming = progress
                    .get_srs(id)?
                    .is_none_or(|existing| srs.last_at > existing.last_at);
                if keep_incoming {
                    progress.upsert_srs(id, &srs)?;
                }
                merged += 1;
            }
            None => skipped += 1,
        }
    }
    Ok((merged, skipped))
}

// ------------------------------------------------------------- channels

#[tauri::command]
pub async fn probe_channel(state: S<'_>, channel: ChannelId) -> CmdResult<ChannelStatus> {
    Ok(channels::probe(&state, channel).await)
}

#[tauri::command]
pub async fn test_channel_key(
    state: S<'_>,
    channel: ChannelId,
    key: String,
) -> CmdResult<ChannelStatus> {
    channels::test_and_store_key(&state, channel, key.into()).await
}

#[tauri::command]
pub fn clear_channel_key(channel: ChannelId) -> CmdResult<()> {
    sf_llm::keystore::delete_key(channel).map_err(|e| CmdError::new("keystore", e.to_string()))
}

#[derive(Debug, Serialize)]
pub struct SpendSummary {
    pub today_requests: u32,
    pub today_cost: f64,
    pub month_requests: u32,
    pub month_cost: f64,
}

#[tauri::command]
pub fn spend_summary(state: S<'_>, tz_offset_secs: i32) -> CmdResult<SpendSummary> {
    let now = now_unix();
    let day_start = day_index(now, tz_offset_secs) * 86_400 - i64::from(tz_offset_secs);
    // 月度提醒用近 30 天滚动窗口(无服务器,无账期概念).
    let month_start = now - 30 * 86_400;
    let progress = state.progress.lock().expect("progress lock");
    let (today_requests, today_cost) = progress.spend_since(day_start)?;
    let (month_requests, month_cost) = progress.spend_since(month_start)?;
    Ok(SpendSummary {
        today_requests,
        today_cost,
        month_requests,
        month_cost,
    })
}

// ------------------------------------------------------------- bench (§3.5)

/// 能力微基准:每个候选免费模型生成 6 句 → 本地校验打分 → 排序入库。
/// 名单指纹随结果存 progress.db,名单变化自动重测(前端比对指纹后调用)。
#[tauri::command]
pub async fn run_bench(app: AppHandle, state: S<'_>) -> CmdResult<Vec<sf_llm::bench::BenchScore>> {
    use futures::StreamExt;
    use sf_llm::bench::{BenchSample, rank};
    use sf_llm::types::GenChunk;
    use sf_pipeline::validate::{DedupeIndex, Validator, VerdictKind};

    let (channel, has_proxy) = {
        let settings = state.settings.lock().expect("settings lock");
        (
            settings
                .ai
                .channel
                .ok_or_else(|| CmdError::new("no_channel", "未配置 AI 通道"))?,
            settings
                .ai
                .proxy_url
                .as_deref()
                .is_some_and(|p| !p.trim().is_empty()),
        )
    };
    let adapter = channels::make_adapter(&state, channel, None)?;
    let ChannelStatus::Ready { models } = channels::probe(&state, channel).await else {
        return Err(CmdError::new("channel", "通道未就绪,无法运行微基准"));
    };
    // 未配代理时跳过"需代理"模型:直连必失败,评测既浪费额度又拉低体验
    // (名单见 channels.json proxy_required_models,面向国内用户)。
    let candidates: Vec<_> = models
        .into_iter()
        .filter(|m| has_proxy || !m.needs_proxy)
        .take(6)
        .collect();
    if candidates.is_empty() {
        return Err(CmdError::new(
            "channel",
            "没有可评测的模型(需代理的模型已跳过)",
        ));
    }
    let fingerprint = sf_pipeline::simhash::fingerprint16(sf_pipeline::simhash::simhash64(
        &candidates
            .iter()
            .map(|m| m.id.as_str())
            .collect::<Vec<_>>()
            .join(","),
    ));

    let level = {
        let settings = state.settings.lock().expect("settings lock");
        settings.level.unwrap_or(LevelId::L3)
    };
    let spec = state.spec_for(level)?.clone();
    let prompt = sf_pipeline::prompt::build_prompt(&spec, "日常寒暄与自我介绍", 6, &[]);

    let mut samples = Vec::new();
    for m in &candidates {
        let _ = app.emit("bench://progress", serde_json::json!({ "model": m.id }));
        let started = std::time::Instant::now();
        let req = sf_llm::types::GenRequest {
            model: m.id.clone(),
            system: prompt.system.clone(),
            user: prompt.user.clone(),
            max_tokens: Some(4096),
            temperature: Some(0.7),
        };
        let mut text = String::new();
        let ok = match adapter.complete_stream(req).await {
            Ok(mut stream) => loop {
                match stream.next().await {
                    Some(Ok(GenChunk::Text { text: t })) => text.push_str(&t),
                    Some(Ok(GenChunk::Done)) | None => break true,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break false,
                }
            },
            Err(_) => false,
        };
        let latency_ms = started.elapsed().as_millis() as u64;

        let (parsed, passed, over_level) = if ok {
            match sf_pipeline::parse::parse_drafts(&text) {
                Ok(batch) => {
                    let validator = Validator::new(&spec, &state.lexicon);
                    let dedupe = DedupeIndex::default();
                    let mut passed = 0u32;
                    let mut over = 0u32;
                    for d in &batch.drafts {
                        match validator.validate(d, "bench", "", &dedupe).verdict {
                            VerdictKind::Pass => passed += 1,
                            VerdictKind::OverLevel => over += 1,
                            _ => {}
                        }
                    }
                    (batch.drafts.len() as u32, passed, over)
                }
                Err(_) => (0, 0, 0),
            }
        } else {
            (0, 0, 0)
        };
        samples.push(BenchSample {
            model: m.id.clone(),
            requested: 6,
            parsed,
            passed,
            over_level,
            latency_ms,
        });
    }

    let ranking = rank(&samples);
    {
        let progress = state.progress.lock().expect("progress lock");
        let now = now_unix();
        for score in &ranking {
            progress.save_bench(
                &score.model,
                f64::from(score.score),
                score.latency_ms as i64,
                &fingerprint,
                now,
            )?;
        }
    }
    // 默认选最高分 (§3.5).
    if let Some(best) = ranking.first() {
        let mut settings = state.settings.lock().expect("settings lock").clone();
        settings.ai.model = Some(best.model.clone());
        state.save_settings(&settings)?;
        *state.settings.lock().expect("settings lock") = settings;
    }
    Ok(ranking)
}

/// Stored bench ranking for a model-list fingerprint — the frontend compares
/// the current list's fingerprint to decide whether a re-test is needed
/// (名单变化自动重测, §3.5).
#[tauri::command]
pub fn bench_ranking(state: S<'_>, fingerprint: String) -> CmdResult<Vec<(String, f64)>> {
    let progress = state.progress.lock().expect("progress lock");
    progress.bench_ranking(&fingerprint)
}

// ------------------------------------------------------------- workshop

#[tauri::command]
pub async fn workshop_start(app: AppHandle, state: S<'_>, params: JobParams) -> CmdResult<u64> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let _ = workshop::run_new_job(app, state, params).await;
    });
    Ok(0)
}

#[tauri::command]
pub fn workshop_stop(state: S<'_>) -> CmdResult<()> {
    state.request_gen_cancel();
    Ok(())
}

#[tauri::command]
pub async fn workshop_resume(app: AppHandle, state: S<'_>, job_id: u64) -> CmdResult<()> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        let _ = workshop::resume_job(app, state, job_id).await;
    });
    Ok(())
}

#[tauri::command]
pub fn workshop_jobs(state: S<'_>) -> CmdResult<Vec<GenJob>> {
    let progress = state.progress.lock().expect("progress lock");
    progress.load_jobs()
}

/// 丢弃句捞回 (§4.4): store a recoverable card into the user library.
#[tauri::command]
pub fn workshop_recover(state: S<'_>, sentence: Sentence) -> CmdResult<i64> {
    let content = state.content.lock().expect("content lock");
    let Some(user) = &content.user else {
        return Err(CmdError::new("content", "用户句库未初始化"));
    };
    let rid = user.insert_sentence(&sentence, "", 1)?;
    Ok(sf_pipeline::store::ContentIndex::USER_ID_OFFSET + rid)
}

// ------------------------------------------------------------- ask AI / weekly

/// 一轮已完成的问答(前端随新问题回传,用于多轮上下文)。
#[derive(Debug, Deserialize)]
pub struct AskTurn {
    pub q: String,
    pub a: String,
}

#[tauri::command]
pub async fn ask_ai(
    app: AppHandle,
    state: S<'_>,
    sentence_id: i64,
    question: String,
    history: Vec<AskTurn>,
) -> CmdResult<()> {
    let (channel, model) = {
        let settings = state.settings.lock().expect("settings lock");
        (
            settings
                .ai
                .channel
                .ok_or_else(|| CmdError::new("no_channel", "未配置 AI 通道"))?,
            settings.ai.model.clone().unwrap_or_default(),
        )
    };
    let sentence = {
        let content = state.content.lock().expect("content lock");
        content
            .sentence_by_id(sentence_id)?
            .ok_or_else(|| CmdError::new("content", "句子不存在"))?
    };
    let adapter = channels::make_adapter(&state, channel, None)?;
    // 近 3 轮上下文随问题送出(限量控 token,§1.4 花钱必透明)
    let mut user = format!("句子:{}\n中文:{}\n", sentence.en, sentence.zh);
    for turn in history.iter().rev().take(3).rev() {
        user.push_str(&format!("此前问:{}\n此前答:{}\n", turn.q, turn.a));
    }
    user.push_str(&format!("问题:{question}"));
    let req = sf_llm::types::GenRequest {
        model,
        system: "你是英语老师,用简体中文简明回答学习者针对给定英文句子的问题。\
                 可用 Markdown(加粗、列表)组织要点。不超过 200 字。"
            .into(),
        user,
        max_tokens: Some(1024),
        temperature: Some(0.3),
    };
    let state2 = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        use futures::StreamExt;
        match adapter.complete_stream(req).await {
            Ok(mut stream) => {
                while let Some(chunk) = stream.next().await {
                    match chunk {
                        Ok(sf_llm::types::GenChunk::Text { text }) => {
                            let _ = app.emit("ask://chunk", text);
                        }
                        Ok(sf_llm::types::GenChunk::Usage {
                            prompt_tokens,
                            completion_tokens,
                        }) => {
                            let progress = state2.progress.lock().expect("progress lock");
                            let _ = progress.spend_add(
                                now_unix(),
                                "ask",
                                prompt_tokens,
                                completion_tokens,
                                0.0,
                            );
                        }
                        Ok(sf_llm::types::GenChunk::Done) => break,
                        Err(e) => {
                            let _ = app.emit("ask://error", e.zh_message());
                            return;
                        }
                    }
                }
                let _ = app.emit("ask://done", ());
            }
            Err(e) => {
                let _ = app.emit("ask://error", e.zh_message());
            }
        }
    });
    Ok(())
}

/// 每周 AI 学习点评 (§4.4): only aggregates leave the machine, never句子内容.
#[tauri::command]
pub async fn weekly_review(state: S<'_>, tz_offset_secs: i32) -> CmdResult<String> {
    let (channel, model) = {
        let settings = state.settings.lock().expect("settings lock");
        (
            settings
                .ai
                .channel
                .ok_or_else(|| CmdError::new("no_channel", "未配置 AI 通道"))?,
            settings.ai.model.clone().unwrap_or_default(),
        )
    };
    let stats = {
        let progress = state.progress.lock().expect("progress lock");
        let logs = progress.all_logs()?;
        sf_core::fold_stats(logs, tz_offset_secs)
    };
    let week_cut = now_unix() - 7 * 86_400;
    let week_days: Vec<_> = stats
        .days
        .iter()
        .filter(|(day, _)| **day * 86_400 >= week_cut)
        .map(|(_, d)| d)
        .collect();
    let attempts: u32 = week_days.iter().map(|d| d.attempts).sum();
    let correct: u32 = week_days.iter().map(|d| d.correct).sum();
    let weak = stats
        .weak_pos
        .first()
        .map(|w| format!("{}(错误占比 {:.0}%)", w.tag.zh_name(), w.share * 100.0))
        .unwrap_or_else(|| "无明显薄弱项".into());
    let adapter = channels::make_adapter(&state, channel, None)?;
    let req = sf_llm::types::GenRequest {
        model,
        system: "你是英语学习教练。根据本周聚合数据,用简体中文写 200 字以内的点评加三条具体建议。"
            .into(),
        user: format!(
            "本周练习 {attempts} 次,正确 {correct} 次,连续打卡 {} 天,最薄弱:{weak}。",
            stats.streak_days
        ),
        max_tokens: Some(1024),
        temperature: Some(0.5),
    };
    use futures::StreamExt;
    let mut stream = adapter
        .complete_stream(req)
        .await
        .map_err(|e| CmdError::new("channel", e.zh_message()))?;
    let mut out = String::new();
    while let Some(chunk) = stream.next().await {
        match chunk {
            Ok(sf_llm::types::GenChunk::Text { text }) => out.push_str(&text),
            Ok(sf_llm::types::GenChunk::Done) => break,
            Ok(_) => {}
            Err(e) => return Err(CmdError::new("channel", e.zh_message())),
        }
    }
    let progress = state.progress.lock().expect("progress lock");
    progress.kv_set("weekly_review", &out)?;
    Ok(out)
}

// ------------------------------------------------------------- backup

/// Backup zip = progress.db + user_content.db, 不含任何密钥 (§4.8).
/// Relative destinations resolve into the app data dir — the exe's folder may
/// not be writable (Program Files).
#[tauri::command]
pub fn backup_export(state: S<'_>, dest: String) -> CmdResult<String> {
    use std::io::Write;
    let dest = std::path::PathBuf::from(dest);
    let dest = if dest.is_absolute() {
        dest
    } else {
        state.paths.root.join(dest)
    };
    let file = std::fs::File::create(&dest)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, path) in [
        ("progress.db", state.paths.progress_db()),
        ("user_content.db", state.paths.user_content_db()),
    ] {
        if path.exists() {
            zip.start_file(name, options)
                .map_err(|e| CmdError::new("backup", e.to_string()))?;
            zip.write_all(&std::fs::read(&path)?)?;
        }
    }
    zip.finish()
        .map_err(|e| CmdError::new("backup", e.to_string()))?;
    Ok(dest.to_string_lossy().into_owned())
}

#[derive(Debug, Serialize)]
pub struct RestorePreview {
    pub srs_incoming: u32,
    pub srs_newer: u32,
    pub logs_incoming: u32,
}

/// Restore preview + merge (差异预览按 last_at 取新, §4.8 / §6.4).
/// `apply: false` only reports what would happen.
#[tauri::command]
pub fn backup_restore(state: S<'_>, src: String, apply: bool) -> CmdResult<RestorePreview> {
    let file = std::fs::File::open(&src)?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| CmdError::new("backup", e.to_string()))?;
    let tmp = state.paths.root.join("restore-tmp.db");
    {
        use std::io::Read;
        let mut entry = archive
            .by_name("progress.db")
            .map_err(|_| CmdError::new("backup", "备份包里没有 progress.db"))?;
        let mut buf = Vec::new();
        entry.read_to_end(&mut buf)?;
        std::fs::write(&tmp, buf)?;
    }
    let incoming = ProgressDb::open(&tmp)?;
    let incoming_srs = incoming.all_srs()?;
    let incoming_logs = incoming.all_logs()?;
    let progress = state.progress.lock().expect("progress lock");
    let mut newer = 0u32;
    for (id, s) in &incoming_srs {
        let keep = progress
            .get_srs(*id)?
            .is_none_or(|cur| s.last_at > cur.last_at);
        if keep {
            newer += 1;
            if apply {
                progress.upsert_srs(*id, s)?;
            }
        }
    }
    if apply {
        for log in &incoming_logs {
            progress.insert_log(log)?;
        }
    }
    drop(progress);
    let _ = std::fs::remove_file(&tmp);
    Ok(RestorePreview {
        srs_incoming: incoming_srs.len() as u32,
        srs_newer: newer,
        logs_incoming: incoming_logs.len() as u32,
    })
}

// ------------------------------------------------------------- tts

/// Offline TTS: returns a wav path when the piper pack is installed, `None`
/// otherwise (frontend falls back to speechSynthesis).
#[tauri::command]
pub async fn tts_speak(
    state: S<'_>,
    text: String,
    us_accent: bool,
    rate: f32,
) -> CmdResult<Option<String>> {
    match &state.tts {
        Some(t) => {
            let path = t.speak(&text, us_accent, rate).await?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
        None => Ok(None),
    }
}

// ------------------------------------------------------------- diagnostics

/// 匿名诊断包 (§4.8): version info + counts, no content, no keys.
#[tauri::command]
pub fn diagnostics(state: S<'_>) -> CmdResult<serde_json::Value> {
    let progress = state.progress.lock().expect("progress lock");
    let logs = progress.all_logs()?;
    let content = state.content.lock().expect("content lock");
    Ok(serde_json::json!({
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "content_rev": content.factory.get_meta("rev")?,
        "sentence_count": content.factory.sentence_count()?,
        "log_count": logs.len(),
        "policy_rev": state.policy.rev,
    }))
}
