//! 阶段三·网格与绑定（§3.3）：alpha mask → 网格三角化（栅格法，单连通无窟窿，
//! §3.3 论证1）→ 骨骼嵌入（8 关节连成骨链）→ 顶点蒙皮权重（骨段反距离，归一化）。
//! 手写零依赖，确定性可测。

use super::{Joints, V2};

/// 三角网格：verts 为 rest（同时作为纹理 UV，纹理即源图）；tris 索引 verts。
#[derive(Clone, Debug)]
pub struct Mesh {
    pub verts: Vec<V2>,
    pub tris: Vec<[usize; 3]>,
}

impl Mesh {
    pub fn tri_count(&self) -> usize {
        self.tris.len()
    }
}

/// 由 mask 建网格（目标三角数约 target_tris）。**Marching-squares**：内部走细栅格，
/// **边界顶点落在真实轮廓上（像素级）**——消除栅格法的多边形棱边与内缩"残缺"。
pub fn build(mask: &[bool], w: usize, h: usize, target_tris: usize) -> Mesh {
    let area = mask.iter().filter(|&&b| b).count().max(1);
    let mut step = ((2.0 * area as f64 / target_tris as f64).sqrt()).round() as usize;
    step = step.clamp(3, 40);
    loop {
        let m = build_ms(mask, w, h, step);
        if m.tris.len() >= 20 || step <= 3 {
            return m;
        }
        step = (step / 2).max(3);
    }
}

/// 顶点去重键：内部栅格节点，或某条栅格边上的轮廓交点。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum VKey {
    Node(usize, usize),  // 栅格节点 (gx,gy)
    EdgeH(usize, usize), // (gx,gy)-(gx+1,gy) 上的交点
    EdgeV(usize, usize), // (gx,gy)-(gx,gy+1) 上的交点
}

fn build_ms(mask: &[bool], w: usize, h: usize, step: usize) -> Mesh {
    let inside = |x: usize, y: usize| x < w && y < h && mask[y * w + x];
    let px = |gx: usize| (gx * step).min(w.saturating_sub(1));
    let py = |gy: usize| (gy * step).min(h.saturating_sub(1));

    let mut verts: Vec<V2> = Vec::new();
    let mut map: std::collections::HashMap<VKey, usize> = std::collections::HashMap::new();
    let mut get = |key: VKey, pos: V2, verts: &mut Vec<V2>| -> usize {
        *map.entry(key).or_insert_with(|| {
            verts.push(pos);
            verts.len() - 1
        })
    };
    // 沿一条水平/垂直栅格边求轮廓交点（内→外之间最后一个实心像素的外缘）
    let cross_h = |gxa: usize, gy: usize, dir: i32| -> V2 {
        let y = py(gy);
        let x0 = px(gxa);
        let mut last = x0 as i32;
        let mut x = x0 as i32;
        let end = (gxa as i32 + dir) * step as i32;
        while x != end && x >= 0 && (x as usize) < w {
            if inside(x as usize, y) {
                last = x;
            } else {
                break;
            }
            x += dir;
        }
        V2::new(last as f64 + dir as f64 * 0.5, y as f64)
    };
    let cross_v = |gx: usize, gya: usize, dir: i32| -> V2 {
        let x = px(gx);
        let y0 = py(gya);
        let mut last = y0 as i32;
        let mut y = y0 as i32;
        let end = (gya as i32 + dir) * step as i32;
        while y != end && y >= 0 && (y as usize) < h {
            if inside(x, y as usize) {
                last = y;
            } else {
                break;
            }
            y += dir;
        }
        V2::new(x as f64, last as f64 + dir as f64 * 0.5)
    };

    let gx_n = w / step + 1;
    let gy_n = h / step + 1;
    let mut tris: Vec<[usize; 3]> = Vec::new();

    for gy in 0..gy_n {
        for gx in 0..gx_n {
            // 四角 in/out
            let cin = [
                inside(px(gx), py(gy)),
                inside(px(gx + 1), py(gy)),
                inside(px(gx + 1), py(gy + 1)),
                inside(px(gx), py(gy + 1)),
            ];
            let n_in = cin.iter().filter(|&&b| b).count();
            if n_in == 0 {
                continue;
            }
            // 角点位置
            let corner = [
                V2::new(px(gx) as f64, py(gy) as f64),
                V2::new(px(gx + 1) as f64, py(gy) as f64),
                V2::new(px(gx + 1) as f64, py(gy + 1) as f64),
                V2::new(px(gx) as f64, py(gy + 1) as f64),
            ];
            let ckey = [
                VKey::Node(gx, gy),
                VKey::Node(gx + 1, gy),
                VKey::Node(gx + 1, gy + 1),
                VKey::Node(gx, gy + 1),
            ];
            // 绕单元格一周，收集"内部角点"和"跨边交点"，得到内部子多边形
            let mut poly: Vec<usize> = Vec::new();
            for e in 0..4 {
                let a = e;
                let b = (e + 1) % 4;
                if cin[a] {
                    poly.push(get(ckey[a], corner[a], &mut verts));
                }
                if cin[a] != cin[b] {
                    // 该边有交点：从内侧角朝外侧角求交
                    let (key, pos) = edge_cross(gx, gy, a, cin[a], &cross_h, &cross_v);
                    poly.push(get(key, pos, &mut verts));
                }
            }
            // 扇形三角化内部多边形
            for i in 1..poly.len().saturating_sub(1) {
                if poly[0] != poly[i] && poly[i] != poly[i + 1] && poly[0] != poly[i + 1] {
                    tris.push([poly[0], poly[i], poly[i + 1]]);
                }
            }
        }
    }
    Mesh { verts, tris }
}

/// 单元格第 e 条边（a=e 角 → b=(e+1)%4 角）的轮廓交点键与位置。a_inside 指 a 角是否在内。
fn edge_cross(
    gx: usize,
    gy: usize,
    e: usize,
    a_inside: bool,
    cross_h: &dyn Fn(usize, usize, i32) -> V2,
    cross_v: &dyn Fn(usize, usize, i32) -> V2,
) -> (VKey, V2) {
    match e {
        0 => {
            // 上边 (gx,gy)-(gx+1,gy)，水平；内侧决定扫描方向
            let pos = if a_inside {
                cross_h(gx, gy, 1)
            } else {
                cross_h(gx + 1, gy, -1)
            };
            (VKey::EdgeH(gx, gy), pos)
        }
        1 => {
            // 右边 (gx+1,gy)-(gx+1,gy+1)，垂直
            let pos = if a_inside {
                cross_v(gx + 1, gy, 1)
            } else {
                cross_v(gx + 1, gy + 1, -1)
            };
            (VKey::EdgeV(gx + 1, gy), pos)
        }
        2 => {
            // 下边 (gx+1,gy+1)-(gx,gy+1)，水平
            let pos = if a_inside {
                cross_h(gx + 1, gy + 1, -1)
            } else {
                cross_h(gx, gy + 1, 1)
            };
            (VKey::EdgeH(gx, gy + 1), pos)
        }
        _ => {
            // 左边 (gx,gy+1)-(gx,gy)，垂直
            let pos = if a_inside {
                cross_v(gx, gy + 1, -1)
            } else {
                cross_v(gx, gy, 1)
            };
            (VKey::EdgeV(gx, gy), pos)
        }
    }
}

/// 骨架：关节位置 + 骨（父,子 关节索引）。索引顺序同 Joints::as_array。
#[derive(Clone, Debug)]
pub struct Skeleton {
    pub joints: Vec<V2>,
    pub bones: Vec<(usize, usize)>,
}

/// 8 关节侧视骨链（§3.4）：脊柱 + 双臂短骨 + 双腿两节。
pub fn skeleton(j: &Joints) -> Skeleton {
    let joints: Vec<V2> = j.as_array().iter().map(|(_, p)| *p).collect();
    // 0 head,1 neck,2 shL,3 shR,4 hip,5 knL,6 knR,7 ankle
    let bones = vec![
        (1, 0), // neck-head
        (4, 1), // hip-neck (脊柱)
        (1, 2), // neck-shoulderL
        (1, 3), // neck-shoulderR
        (4, 5), // hip-kneeL
        (5, 7), // kneeL-ankle
        (4, 6), // hip-kneeR
        (6, 7), // kneeR-ankle
    ];
    Skeleton { joints, bones }
}

/// 每关节最近的网格顶点（ARAP 强手柄）。
pub fn handle_verts(mesh: &Mesh, skel: &Skeleton) -> Vec<usize> {
    skel.joints
        .iter()
        .map(|&j| {
            mesh.verts
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.dist(j).partial_cmp(&b.dist(j)).unwrap())
                .map(|(i, _)| i)
                .unwrap_or(0)
        })
        .collect()
}

/// 点到线段距离。
fn dist_to_segment(p: V2, a: V2, b: V2) -> f64 {
    let ab = b.sub(a);
    let l2 = ab.dot(ab);
    if l2 < 1e-9 {
        return p.dist(a);
    }
    let t = (p.sub(a).dot(ab) / l2).clamp(0.0, 1.0);
    p.dist(a.add(ab.scale(t)))
}

/// 蒙皮权重：每顶点对每根骨的反距离权重（归一化，和为 1）。
pub fn skin_weights(mesh: &Mesh, skel: &Skeleton) -> Vec<Vec<f64>> {
    mesh.verts
        .iter()
        .map(|&v| {
            let mut ws: Vec<f64> = skel
                .bones
                .iter()
                .map(|&(p, c)| {
                    let d = dist_to_segment(v, skel.joints[p], skel.joints[c]);
                    1.0 / (d * d + 4.0)
                })
                .collect();
            let sum: f64 = ws.iter().sum();
            if sum > 1e-12 {
                for x in ws.iter_mut() {
                    *x /= sum;
                }
            }
            ws
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filled_rect_mask(w: usize, h: usize) -> Vec<bool> {
        vec![true; w * h]
    }

    #[test]
    fn grid_mesh_is_connected_and_sized() {
        let (w, h) = (100, 200);
        let mask = filled_rect_mask(w, h);
        let m = build(&mask, w, h, 300);
        assert!(m.tri_count() >= 100, "got {} tris", m.tri_count());
        assert!(!m.verts.is_empty());
        // 所有三角索引合法
        for t in &m.tris {
            for &i in t {
                assert!(i < m.verts.len());
            }
        }
    }

    #[test]
    fn weights_sum_to_one() {
        let (w, h) = (100, 200);
        let mask = filled_rect_mask(w, h);
        let m = build(&mask, w, h, 300);
        let j = Joints::heuristic(0.0, 0.0, 100.0, 200.0);
        let skel = skeleton(&j);
        let weights = skin_weights(&m, &skel);
        assert_eq!(weights.len(), m.verts.len());
        for wv in &weights {
            let s: f64 = wv.iter().sum();
            assert!((s - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn handles_map_near_joints() {
        let (w, h) = (100, 200);
        let mask = filled_rect_mask(w, h);
        let m = build(&mask, w, h, 400);
        let j = Joints::heuristic(0.0, 0.0, 100.0, 200.0);
        let skel = skeleton(&j);
        let handles = handle_verts(&m, &skel);
        assert_eq!(handles.len(), 8);
        for (hi, &vi) in handles.iter().enumerate() {
            // 最近顶点应离该关节不超过一个栅格步长的量级
            assert!(m.verts[vi].dist(skel.joints[hi]) < 30.0);
        }
    }
}
