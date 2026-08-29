//! 阶段四·运动源（§3.4）：内置 walk/run/jump/wave 侧视步态剪辑。
//!
//! 架构同方案（§3.4、附录C）：剪辑是**紧凑 JSON 关键帧**，运行期零解析依赖、可热更/换入
//! 真 CMU BVH 预处理产物。本次内置剪辑为**参数化侧视步态**（E48 许可下的合法生成法）：
//! 每帧给出 8 关节相对 rest 的偏移（按骨架高度归一，尺度无关）；bake 施加 offset·height。
//! 走/跑区分（§3.4）：run 步频高、幅度大、躯干前倾、双足离地相；jump 五段（一次性）。

use serde::{Deserialize, Serialize};

use super::V2;

/// 8 关节偏移帧，顺序同 Joints::as_array（head/neck/shL/shR/hip/knL/knR/ankle）。
pub type Pose = [V2; 8];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    #[serde(rename = "loop")]
    pub looping: bool,
    /// 每循环步幅（按骨架高度归一；供速度-步频锁定，非运动=0）。
    pub stride: f64,
    pub frames: Vec<Pose>,
}

impl Clip {
    /// 在 phase∈[0,1) 采样（loop 环绕，once 钳制），线性插值关键帧。
    pub fn pose(&self, phase: f64) -> Pose {
        let n = self.frames.len();
        if n == 0 {
            return [V2::default(); 8];
        }
        if n == 1 {
            return self.frames[0];
        }
        let x = if self.looping {
            (phase.rem_euclid(1.0)) * n as f64
        } else {
            phase.clamp(0.0, 1.0) * (n - 1) as f64
        };
        let i0 = (x.floor() as usize).min(n - 1);
        let i1 = if self.looping {
            (i0 + 1) % n
        } else {
            (i0 + 1).min(n - 1)
        };
        let t = x - x.floor();
        let mut out = [V2::default(); 8];
        for (k, o) in out.iter_mut().enumerate() {
            *o = self.frames[i0][k]
                .scale(1.0 - t)
                .add(self.frames[i1][k].scale(t));
        }
        out
    }
}

const K: usize = 16; // 每剪辑关键帧数

fn sample_clip(looping: bool, stride: f64, f: impl Fn(f64) -> Pose) -> Clip {
    let frames = (0..K).map(|i| f(i as f64 / K as f64)).collect();
    Clip {
        looping,
        stride,
        frames,
    }
}

/// 内置剪辑（参数化侧视步态）。未知名回退 wave。
pub fn builtin(name: &str) -> Clip {
    match name {
        "walk" => sample_clip(true, 0.5, walk_pose),
        "run" => sample_clip(true, 0.85, run_pose),
        "jump" => sample_clip(false, 0.0, jump_pose),
        _ => sample_clip(true, 0.0, wave_pose),
    }
}

const TAU: f64 = std::f64::consts::TAU;
fn v(x: f64, y: f64) -> V2 {
    V2::new(x, y)
}

/// 走：双腿相位差 0.5 交替迈步、身体二次触地起伏、手臂反向摆。
fn walk_pose(p: f64) -> Pose {
    let a = TAU * p;
    let hip_bob = -0.012 + 0.012 * (2.0 * a).cos(); // 一循环两次起伏
    let head = v(0.0, hip_bob * 0.8);
    let neck = v(0.0, hip_bob * 0.9);
    let sh_l = v(-0.06 * a.sin(), hip_bob); // 反向于左腿
    let sh_r = v(-0.06 * (a + TAU * 0.5).sin(), hip_bob);
    let hip = v(0.0, hip_bob);
    let kn_l = v(0.12 * a.sin(), hip_bob - 0.05 * a.sin().max(0.0));
    let kn_r = v(
        0.12 * (a + TAU * 0.5).sin(),
        hip_bob - 0.05 * (a + TAU * 0.5).sin().max(0.0),
    );
    let ankle = v(0.05 * a.sin(), 0.0);
    [head, neck, sh_l, sh_r, hip, kn_l, kn_r, ankle]
}

/// 跑：走的强化——幅度更大、抬腿更高、躯干前倾、身体整体起落更深（含离地相）。
fn run_pose(p: f64) -> Pose {
    let a = TAU * p;
    let lean = 0.045; // 躯干前倾（上身整体前移）
    let hip_bob = -0.02 + 0.03 * (2.0 * a).cos();
    let head = v(lean * 1.2, hip_bob * 0.8);
    let neck = v(lean, hip_bob * 0.9);
    let sh_l = v(lean - 0.10 * a.sin(), hip_bob);
    let sh_r = v(lean - 0.10 * (a + TAU * 0.5).sin(), hip_bob);
    let hip = v(lean * 0.5, hip_bob);
    let kn_l = v(0.18 * a.sin(), hip_bob - 0.10 * a.sin().max(0.0));
    let kn_r = v(
        0.18 * (a + TAU * 0.5).sin(),
        hip_bob - 0.10 * (a + TAU * 0.5).sin().max(0.0),
    );
    let ankle = v(0.09 * a.sin(), -0.02 * (2.0 * a).sin().abs());
    [head, neck, sh_l, sh_r, hip, kn_l, kn_r, ankle]
}

/// 跳：一次性五段——下蹲→蹬伸→滞空→落地→回落。手臂上摆。
fn jump_pose(p: f64) -> Pose {
    // 身体竖直位移曲线（负=上升）
    let body = if p < 0.15 {
        0.06 * (p / 0.15) // 下蹲
    } else if p < 0.30 {
        0.06 - 0.16 * ((p - 0.15) / 0.15) // 蹬伸
    } else if p < 0.70 {
        let k = (p - 0.30) / 0.40;
        -0.10 - 0.10 * (std::f64::consts::PI * k).sin() // 滞空
    } else if p < 0.82 {
        -0.10 + 0.13 * ((p - 0.70) / 0.12) // 落地
    } else {
        0.03 * (1.0 - (p - 0.82) / 0.18) // 回落归位
    };
    let crouch = (0.05_f64).max(0.0)
        * (if p < 0.15 {
            p / 0.15
        } else {
            (0.30 - p).max(0.0) / 0.15
        });
    let arms = if (0.15..0.55).contains(&p) {
        -0.10
    } else {
        0.0
    }; // 蹬伸-滞空手臂上摆
    let head = v(0.0, body);
    let neck = v(0.0, body);
    let sh_l = v(0.0, body + arms);
    let sh_r = v(0.0, body + arms);
    let hip = v(0.0, body);
    let kn_l = v(0.0, body + crouch);
    let kn_r = v(0.0, body + crouch);
    let ankle = v(0.0, (body + crouch * 0.5).min(0.0));
    [head, neck, sh_l, sh_r, hip, kn_l, kn_r, ankle]
}

/// 招手：右臂上举并左右摆，其余轻微呼吸。
fn wave_pose(p: f64) -> Pose {
    let a = TAU * p;
    let breath = -0.006 * (2.0 * a).sin();
    let wave_x = 0.05 * (a * 2.0).sin();
    let head = v(0.0, breath);
    let neck = v(0.0, breath);
    let sh_l = v(0.0, breath);
    let sh_r = v(wave_x, -0.14);
    let hip = v(0.0, breath * 0.5);
    let kn_l = v(0.0, 0.0);
    let kn_r = v(0.0, 0.0);
    let ankle = v(0.0, 0.0);
    [head, neck, sh_l, sh_r, hip, kn_l, kn_r, ankle]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_loops_continuously() {
        let c = builtin("walk");
        let p0 = c.pose(0.0);
        let p1 = c.pose(0.999);
        for k in 0..8 {
            assert!(
                p0[k].dist(p1[k]) < 0.05,
                "walk loop discontinuity at joint {k}"
            );
        }
        assert!(c.stride > 0.0 && c.looping);
    }

    #[test]
    fn run_bigger_than_walk() {
        let (w, r) = (builtin("walk"), builtin("run"));
        // 同相位下 run 的膝盖摆幅应更大
        let wl = w.pose(0.25)[5].x.abs();
        let rl = r.pose(0.25)[5].x.abs();
        assert!(rl > wl, "run knee {rl} should exceed walk {wl}");
        assert!(r.stride > w.stride);
    }

    #[test]
    fn jump_is_once_and_airborne() {
        let c = builtin("jump");
        assert!(!c.looping && c.stride == 0.0);
        // 滞空中段身体应显著上移（dy 负）
        let mid = c.pose(0.5)[4].y;
        assert!(mid < -0.08, "jump apex dy {mid}");
    }

    #[test]
    fn clip_json_round_trip() {
        let c = builtin("walk");
        let json = serde_json::to_string(&c).unwrap();
        let back: Clip = serde_json::from_str(&json).unwrap();
        assert_eq!(back.frames.len(), c.frames.len());
        assert!((back.stride - c.stride).abs() < 1e-9);
        assert_eq!(back.pose(0.3)[5], c.pose(0.3)[5]);
    }
}
