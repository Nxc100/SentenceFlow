//! 阶段五+六·面部次级 + 烘焙集成（§3.5/§3.6）：把单张图 rig 后逐相位采样 →
//! ARAP 形变 → 软光栅 → 质心对齐入格 → 现有 layout::compose_sheet（零 IPC）。
//! 产物为标准 PetKit 图集 + 状态清单，交由 store 落盘、validator 校验、运行时播放。
//!
//! 状态映射（现有 8 状态契约内）：idle=呼吸 5 帧 pingpong、walk-right=侧视步态 6 帧、
//! walk-left=镜像、happy=起跳 5 帧（与运行时 happy 跳物理同构）。run/wave 剪辑已备于
//! motion.rs，待 PetKit v0.2 的 run 行接入。

use std::collections::BTreeMap;

use image::RgbaImage;

use super::arap::Arap;
use super::mesh::{self, Mesh, Skeleton};
use super::motion::{self, Pose};
use super::{Joints, V2, contour, raster};
use crate::error::{PetError, PetResult};
use crate::imaging::{ALPHA_SOLID, layout};
use crate::petkit::StateId;

const HANDLE_W: f64 = 10.0;
const SOFT_W: f64 = 2.0;
const ARAP_ITERS: usize = 3;
const TARGET_TRIS: usize = 1400;

pub struct BakedState {
    pub id: StateId,
    pub frames: u32,
    pub fps: u32,
    pub looping: bool,
    pub mirror_of: Option<String>,
}

pub struct Baked {
    pub sheet: RgbaImage,
    pub states: Vec<BakedState>,
}

/// 单图 → 烘焙动画图集。joints 为 8 关节（认主或启发式）。
pub fn bake(source: &RgbaImage, joints: &Joints) -> PetResult<Baked> {
    let (mask, w, h) = contour::solid_mask(source, ALPHA_SOLID);
    if !mask.iter().any(|&b| b) {
        return Err(PetError::msg("图像无不透明主体，无法烘焙"));
    }
    let bbox = layout::bbox(source).ok_or_else(|| PetError::msg("空图像"))?;
    let skel_h = bbox.h.max(1) as f64;

    // 轮廓提取（§3.3）：作主体有效性守卫；网格用 marching-squares 让边界贴合此轮廓。
    let poly = contour::extract(source);
    if poly.len() < 3 {
        return Err(PetError::msg("轮廓提取失败（主体过小或过碎）"));
    }
    let mesh = mesh::build(&mask, w, h, TARGET_TRIS);
    if mesh.tri_count() == 0 {
        return Err(PetError::msg("网格三角化失败（主体过小）"));
    }
    let skel = mesh::skeleton(joints);
    let handles = mesh::handle_verts(&mesh, &skel);
    let weights = mesh::skin_weights(&mesh, &skel);
    let arap = Arap::new(&mesh, &handles, HANDLE_W, SOFT_W)
        .ok_or_else(|| PetError::msg("ARAP 预分解失败（网格退化）"))?;

    // 光栅画布：留边容纳肢体外摆
    let margin = (skel_h * 0.35).round().max(24.0);
    let out_w = source.width() + (margin as u32) * 2;
    let out_h = source.height() + (margin as u32) * 2;
    let offset = V2::new(margin, margin);

    let ctx = Rig {
        source,
        mesh: &mesh,
        skel: &skel,
        weights: &weights,
        arap: &arap,
        skel_h,
        out_w,
        out_h,
        offset,
    };

    // idle：呼吸 5 帧（幅度线性上升，运行时 pingpong 播成一次完整呼吸）
    let idle: Vec<RgbaImage> = (0..5)
        .map(|i| {
            let amt = i as f64 / 4.0;
            ctx.render(&scale_pose(&breath_pose(), amt))
        })
        .collect();
    // walk-right：侧视步态 6 帧
    let walk_clip = motion::builtin("walk");
    let walk: Vec<RgbaImage> = (0..6)
        .map(|i| ctx.render(&walk_clip.pose(i as f64 / 6.0)))
        .collect();
    // happy：起跳 5 帧（一次性）
    let jump_clip = motion::builtin("jump");
    let happy: Vec<RgbaImage> = (0..5)
        .map(|i| ctx.render(&jump_clip.pose(i as f64 / 4.0)))
        .collect();

    // 统一缩放 → 入格 → 合成
    let all: Vec<&RgbaImage> = idle.iter().chain(walk.iter()).chain(happy.iter()).collect();
    let scale = layout::global_scale(&all);
    let fit = |frames: &[RgbaImage]| {
        frames
            .iter()
            .map(|f| layout::fit_into_cell(f, scale))
            .collect::<Vec<_>>()
    };
    let mut rows: BTreeMap<StateId, Vec<RgbaImage>> = BTreeMap::new();
    rows.insert(StateId::Idle, fit(&idle));
    rows.insert(StateId::WalkRight, fit(&walk));
    rows.insert(StateId::Happy, fit(&happy));
    let sheet = layout::compose_sheet(&rows);

    let states = vec![
        BakedState {
            id: StateId::Idle,
            frames: 5,
            fps: 8,
            looping: true,
            mirror_of: None,
        },
        BakedState {
            id: StateId::WalkRight,
            frames: 6,
            fps: 10,
            looping: true,
            mirror_of: None,
        },
        BakedState {
            id: StateId::WalkLeft,
            frames: 6,
            fps: 10,
            looping: true,
            mirror_of: Some("walk-right".into()),
        },
        BakedState {
            id: StateId::Happy,
            frames: 5,
            fps: 10,
            looping: false,
            mirror_of: None,
        },
    ];
    Ok(Baked { sheet, states })
}

/// rig 上下文（一次构建、多相位复用）。
struct Rig<'a> {
    source: &'a RgbaImage,
    mesh: &'a Mesh,
    skel: &'a Skeleton,
    weights: &'a [Vec<f64>],
    arap: &'a Arap,
    skel_h: f64,
    out_w: u32,
    out_h: u32,
    offset: V2,
}

impl Rig<'_> {
    /// 一个姿势偏移 → 形变帧。offset 单位为骨架高度归一。
    fn render(&self, pose: &Pose) -> RgbaImage {
        // 摆出目标关节
        let posed: Vec<V2> = self
            .skel
            .joints
            .iter()
            .enumerate()
            .map(|(k, &j)| j.add(pose[k].scale(self.skel_h)))
            .collect();
        // LBS 目标（每顶点）
        let targets: Vec<V2> = (0..self.mesh.verts.len())
            .map(|vi| {
                let v = self.mesh.verts[vi];
                let mut acc = V2::default();
                for (b, &(pi, ci)) in self.skel.bones.iter().enumerate() {
                    let s = bone_transform(
                        self.skel.joints[pi],
                        self.skel.joints[ci],
                        posed[pi],
                        posed[ci],
                        v,
                    );
                    acc = acc.add(s.scale(self.weights[vi][b]));
                }
                acc
            })
            .collect();
        let deformed = self.arap.solve(&targets, ARAP_ITERS);
        raster::rasterize(
            self.source,
            self.mesh,
            &deformed,
            self.out_w,
            self.out_h,
            self.offset,
        )
    }
}

/// 2D 骨段刚性变换：把 rest 段 (Pr→Cr) 旋到 posed 段 (Pp→Cp)，作用于点 x。
fn bone_transform(pr: V2, cr: V2, pp: V2, cp: V2, x: V2) -> V2 {
    let a_rest = (cr.y - pr.y).atan2(cr.x - pr.x);
    let a_pose = (cp.y - pp.y).atan2(cp.x - pp.x);
    let th = a_pose - a_rest;
    let (c, s) = (th.cos(), th.sin());
    let d = x.sub(pr);
    V2::new(pp.x + c * d.x - s * d.y, pp.y + s * d.x + c * d.y)
}

/// 呼吸姿势偏移（§3.5）：胸腔上抬（neck/shoulders/head 上移），髋部近静止 → 纵向体积起伏。
fn breath_pose() -> Pose {
    [
        V2::new(0.0, -0.020), // head
        V2::new(0.0, -0.018), // neck
        V2::new(0.0, -0.016), // shoulder_l
        V2::new(0.0, -0.016), // shoulder_r
        V2::new(0.0, 0.004),  // hip（腹部微沉）
        V2::new(0.0, 0.0),    // knee_l
        V2::new(0.0, 0.0),    // knee_r
        V2::new(0.0, 0.0),    // ankle
    ]
}

fn scale_pose(p: &Pose, k: f64) -> Pose {
    let mut out = [V2::default(); 8];
    for i in 0..8 {
        out[i] = p[i].scale(k);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn rect(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32) {
        for y in y0..y1 {
            for x in x0..x1 {
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    img.put_pixel(x as u32, y as u32, Rgba([210, 150, 90, 255]));
                }
            }
        }
    }

    /// 合成一个"姜饼人"：头 + 躯干 + 两腿 + 两臂，实心不透明。
    fn gingerbread() -> RgbaImage {
        let mut img = RgbaImage::from_pixel(120, 180, Rgba([0, 0, 0, 0]));
        // 头（圆）
        for y in 10..46 {
            for x in 40..80 {
                let (dx, dy) = (x as f64 - 60.0, y as f64 - 28.0);
                if dx * dx + dy * dy <= 20.0 * 20.0 {
                    img.put_pixel(x, y, Rgba([210, 150, 90, 255]));
                }
            }
        }
        rect(&mut img, 48, 46, 72, 120); // 躯干
        rect(&mut img, 30, 52, 48, 66); // 左臂
        rect(&mut img, 72, 52, 90, 66); // 右臂
        rect(&mut img, 48, 120, 60, 172); // 左腿
        rect(&mut img, 60, 120, 72, 172); // 右腿
        img
    }

    #[test]
    fn bake_produces_valid_sheet_with_motion() {
        let src = gingerbread();
        let b = layout::bbox(&src).unwrap();
        let joints = Joints::heuristic(b.x0 as f64, b.y0 as f64, b.w as f64, b.h as f64);
        let baked = bake(&src, &joints).unwrap();

        assert_eq!(baked.sheet.dimensions(), (1536, 1664));

        // idle / walk / happy 行均有内容
        let idle0 = layout::cell_at(&baked.sheet, StateId::Idle.row(), 0);
        let happy2 = layout::cell_at(&baked.sheet, StateId::Happy.row(), 2);
        assert!(layout::bbox(&idle0).is_some(), "idle empty");
        assert!(layout::bbox(&happy2).is_some(), "happy empty");
        for c in 0..6 {
            assert!(
                layout::bbox(&layout::cell_at(&baked.sheet, StateId::WalkRight.row(), c)).is_some(),
                "walk frame {c} empty"
            );
        }

        // 运动确实存在：6 帧步态中至少有一对相位帧的轮廓显著不同（摆步相位在 p≈0.17/0.83）
        let walk: Vec<RgbaImage> = (0..6)
            .map(|c| layout::cell_at(&baked.sheet, StateId::WalkRight.row(), c))
            .collect();
        let mut max_diff = 0.0f64;
        for i in 0..6 {
            for j in i + 1..6 {
                max_diff = max_diff.max(frame_diff(&walk[i], &walk[j]));
            }
        }
        assert!(
            max_diff > 0.02,
            "walk motion too small (max pairwise diff {max_diff})"
        );
        // happy 起跳中段应与 idle 中性帧不同
        assert!(frame_diff(&idle0, &happy2) > 0.01, "happy shows no jump");
        // walk-left 行未烘焙像素（运行时镜像派生）
        assert!(layout::bbox(&layout::cell_at(&baked.sheet, StateId::WalkLeft.row(), 0)).is_none());
    }

    #[test]
    fn baked_sheet_passes_existing_validator() {
        // §3.6 验收：烘焙产物必须 100% 通过现有校验器。
        let src = gingerbread();
        let b = layout::bbox(&src).unwrap();
        let joints = Joints::heuristic(b.x0 as f64, b.y0 as f64, b.w as f64, b.h as f64);
        let baked = bake(&src, &joints).unwrap();

        let mut spec = crate::petkit::PetSpec::new("测试", "L2");
        for bs in &baked.states {
            spec.states.insert(
                bs.id.key().to_string(),
                crate::petkit::StateClip {
                    row: bs.id.row(),
                    frames: bs.frames,
                    fps: bs.fps,
                    looping: bs.looping,
                    mirror_of: bs.mirror_of.clone(),
                    source: None,
                },
            );
        }
        let report = crate::validator::validate(&spec, &baked.sheet);
        let errors: Vec<_> = report
            .findings
            .iter()
            .filter(|f| f.level == crate::validator::Level::Error)
            .map(|f| f.message.clone())
            .collect();
        assert!(report.ok, "baked sheet failed validator: {errors:?}");
    }

    /// 两帧不透明像素占位差异比例。
    fn frame_diff(a: &RgbaImage, b: &RgbaImage) -> f64 {
        let mut diff = 0u64;
        let mut total = 0u64;
        for (pa, pb) in a.pixels().zip(b.pixels()) {
            let sa = pa.0[3] >= ALPHA_SOLID;
            let sb = pb.0[3] >= ALPHA_SOLID;
            if sa || sb {
                total += 1;
                if sa != sb {
                    diff += 1;
                }
            }
        }
        if total == 0 {
            0.0
        } else {
            diff as f64 / total as f64
        }
    }
}
