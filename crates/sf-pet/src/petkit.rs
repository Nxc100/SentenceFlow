//! PetKit v0.1 —— 开放宠物素材包规范。
//!
//! 图集几何：8 列 × 8 行，单元格 192×208，成品 1536×1664。
//! 状态契约：8 个生活化状态，行号固定；缺失状态由运行时降级补齐。
//!
//! 落盘与打包分别见 [`crate::store`] 与 [`crate::archive`]。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const SPEC_VERSION: &str = "petkit/0.1";
pub const CELL_W: u32 = 192;
pub const CELL_H: u32 = 208;
pub const GRID_COLS: u32 = 8;
pub const GRID_ROWS: u32 = 8;
pub const SHEET_W: u32 = CELL_W * GRID_COLS; // 1536
pub const SHEET_H: u32 = CELL_H * GRID_ROWS; // 1664
pub const MAX_FRAMES: u32 = GRID_COLS;
/// 素材规范推荐绿幕色（§5.1）；打包后的 sheet 已是透明底，故 pet.json 通常不再携带。
#[allow(dead_code)]
pub const CHROMA_GREEN: &str = "#00FF00";

/// 8 状态契约（方案 §5.2），行号即枚举顺序。
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StateId {
    Idle,
    WalkRight,
    WalkLeft,
    Happy,
    Eat,
    Sleep,
    Remind,
    Drag,
}

impl StateId {
    pub const ALL: [StateId; 8] = [
        StateId::Idle,
        StateId::WalkRight,
        StateId::WalkLeft,
        StateId::Happy,
        StateId::Eat,
        StateId::Sleep,
        StateId::Remind,
        StateId::Drag,
    ];

    pub fn row(self) -> u32 {
        Self::ALL.iter().position(|s| *s == self).unwrap() as u32
    }

    pub fn key(self) -> &'static str {
        match self {
            StateId::Idle => "idle",
            StateId::WalkRight => "walk-right",
            StateId::WalkLeft => "walk-left",
            StateId::Happy => "happy",
            StateId::Eat => "eat",
            StateId::Sleep => "sleep",
            StateId::Remind => "remind",
            StateId::Drag => "drag",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|s| s.key() == key)
    }

    pub fn display_name(self) -> &'static str {
        match self {
            StateId::Idle => "待机",
            StateId::WalkRight => "右走",
            StateId::WalkLeft => "左走",
            StateId::Happy => "开心",
            StateId::Eat => "吃东西",
            StateId::Sleep => "睡觉",
            StateId::Remind => "提醒",
            StateId::Drag => "被拎起",
        }
    }

    pub fn default_fps(self) -> u32 {
        match self {
            StateId::Idle => 8,
            StateId::WalkRight | StateId::WalkLeft => 10,
            StateId::Happy => 10,
            StateId::Eat => 8,
            StateId::Sleep => 4,
            StateId::Remind => 10,
            StateId::Drag => 1,
        }
    }

    pub fn default_loop(self) -> bool {
        !matches!(self, StateId::Happy | StateId::Eat)
    }

    /// 建议帧数范围（含），用于咒语与校验提示。
    pub fn recommended_frames(self) -> (u32, u32) {
        match self {
            StateId::Idle => (4, 8),
            StateId::WalkRight | StateId::WalkLeft => (6, 8),
            StateId::Happy | StateId::Eat | StateId::Remind => (4, 6),
            StateId::Sleep => (2, 4),
            StateId::Drag => (1, 2),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CellSize {
    pub w: u32,
    pub h: u32,
}

/// 素材来源：这条动画是**真素材**还是**程序烘焙**出来的。
///
/// 运行时据此决定走路怎么画：骨骼烘焙（`rig::bake`）的 walk 是把 idle 帧扭出来的，
/// 有香蕉弯/滑步问题，故由 `cutout` 分层剪纸接管；而视频/网格导入的 walk 是真实步态，
/// 必须原样播放——否则用户辛苦生成的奔跑动画会被程序化走路悄悄顶掉。
pub const SOURCE_REAL: &str = "real";
pub const SOURCE_BAKED: &str = "baked";

/// 一条状态动画在图集中的位置与播放参数。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StateClip {
    pub row: u32,
    pub frames: u32,
    pub fps: u32,
    #[serde(rename = "loop")]
    pub looping: bool,
    #[serde(rename = "mirrorOf", skip_serializing_if = "Option::is_none")]
    pub mirror_of: Option<String>,
    /// `"real"` = 导入的真素材；`"baked"` = 程序烘焙。缺省表示旧档，
    /// 由 `store::load_spec` 按是否留有源帧回填。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// 道具锚点（改进方案 §7.1，单元格坐标系 0..192 × 0..208）。
/// 缺省时运行时按 idle 首帧包围盒推导（嘴部 ≈ bbox 上 1/3 中线，头顶 ≈ bbox 顶点）。
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Anchor {
    pub x: u32,
    pub y: u32,
}

/// pet.json（方案 §5.3）。`tier`/`anchors` 为本产品扩展字段，可缺省；
/// `template`/`head_anchor`（模板换头，V1.1）在规范中预留，见 docs/petkit-spec.md。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PetSpec {
    pub spec: String,
    pub name: String,
    #[serde(default = "default_author")]
    pub author: String,
    pub cell: CellSize,
    pub sheet: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chroma: Option<String>,
    pub states: BTreeMap<String, StateClip>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchors: Option<BTreeMap<String, Anchor>>,
}

fn default_author() -> String {
    "local".into()
}

impl PetSpec {
    pub fn new(name: &str, tier: &str) -> Self {
        PetSpec {
            spec: SPEC_VERSION.into(),
            name: name.into(),
            author: "local".into(),
            cell: CellSize {
                w: CELL_W,
                h: CELL_H,
            },
            sheet: "spritesheet.webp".into(),
            chroma: None,
            states: BTreeMap::new(),
            tier: Some(tier.into()),
            created: Some(chrono::Local::now().to_rfc3339()),
            anchors: None,
        }
    }

    pub fn clip(&self, id: StateId) -> Option<&StateClip> {
        self.states.get(id.key())
    }

    /// 推导档位：显式 tier 优先，否则按状态覆盖度推断。
    pub fn effective_tier(&self) -> String {
        if let Some(t) = &self.tier {
            return t.clone();
        }
        let n = self.states.len();
        let idle_frames = self.clip(StateId::Idle).map(|c| c.frames).unwrap_or(0);
        if n >= 6 {
            "L2".into()
        } else if idle_frames >= 2 {
            "L1".into()
        } else {
            "L0".into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_rows_match_contract() {
        assert_eq!(StateId::Idle.row(), 0);
        assert_eq!(StateId::WalkRight.row(), 1);
        assert_eq!(StateId::WalkLeft.row(), 2);
        assert_eq!(StateId::Happy.row(), 3);
        assert_eq!(StateId::Eat.row(), 4);
        assert_eq!(StateId::Sleep.row(), 5);
        assert_eq!(StateId::Remind.row(), 6);
        assert_eq!(StateId::Drag.row(), 7);
    }

    #[test]
    fn spec_json_round_trip_matches_doc_shape() {
        let mut spec = PetSpec::new("咪咪", "L2");
        spec.chroma = Some(CHROMA_GREEN.into());
        spec.states.insert(
            "idle".into(),
            StateClip {
                row: 0,
                frames: 6,
                fps: 8,
                looping: true,
                mirror_of: None,
                source: None,
            },
        );
        spec.states.insert(
            "walk-left".into(),
            StateClip {
                row: 2,
                frames: 8,
                fps: 10,
                looping: true,
                mirror_of: Some("walk-right".into()),
                source: None,
            },
        );
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json["spec"], "petkit/0.1");
        assert_eq!(json["cell"]["w"], 192);
        assert_eq!(json["states"]["idle"]["loop"], true);
        assert_eq!(json["states"]["walk-left"]["mirrorOf"], "walk-right");
        let back: PetSpec = serde_json::from_value(json).unwrap();
        assert_eq!(
            back.clip(StateId::WalkLeft).unwrap().mirror_of.as_deref(),
            Some("walk-right")
        );
    }

    #[test]
    fn tier_inference() {
        let mut spec = PetSpec::new("x", "L0");
        spec.tier = None;
        spec.states.insert(
            "idle".into(),
            StateClip {
                row: 0,
                frames: 1,
                fps: 1,
                looping: true,
                mirror_of: None,
                source: None,
            },
        );
        assert_eq!(spec.effective_tier(), "L0");
        spec.states.get_mut("idle").unwrap().frames = 6;
        assert_eq!(spec.effective_tier(), "L1");
    }
}
