//! 阶段三·ARAP 重姿态（§3.3 论证2/3，L2/L3）：As-Rigid-As-Possible 局部-全局迭代。
//! 骨骼摆出目标姿势 → 网格以「尽可能刚性」的能量最小化跟随，天然带挤压拉伸生命感。
//!
//! 能量 E = Σ_i Σ_{j∈N(i)} |(p'_i-p'_j) - R_i(rest_i-rest_j)|²  +  Σ_i c_i|p'_i - t_i|²
//!  - 局部步：逐顶点求最优旋转 R_i（2×2 闭式角，无需 SVD 库）
//!  - 全局步：解 (L+C) p' = rhs（图 Laplacian + 约束），手写稠密 Cholesky 分解一次、多相位复用
//!
//! 数百顶点稠密分解即可（§1.1），确定性可测：刚体位姿为零能量→应被精确复现（旋转刚性测试）。

use super::V2;
use super::mesh::Mesh;

/// 稠密 SPD Cholesky（下三角 L，M = L·Lᵀ），行主序。
struct Cholesky {
    n: usize,
    l: Vec<f64>,
}

impl Cholesky {
    fn factor(m: &[f64], n: usize) -> Option<Cholesky> {
        let mut l = vec![0.0f64; n * n];
        for i in 0..n {
            for j in 0..=i {
                let mut sum = m[i * n + j];
                for k in 0..j {
                    sum -= l[i * n + k] * l[j * n + k];
                }
                if i == j {
                    if sum <= 1e-12 {
                        return None; // 非正定
                    }
                    l[i * n + j] = sum.sqrt();
                } else {
                    l[i * n + j] = sum / l[j * n + j];
                }
            }
        }
        Some(Cholesky { n, l })
    }

    /// 解 M x = b（前代 + 回代）。
    // 三角回代天生是下标算法：`l[i*n+k]` 与 `y[k]` 的配对就是公式本身。
    #[allow(clippy::needless_range_loop)]
    fn solve(&self, b: &[f64]) -> Vec<f64> {
        let n = self.n;
        let mut y = vec![0.0f64; n];
        for i in 0..n {
            let mut sum = b[i];
            for k in 0..i {
                sum -= self.l[i * n + k] * y[k];
            }
            y[i] = sum / self.l[i * n + i];
        }
        let mut x = vec![0.0f64; n];
        for i in (0..n).rev() {
            let mut sum = y[i];
            for k in i + 1..n {
                sum -= self.l[k * n + i] * x[k];
            }
            x[i] = sum / self.l[i * n + i];
        }
        x
    }
}

pub struct Arap {
    n: usize,
    rest: Vec<V2>,
    neighbors: Vec<Vec<usize>>,
    chol: Cholesky,
    c: Vec<f64>,
}

impl Arap {
    /// 预分解。handles：强约束顶点（骨骼手柄）；handle_w 大、soft_w 小（全顶点软约束到 LBS 目标）。
    pub fn new(mesh: &Mesh, handles: &[usize], handle_w: f64, soft_w: f64) -> Option<Arap> {
        let n = mesh.verts.len();
        if n == 0 {
            return None;
        }
        // 邻接（去重）
        let mut nb: Vec<std::collections::BTreeSet<usize>> = vec![Default::default(); n];
        for t in &mesh.tris {
            for a in 0..3 {
                for b in 0..3 {
                    if a != b {
                        nb[t[a]].insert(t[b]);
                    }
                }
            }
        }
        let neighbors: Vec<Vec<usize>> = nb.iter().map(|s| s.iter().copied().collect()).collect();

        // c 向量
        let mut c = vec![soft_w; n];
        for &h in handles {
            if h < n {
                c[h] = handle_w;
            }
        }
        // M = L + diag(c)，L 图 Laplacian（w_ij=1）
        let mut m = vec![0.0f64; n * n];
        for i in 0..n {
            m[i * n + i] = neighbors[i].len() as f64 + c[i];
            for &j in &neighbors[i] {
                m[i * n + j] -= 1.0;
            }
        }
        let chol = Cholesky::factor(&m, n)?;
        Some(Arap {
            n,
            rest: mesh.verts.clone(),
            neighbors,
            chol,
            c,
        })
    }

    /// 求解形变后顶点。targets：每顶点目标（LBS 结果，全顶点软约束）；iters：局部-全局迭代次数。
    pub fn solve(&self, targets: &[V2], iters: usize) -> Vec<V2> {
        let n = self.n;
        let mut p = targets.to_vec(); // LBS 暖启动
        for _ in 0..iters.max(1) {
            // —— 局部步：每顶点最优旋转角 ——
            let mut theta = vec![0.0f64; n];
            for i in 0..n {
                let (mut s00, mut s01, mut s10, mut s11) = (0.0, 0.0, 0.0, 0.0);
                for &j in &self.neighbors[i] {
                    let e = self.rest[i].sub(self.rest[j]); // rest edge
                    let dp = p[i].sub(p[j]); // current edge
                    s00 += e.x * dp.x;
                    s01 += e.x * dp.y;
                    s10 += e.y * dp.x;
                    s11 += e.y * dp.y;
                }
                // 最优旋转最大化 trace(R·S)：θ = atan2(S01-S10, S00+S11)（2×2 闭式）
                theta[i] = (s01 - s10).atan2(s00 + s11);
            }
            // —— 全局步：rhs_i = c_i·t_i + Σ_j 0.5(R_i+R_j)(rest_i-rest_j) ——
            let mut bx = vec![0.0f64; n];
            let mut by = vec![0.0f64; n];
            for i in 0..n {
                let (ci, si) = (theta[i].cos(), theta[i].sin());
                let mut rx = self.c[i] * targets[i].x;
                let mut ry = self.c[i] * targets[i].y;
                for &j in &self.neighbors[i] {
                    let e = self.rest[i].sub(self.rest[j]);
                    let (cj, sj) = (theta[j].cos(), theta[j].sin());
                    // 0.5(R_i+R_j)·e
                    let ex = 0.5 * ((ci * e.x - si * e.y) + (cj * e.x - sj * e.y));
                    let ey = 0.5 * ((si * e.x + ci * e.y) + (sj * e.x + cj * e.y));
                    rx += ex;
                    ry += ey;
                }
                bx[i] = rx;
                by[i] = ry;
            }
            let nx = self.chol.solve(&bx);
            let ny = self.chol.solve(&by);
            for i in 0..n {
                p[i] = V2::new(nx[i], ny[i]);
            }
        }
        p
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Joints, mesh};
    use super::*;

    fn square_mesh() -> Mesh {
        let mask = vec![true; 60 * 60];
        mesh::build(&mask, 60, 60, 300)
    }

    fn arap_for(m: &Mesh) -> Arap {
        let j = Joints::heuristic(0.0, 0.0, 60.0, 60.0);
        let skel = mesh::skeleton(&j);
        let handles = mesh::handle_verts(m, &skel);
        Arap::new(m, &handles, 1000.0, 0.05).unwrap()
    }

    #[test]
    fn identity_returns_rest() {
        let m = square_mesh();
        let a = arap_for(&m);
        let out = a.solve(&m.verts, 3);
        for (o, r) in out.iter().zip(m.verts.iter()) {
            assert!(o.dist(*r) < 1e-4, "identity drift {:?} vs {:?}", o, r);
        }
    }

    #[test]
    fn pure_translation_follows() {
        let m = square_mesh();
        let a = arap_for(&m);
        let shift = V2::new(12.0, -7.0);
        let targets: Vec<V2> = m.verts.iter().map(|v| v.add(shift)).collect();
        let out = a.solve(&targets, 3);
        for (o, r) in out.iter().zip(m.verts.iter()) {
            assert!(o.dist(r.add(shift)) < 1e-3);
        }
    }

    #[test]
    fn pure_rotation_is_rigid() {
        // 刚体旋转是零能量位姿——ARAP 应精确复现（关键正确性测试）。
        let m = square_mesh();
        let a = arap_for(&m);
        let ang = 0.5_f64; // ~28.6°
        let (c, s) = (ang.cos(), ang.sin());
        let pivot = V2::new(30.0, 30.0);
        let rot = |v: V2| {
            let d = v.sub(pivot);
            V2::new(pivot.x + c * d.x - s * d.y, pivot.y + s * d.x + c * d.y)
        };
        let targets: Vec<V2> = m.verts.iter().map(|&v| rot(v)).collect();
        let out = a.solve(&targets, 6);
        let mut max_err = 0.0f64;
        for (o, &v) in out.iter().zip(m.verts.iter()) {
            max_err = max_err.max(o.dist(rot(v)));
        }
        assert!(max_err < 0.5, "rigid rotation max err {max_err}");
    }
}
