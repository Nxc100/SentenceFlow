//! Licensing + trial orchestration (spec §4.6, §7.6).
//!
//! * License: `.sflic` verified against the embedded public key, stored as a
//!   plain file (换机 = 拷文件, §1.3).
//! * Trial: 14-day anchor persisted twice — a JSON file and an OS-keyring
//!   entry; on read the two are merged pessimistically (earliest start,
//!   latest observed clock) so deleting one of them doesn't reset the trial.
//!   Keyring absence degrades to file-only, by design (§4.6: 不做更深对抗).

use crate::error::{CmdError, CmdResult};
use crate::paths::AppPaths;
use serde::{Deserialize, Serialize};
use sf_license::trial::{LAPSED_DAILY_SENTENCES, TrialState, TrialVerdict};
use sf_license::{LicensePayload, verify};

/// This build's product major version (license `major_max` gate).
pub const APP_MAJOR: u32 = 1;

/// DEV public key — replaced with the production key before release
/// (`sf-license keygen`; private half lives offline, §7.6). The matching dev
/// private key sits in `tools/dev-keys/` so activation is testable end-to-end.
pub const LICENSE_PUBLIC_KEY_B64: &str = "fTCckVC7QsQqP+nM6wu8PhVfTafuBgON8vf0+fv0eSM=";

const TRIAL_KEYRING_SERVICE: &str = "sentenceflow";
const TRIAL_KEYRING_ACCOUNT: &str = "trial-anchor";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LicenseState {
    Licensed {
        email_masked: String,
        edition: String,
        major_max: u32,
    },
    Trial {
        days_left: u32,
    },
    /// 体验模式 (每日 5 句). `clock_rollback` drives the 授权页说明 (§6.4).
    Lapsed {
        clock_rollback: bool,
        daily_limit: u32,
    },
}

fn read_keyring_anchor() -> Option<TrialState> {
    let entry = keyring::Entry::new(TRIAL_KEYRING_SERVICE, TRIAL_KEYRING_ACCOUNT).ok()?;
    let raw = entry.get_password().ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_keyring_anchor(state: &TrialState) {
    if let Ok(entry) = keyring::Entry::new(TRIAL_KEYRING_SERVICE, TRIAL_KEYRING_ACCOUNT)
        && let Ok(raw) = serde_json::to_string(state)
    {
        let _ = entry.set_password(&raw);
    }
}

fn read_file_anchor(paths: &AppPaths) -> Option<TrialState> {
    let raw = std::fs::read_to_string(paths.trial_file()).ok()?;
    serde_json::from_str(&raw).ok()
}

fn write_file_anchor(paths: &AppPaths, state: &TrialState) -> CmdResult<()> {
    std::fs::write(paths.trial_file(), serde_json::to_string(state)?)?;
    Ok(())
}

/// Pessimistic merge of the two anchor copies.
fn merge(a: TrialState, b: TrialState) -> TrialState {
    TrialState {
        started_at: a.started_at.min(b.started_at),
        last_seen: a.last_seen.max(b.last_seen),
        rolled_back: a.rolled_back || b.rolled_back,
    }
}

/// Load (or initialize) the trial anchor, advance it to `now`, persist both
/// copies, and return the verdict.
pub fn advance_trial(paths: &AppPaths, now: i64) -> CmdResult<TrialVerdict> {
    let anchor = match (read_file_anchor(paths), read_keyring_anchor()) {
        (Some(f), Some(k)) => merge(f, k),
        (Some(one), None) | (None, Some(one)) => one,
        (None, None) => TrialState::start(now),
    };
    let (next, verdict) = anchor.advance(now);
    write_file_anchor(paths, &next)?;
    write_keyring_anchor(&next);
    Ok(verdict)
}

/// Verify + persist a pasted/dropped license document.
pub fn activate(paths: &AppPaths, sflic_json: &str) -> CmdResult<LicensePayload> {
    let key = sf_license::parse_public_key(LICENSE_PUBLIC_KEY_B64)
        .map_err(|e| CmdError::internal(format!("embedded key invalid: {e}")))?;
    let payload = verify(sflic_json, &key).map_err(|e| CmdError::new("license", e.to_string()))?;
    if !payload.permits_major(APP_MAJOR) {
        return Err(CmdError::new(
            "license",
            format!(
                "此许可证覆盖到 v{}.x,当前版本为 v{APP_MAJOR}.x,需要升级许可证",
                payload.major_max
            ),
        ));
    }
    std::fs::write(paths.license_file(), sflic_json.trim())?;
    Ok(payload)
}

/// Current licensing state, advancing the trial clock as a side effect.
pub fn current_state(paths: &AppPaths, now: i64) -> CmdResult<LicenseState> {
    if let Ok(raw) = std::fs::read_to_string(paths.license_file())
        && let Ok(key) = sf_license::parse_public_key(LICENSE_PUBLIC_KEY_B64)
        && let Ok(payload) = verify(&raw, &key)
        && payload.permits_major(APP_MAJOR)
    {
        return Ok(LicenseState::Licensed {
            email_masked: payload.masked_email(),
            edition: payload.edition,
            major_max: payload.major_max,
        });
    }
    match advance_trial(paths, now)? {
        TrialVerdict::Active { days_left } => Ok(LicenseState::Trial { days_left }),
        TrialVerdict::Expired => Ok(LicenseState::Lapsed {
            clock_rollback: false,
            daily_limit: LAPSED_DAILY_SENTENCES,
        }),
        TrialVerdict::ExpiredClockRollback => Ok(LicenseState::Lapsed {
            clock_rollback: true,
            daily_limit: LAPSED_DAILY_SENTENCES,
        }),
    }
}

/// Export a copy of the stored license (授权卡 [导出备份], §5.5).
pub fn export_license(paths: &AppPaths) -> CmdResult<String> {
    std::fs::read_to_string(paths.license_file())
        .map_err(|_| CmdError::new("license", "尚未激活,没有可导出的许可证"))
}
