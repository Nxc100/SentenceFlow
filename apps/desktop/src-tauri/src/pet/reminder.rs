//! 提醒系统:一次性日程、循环提醒、番茄钟。
//!
//! 由 [`super::heartbeat`] 的 1s 心跳驱动;到点走「系统通知 + `pet://reminder`
//! 事件(宠物播 remind 动画 + 气泡)」双通道。
//!
//! 与 HatchDesk 的差异:状态不再是独立的 tauri `State`,而是 [`ReminderStore`]
//! 收在 `PetState` 里;落盘路径由调用方给。

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::{DateTime, Duration, Local};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;

use crate::error::{CmdError, CmdResult};

// ---------------------------------------------------------------- 数据模型

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reminder {
    pub id: String,
    pub title: String,
    /// "once" | "every"
    pub kind: String,
    /// once:RFC3339 触发时刻
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    /// every:间隔分钟
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u32>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fired: Option<String>,
    pub created: String,
}

/// 触发事件载荷(广播给宠物窗与「AI 萌宠」页)。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FiredEvent {
    /// "once" | "every" | "pomodoro" | "snooze"
    pub kind: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PomodoroCfg {
    pub work_min: u32,
    pub break_min: u32,
    pub rounds: u32,
}

impl Default for PomodoroCfg {
    fn default() -> Self {
        PomodoroCfg {
            work_min: 25,
            break_min: 5,
            rounds: 4,
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PomodoroStatus {
    pub running: bool,
    /// "work" | "break" | "idle"
    pub phase: String,
    pub round: u32,
    pub rounds: u32,
    pub remaining_secs: i64,
    pub total_secs: i64,
}

struct PomodoroInner {
    cfg: PomodoroCfg,
    running: bool,
    phase: String,
    round: u32,
    phase_end: DateTime<Local>,
}

impl Default for PomodoroInner {
    fn default() -> Self {
        PomodoroInner {
            cfg: PomodoroCfg::default(),
            running: false,
            phase: "idle".into(),
            round: 0,
            phase_end: Local::now(),
        }
    }
}

/// 稍后提醒的一条排队项:(触发时刻, 标题, 原提醒 id)。
type Snooze = (DateTime<Local>, String, Option<String>);

/// 提醒仓库。三把独立的小锁,临界区都只覆盖一次读改写,不跨 await。
pub struct ReminderStore {
    file: PathBuf,
    reminders: Mutex<Vec<Reminder>>,
    /// 稍后提醒队列(不持久化:重启后不该再蹦出上次的「5 分钟后再提醒」)。
    snoozes: Mutex<Vec<Snooze>>,
    pomodoro: Mutex<PomodoroInner>,
}

impl ReminderStore {
    pub fn load(file: PathBuf) -> Self {
        let reminders = sf_pet::read_json_text(&file)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        ReminderStore {
            file,
            reminders: Mutex::new(reminders),
            snoozes: Mutex::new(Vec::new()),
            pomodoro: Mutex::new(PomodoroInner::default()),
        }
    }

    fn persist(&self, list: &[Reminder]) -> CmdResult<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&self.file, serde_json::to_string_pretty(list)?)?;
        Ok(())
    }

    pub fn list(&self) -> Vec<Reminder> {
        self.reminders.lock().expect("pet reminders lock").clone()
    }

    pub fn create(
        &self,
        title: String,
        kind: String,
        at: Option<String>,
        minutes: Option<u32>,
    ) -> CmdResult<Reminder> {
        let reminder = build_reminder(title, kind, at, minutes, Local::now())?;
        let snapshot = {
            let mut guard = self.reminders.lock().expect("pet reminders lock");
            guard.push(reminder.clone());
            guard.clone()
        };
        self.persist(&snapshot)?;
        Ok(reminder)
    }

    pub fn toggle(&self, id: &str, enabled: bool) -> CmdResult<()> {
        let snapshot = {
            let mut guard = self.reminders.lock().expect("pet reminders lock");
            if let Some(r) = guard.iter_mut().find(|r| r.id == id) {
                r.enabled = enabled;
                // 重新启用一次性提醒时清掉已触发标记
                if enabled && r.kind == "once" {
                    r.last_fired = None;
                }
            }
            guard.clone()
        };
        self.persist(&snapshot)
    }

    pub fn delete(&self, id: &str) -> CmdResult<()> {
        let snapshot = {
            let mut guard = self.reminders.lock().expect("pet reminders lock");
            guard.retain(|r| r.id != id);
            guard.clone()
        };
        self.persist(&snapshot)
    }

    /// 稍后提醒:m 分钟后以同标题再次触发(内存队列,不改原提醒节奏)。
    pub fn snooze(&self, title: String, reminder_id: Option<String>, minutes: u32) {
        let at = Local::now() + Duration::minutes(minutes.clamp(1, 120) as i64);
        self.snoozes
            .lock()
            .expect("pet snoozes lock")
            .push((at, title, reminder_id));
    }

    pub fn pomodoro_start(&self, cfg: PomodoroCfg) -> PomodoroStatus {
        let mut p = self.pomodoro.lock().expect("pet pomodoro lock");
        let cfg = PomodoroCfg {
            work_min: cfg.work_min.clamp(1, 120),
            break_min: cfg.break_min.clamp(1, 60),
            rounds: cfg.rounds.clamp(1, 12),
        };
        p.cfg = cfg;
        p.running = true;
        p.phase = "work".into();
        p.round = 1;
        p.phase_end = Local::now() + Duration::minutes(cfg.work_min as i64);
        status_of(&p)
    }

    pub fn pomodoro_stop(&self) -> PomodoroStatus {
        let mut p = self.pomodoro.lock().expect("pet pomodoro lock");
        let cfg = p.cfg;
        *p = PomodoroInner {
            cfg,
            ..PomodoroInner::default()
        };
        status_of(&p)
    }

    pub fn pomodoro_status(&self) -> PomodoroStatus {
        status_of(&self.pomodoro.lock().expect("pet pomodoro lock"))
    }
}

/// 组装一条提醒(纯函数:校验 + 归一化时间,`now` 注入以便测试)。
fn build_reminder(
    title: String,
    kind: String,
    at: Option<String>,
    minutes: Option<u32>,
    now: DateTime<Local>,
) -> CmdResult<Reminder> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(err("提醒内容不能为空"));
    }
    match kind.as_str() {
        "once" => {
            let t = parse_time(at.as_deref().ok_or_else(|| err("缺少提醒时间"))?)?;
            if t <= now {
                return Err(err("提醒时间必须在未来"));
            }
        }
        "every" => {
            let m = minutes.ok_or_else(|| err("缺少间隔分钟数"))?;
            if !(1..=24 * 60).contains(&m) {
                return Err(err("间隔需在 1–1440 分钟之间"));
            }
        }
        _ => return Err(err("未知提醒类型")),
    }
    Ok(Reminder {
        id: format!("r{}", now.format("%Y%m%d%H%M%S%3f")),
        title,
        kind,
        at: at
            .map(|s| parse_time(&s).map(|t| t.to_rfc3339()))
            .transpose()?,
        minutes,
        enabled: true,
        last_fired: None,
        created: now.to_rfc3339(),
    })
}

/// 接受 RFC3339 或 `<input type="datetime-local">` 的 `%Y-%m-%dT%H:%M[:%S]`。
fn parse_time(s: &str) -> CmdResult<DateTime<Local>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Local));
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            if let Some(dt) = naive.and_local_timezone(Local).single() {
                return Ok(dt);
            }
        }
    }
    Err(err(format!("无法解析时间: {s}")))
}

fn status_of(p: &PomodoroInner) -> PomodoroStatus {
    let total = match p.phase.as_str() {
        "work" => p.cfg.work_min as i64 * 60,
        "break" => p.cfg.break_min as i64 * 60,
        _ => 0,
    };
    PomodoroStatus {
        running: p.running,
        phase: p.phase.clone(),
        round: p.round,
        rounds: p.cfg.rounds,
        remaining_secs: if p.running {
            (p.phase_end - Local::now()).num_seconds().max(0)
        } else {
            0
        },
        total_secs: total,
    }
}

fn err(msg: impl Into<String>) -> CmdError {
    CmdError::new("pet", msg)
}

// ---------------------------------------------------------------- 心跳

/// 触发:系统通知 + `pet://reminder`(宠物播 remind + 气泡)。
fn fire(app: &AppHandle, event: FiredEvent) {
    let _ = app
        .notification()
        .builder()
        .title("句流 · AI 萌宠")
        .body(&event.title)
        .show();
    let _ = app.emit(super::EV_REMINDER, event);
}

/// 每秒调用一次(见 [`super::heartbeat`])。
pub fn tick(app: &AppHandle, store: &ReminderStore) {
    let now = Local::now();

    // —— 普通提醒 ——
    let mut fired: Vec<FiredEvent> = Vec::new();
    let mut dirty = false;
    let snapshot = {
        let mut guard = store.reminders.lock().expect("pet reminders lock");
        for r in guard.iter_mut() {
            if !r.enabled {
                continue;
            }
            match r.kind.as_str() {
                "once" => {
                    if r.last_fired.is_some() {
                        continue;
                    }
                    let due =
                        r.at.as_deref()
                            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                            .is_some_and(|at| now >= at.with_timezone(&Local));
                    if due {
                        r.last_fired = Some(now.to_rfc3339());
                        r.enabled = false;
                        dirty = true;
                        fired.push(FiredEvent {
                            kind: "once".into(),
                            title: r.title.clone(),
                            reminder_id: Some(r.id.clone()),
                        });
                    }
                }
                "every" => {
                    let Some(m) = r.minutes else { continue };
                    let anchor = r
                        .last_fired
                        .as_deref()
                        .or(Some(r.created.as_str()))
                        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                        .map(|t| t.with_timezone(&Local))
                        .unwrap_or(now);
                    if now >= anchor + Duration::minutes(m as i64) {
                        r.last_fired = Some(now.to_rfc3339());
                        dirty = true;
                        fired.push(FiredEvent {
                            kind: "every".into(),
                            title: r.title.clone(),
                            reminder_id: Some(r.id.clone()),
                        });
                    }
                }
                _ => {}
            }
        }
        dirty.then(|| guard.clone())
    };
    if let Some(list) = snapshot {
        let _ = store.persist(&list);
    }

    // —— 稍后提醒队列 ——
    {
        let mut snoozes = store.snoozes.lock().expect("pet snoozes lock");
        let due: Vec<_> = snoozes
            .iter()
            .filter(|(at, ..)| now >= *at)
            .cloned()
            .collect();
        snoozes.retain(|(at, ..)| now < *at);
        drop(snoozes);
        for (_, title, rid) in due {
            fired.push(FiredEvent {
                kind: "snooze".into(),
                title,
                reminder_id: rid,
            });
        }
    }

    for event in fired {
        fire(app, event);
    }

    // —— 番茄钟 ——
    let transition = pomodoro_tick(app, store, now);
    if let Some(event) = transition {
        fire(app, event);
    }
}

/// 番茄钟推进一格。返回需要播报的相位切换事件。
fn pomodoro_tick(
    app: &AppHandle,
    store: &ReminderStore,
    now: DateTime<Local>,
) -> Option<FiredEvent> {
    let mut p = store.pomodoro.lock().expect("pet pomodoro lock");
    if !p.running {
        return None;
    }
    if now < p.phase_end {
        let status = status_of(&p);
        drop(p);
        let _ = app.emit(super::EV_POMODORO, status);
        return None;
    }
    let event = if p.phase == "work" {
        if p.round >= p.cfg.rounds {
            let done = p.round;
            let cfg = p.cfg;
            *p = PomodoroInner {
                cfg,
                ..PomodoroInner::default()
            };
            FiredEvent {
                kind: "pomodoro".into(),
                title: format!("🍅 {done} 轮番茄全部完成,好好休息!"),
                reminder_id: None,
            }
        } else {
            p.phase = "break".into();
            p.phase_end = now + Duration::minutes(p.cfg.break_min as i64);
            FiredEvent {
                kind: "pomodoro".into(),
                title: "🍅 专注结束,起来伸个懒腰吧".into(),
                reminder_id: None,
            }
        }
    } else {
        p.phase = "work".into();
        p.round += 1;
        p.phase_end = now + Duration::minutes(p.cfg.work_min as i64);
        FiredEvent {
            kind: "pomodoro".into(),
            title: "🍅 休息结束,开始下一轮专注".into(),
            reminder_id: None,
        }
    };
    let status = status_of(&p);
    drop(p);
    let _ = app.emit(super::EV_POMODORO, status);
    Some(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now() -> DateTime<Local> {
        Local::now()
    }

    #[test]
    fn rejects_invalid_reminders() {
        let t = now();
        let future = (t + Duration::hours(1)).to_rfc3339();
        assert!(build_reminder("  ".into(), "once".into(), Some(future.clone()), None, t).is_err());
        assert!(build_reminder("喝水".into(), "weekly".into(), None, None, t).is_err());
        // 一次性提醒缺时间 / 时间在过去
        assert!(build_reminder("喝水".into(), "once".into(), None, None, t).is_err());
        let past = (t - Duration::hours(1)).to_rfc3339();
        assert!(build_reminder("喝水".into(), "once".into(), Some(past), None, t).is_err());
        // 循环提醒的间隔越界
        assert!(build_reminder("喝水".into(), "every".into(), None, Some(0), t).is_err());
        assert!(build_reminder("喝水".into(), "every".into(), None, Some(1441), t).is_err());
    }

    #[test]
    fn accepts_datetime_local_from_the_browser_input() {
        // <input type="datetime-local"> 给的是没有时区后缀的本地时间
        let t = now();
        let local = (t + Duration::hours(2))
            .format("%Y-%m-%dT%H:%M")
            .to_string();
        let r = build_reminder("背单词".into(), "once".into(), Some(local), None, t).unwrap();
        assert!(r.enabled && r.last_fired.is_none());
        assert!(
            DateTime::parse_from_rfc3339(r.at.as_deref().unwrap()).is_ok(),
            "落盘的时间统一归一化成 RFC3339"
        );
        assert!(r.id.starts_with('r'));
    }

    /// 标题两端空白要吃掉,但中间的内容原样保留。
    #[test]
    fn trims_title() {
        let t = now();
        let r =
            build_reminder("  该 练 句 了  ".into(), "every".into(), None, Some(30), t).unwrap();
        assert_eq!(r.title, "该 练 句 了");
        assert_eq!(r.minutes, Some(30));
    }

    /// CRUD 往返:落盘后重新 load 应拿到同一份清单。
    #[test]
    fn crud_round_trips_through_disk() {
        let dir = std::env::temp_dir().join("sf-pet-reminders-crud");
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("reminders.json");

        let store = ReminderStore::load(file.clone());
        assert!(store.list().is_empty(), "文件不存在时应是空清单");
        let r = store
            .create("喝水".into(), "every".into(), None, Some(45))
            .unwrap();
        assert_eq!(store.list().len(), 1);

        store.toggle(&r.id, false).unwrap();
        assert!(!ReminderStore::load(file.clone()).list()[0].enabled);

        store.delete(&r.id).unwrap();
        assert!(ReminderStore::load(file).list().is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 番茄钟配置必须钳位:界面之外还有 IPC 直呼,
    /// `rounds: u32::MAX` 会让「第 x/y 轮」文案与进度盘直接失真。
    #[test]
    fn pomodoro_config_is_clamped() {
        let store = ReminderStore::load(
            std::env::temp_dir().join("sf-pet-reminders-pomodoro/reminders.json"),
        );
        let s = store.pomodoro_start(PomodoroCfg {
            work_min: 9999,
            break_min: 0,
            rounds: 99,
        });
        assert!(s.running && s.phase == "work" && s.round == 1);
        assert_eq!(s.rounds, 12);
        assert_eq!(s.total_secs, 120 * 60);

        let stopped = store.pomodoro_stop();
        assert!(!stopped.running && stopped.phase == "idle");
        assert_eq!(stopped.remaining_secs, 0);
    }

    /// 稍后提醒的分钟数同样钳位(1–120)。
    #[test]
    fn snooze_is_queued_in_memory_only() {
        let dir = std::env::temp_dir().join("sf-pet-reminders-snooze");
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("reminders.json");
        let store = ReminderStore::load(file.clone());
        store.snooze("回来练句".into(), None, 5);
        assert_eq!(store.snoozes.lock().unwrap().len(), 1);
        assert!(!file.exists(), "稍后提醒不落盘");
        std::fs::remove_dir_all(&dir).ok();
    }
}
