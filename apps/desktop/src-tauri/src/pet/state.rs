//! AI 萌宠的进程内状态。
//!
//! **锁序**(句流既有约定 `progress → content → settings` 的延伸):
//! pet 的锁永远**最后**取,绝不跨 await 持有,更不在持 pet 锁时去取句流的三把锁。
//! 实践上每个命令都是「先把需要的设置克隆出来、释放句流的锁,再动 pet 状态」。

use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;

use super::paths::PetPaths;
use super::reminder::ReminderStore;
use super::watcher::WatcherState;

/// 1s 心跳线程的把手。
///
/// 原实现是 `loop { tick; sleep }` 且永不退出、启动即跑;句流里宠物默认关闭,
/// 心跳必须能随总开关起停,退出时也要收干净。
struct Heartbeat {
    stop: std::sync::Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

pub struct PetState {
    pub paths: PetPaths,
    /// 孵化向导会话(素材暂存区,可达数十 MB)。
    pub wizard: sf_pet::wizard::WizardState,
    pub reminders: ReminderStore,
    pub watcher: WatcherState,
    /// 同一秒内连续孵化的防撞号(原实现是进程级 `static`,进程共享后收进状态)。
    seq: AtomicU32,
    heartbeat: Mutex<Option<Heartbeat>>,
}

impl PetState {
    pub fn new(app_data_root: &Path) -> Self {
        let paths = PetPaths::new(app_data_root);
        let reminders = ReminderStore::load(paths.reminders_file());
        Self {
            paths,
            wizard: sf_pet::wizard::WizardState::default(),
            reminders,
            watcher: WatcherState::default(),
            seq: AtomicU32::new(0),
            heartbeat: Mutex::new(None),
        }
    }

    /// 取下一个防撞号。
    pub fn next_seq(&self) -> u32 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    /// 起心跳线程(幂等:已在跑就什么都不做)。
    pub fn start_heartbeat(&self, app: tauri::AppHandle) {
        let mut guard = self.heartbeat.lock().expect("pet heartbeat lock");
        if guard.is_some() {
            return;
        }
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::Builder::new()
            .name("pet-heartbeat".into())
            .spawn(move || {
                while !flag.load(Ordering::SeqCst) {
                    super::heartbeat_tick(&app);
                    // 拆成 10 段轮询,停止旗标最迟 100ms 后生效(退出时不拖住进程)
                    for _ in 0..10 {
                        if flag.load(Ordering::SeqCst) {
                            return;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            });
        match handle {
            Ok(handle) => *guard = Some(Heartbeat { stop, handle }),
            Err(e) => tracing::warn!("宠物心跳线程启动失败,提醒与番茄钟将不工作: {e}"),
        }
    }

    /// 停心跳线程并等它退出。
    pub fn stop_heartbeat(&self) {
        let taken = self.heartbeat.lock().expect("pet heartbeat lock").take();
        if let Some(hb) = taken {
            hb.stop.store(true, Ordering::SeqCst);
            let _ = hb.handle.join();
        }
    }

    #[cfg(test)]
    fn heartbeat_running(&self) -> bool {
        self.heartbeat.lock().expect("pet heartbeat lock").is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_increments_for_same_second_hatches() {
        let state = PetState::new(&std::env::temp_dir().join("sf-pet-state-seq"));
        let a = state.next_seq();
        let b = state.next_seq();
        assert_ne!(a, b, "同秒两次孵化必须拿到不同的序号");
        assert_ne!(
            sf_pet::store::new_pet_id(a),
            sf_pet::store::new_pet_id(b),
            "序号不同 ⇒ 宠物 ID 不同"
        );
    }

    /// 默认不起心跳:总开关关闭时进程里不该多出一个每秒醒一次的线程。
    #[test]
    fn heartbeat_is_not_running_by_default() {
        let state = PetState::new(&std::env::temp_dir().join("sf-pet-state-hb"));
        assert!(!state.heartbeat_running());
        state.stop_heartbeat(); // 未启动时停止应是无操作
        assert!(!state.heartbeat_running());
    }
}
