//! 本地零生成动画引擎·级三骨骼烘焙器（《本地零生成动画引擎技术方案》v1.1）。
//!
//! 架构（§1）：**离线烘焙器**，不是运行时渲染器。产物是标准 PetKit 精灵条
//! （idle 呼吸 pingpong / walk / run / jump），在孵化/进化时烘焙一次，运行时仍是
//! 现有 8–12fps 精灵播放器 + 生命感叠加层——与平台生成路线的产物完全同构。
//!
//! 六阶段管线（§3）：
//! ```text
//! 图片 → ①预处理(已由环节七抠图) → ②结构理解(认主/启发关节)
//!      → ③网格与绑定(轮廓→三角化→骨骼→ARAP) → ④运动源(参数化侧视步态)
//!      → ⑤面部与次级(呼吸曲线) → ⑥烘焙(相位采样→软光栅→质心对齐→compose)
//! ```
//!
//! 工程取舍（本次落地）：**零新增依赖，全部手写并单元测试**——轮廓(marching squares)、
//! 网格(mask 网格三角化)、ARAP(局部-全局 + 手写 Cholesky)、软光栅、运动(参数化侧视步态
//! JSON)。方案 §3.6 指定软光栅"手写零依赖"；无头构建无法做人肉盲评，故**确定性单测**是
//! 唯一可靠质量闸门，手写几何完全可测，而不可验证的 BVH 重定向/ONNX 反而是质量风险。
//! 参数化步态是 E48"标注可用任何方式生成"许可下的合法生成法；`tools/bvh_prep/` 仍是
//! 日后换入真 CMU BVH 的入口。

pub mod arap;
pub mod bake;
pub mod contour;
pub mod interp;
pub mod mesh;
pub mod motion;
pub mod raster;

/// 2D 向量（引擎内部几何一律 f64）。
#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct V2 {
    pub x: f64,
    pub y: f64,
}

// 几何内核统一用具名方法链（`a.sub(b).scale(k).add(c)`），比混用
// `+`/`-` 与 `.scale()` 更容易读出运算顺序；`add`/`sub` 与 `std::ops` 同名属有意为之。
#[allow(clippy::should_implement_trait)]
impl V2 {
    pub fn new(x: f64, y: f64) -> Self {
        V2 { x, y }
    }
    pub fn add(self, o: V2) -> V2 {
        V2::new(self.x + o.x, self.y + o.y)
    }
    pub fn sub(self, o: V2) -> V2 {
        V2::new(self.x - o.x, self.y - o.y)
    }
    pub fn scale(self, s: f64) -> V2 {
        V2::new(self.x * s, self.y * s)
    }
    pub fn dot(self, o: V2) -> f64 {
        self.x * o.x + self.y * o.y
    }
    pub fn len(self) -> f64 {
        self.dot(self).sqrt()
    }
    pub fn dist(self, o: V2) -> f64 {
        self.sub(o).len()
    }
}

/// 8 关节（认主仪式 §14.2 / 附录 A）：缺省由包围盒启发式填充。
#[derive(Clone, Copy, Debug)]
pub struct Joints {
    pub head: V2,
    pub neck: V2,
    pub shoulder_l: V2,
    pub shoulder_r: V2,
    pub hip: V2,
    pub knee_l: V2,
    pub knee_r: V2,
    pub ankle: V2,
}

impl Joints {
    /// 按不透明包围盒的比例启发式推关节（无认主时的兜底，§3.2 通道 A）。
    /// bbox：(x0,y0,w,h)，图像像素坐标。
    pub fn heuristic(x0: f64, y0: f64, w: f64, h: f64) -> Self {
        let cx = x0 + w * 0.5;
        let y = |f: f64| y0 + h * f;
        Joints {
            head: V2::new(cx, y(0.10)),
            neck: V2::new(cx, y(0.28)),
            shoulder_l: V2::new(cx - w * 0.18, y(0.30)),
            shoulder_r: V2::new(cx + w * 0.18, y(0.30)),
            hip: V2::new(cx, y(0.60)),
            knee_l: V2::new(cx - w * 0.10, y(0.80)),
            knee_r: V2::new(cx + w * 0.10, y(0.80)),
            ankle: V2::new(cx, y(0.98)),
        }
    }

    /// 有序列表（关节嵌入与手柄映射用）。
    pub fn as_array(&self) -> [(&'static str, V2); 8] {
        [
            ("head", self.head),
            ("neck", self.neck),
            ("shoulder_l", self.shoulder_l),
            ("shoulder_r", self.shoulder_r),
            ("hip", self.hip),
            ("knee_l", self.knee_l),
            ("knee_r", self.knee_r),
            ("ankle", self.ankle),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_ops() {
        let a = V2::new(3.0, 4.0);
        assert!((a.len() - 5.0).abs() < 1e-9);
        assert_eq!(a.add(V2::new(1.0, 1.0)), V2::new(4.0, 5.0));
        assert!((a.dist(V2::new(0.0, 0.0)) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn heuristic_joints_within_bbox() {
        let j = Joints::heuristic(10.0, 20.0, 100.0, 200.0);
        for (_, p) in j.as_array() {
            assert!(p.x >= 10.0 && p.x <= 110.0);
            assert!(p.y >= 20.0 && p.y <= 220.0);
        }
        assert!(j.head.y < j.hip.y && j.hip.y < j.ankle.y);
    }
}
