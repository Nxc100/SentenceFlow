//! 咒语包 v2（改进方案环节二）：网格版式 + 参考图锚定，豆包默认、即梦进阶、
//! ChatGPT/Gemini 可选。全部版式/角标 ROI/文案数据化在 data.json，随版本热更。

pub mod guide;

use serde::{Deserialize, Serialize};

use crate::imaging::chroma::CornerRoi;

const DATA: &str = include_str!("data.json");

// ---------------------------------------------------------------- 原始数据

#[derive(Clone, Deserialize)]
struct RawPlatform {
    id: String,
    name: String,
    role: String,
    url: String,
    canvas_line: String,
    grid_cols: u32,
    english: bool,
    suffix: String,
    tips: Vec<String>,
}

#[derive(Clone, Deserialize)]
struct RawState {
    id: String,
    name: String,
    frames: u32,
    action: String,
    grid_desc: String,
    grid_desc_en: String,
    note: String,
}

#[derive(Clone, Deserialize)]
struct RawPair {
    id: String,
    primary: bool,
    free: bool,
    states: Vec<String>,
    title: String,
    desc: String,
}

#[derive(Clone, Deserialize)]
struct RawVideoPlatform {
    label: String,
    setup: String,
    idle: String,
    walk: String,
}

#[derive(Clone, Deserialize)]
struct RawVideoSegment {
    state: String,
    seconds: f32,
    frames: u32,
    desc: String,
}

#[derive(Clone, Deserialize)]
struct RawVideoPlan {
    id: String,
    title: String,
    desc: String,
    free: bool,
    segments: Vec<RawVideoSegment>,
}

/// 一次生成、多状态点亮的脚本化视频路线。
#[derive(Clone, Deserialize)]
struct RawVideoFull {
    setup: String,
    rules: String,
    outro: String,
    plans: Vec<RawVideoPlan>,
}

#[derive(Clone, Deserialize)]
struct RawVideo {
    jimeng: RawVideoPlatform,
    doubao: RawVideoPlatform,
    note: String,
    full: RawVideoFull,
}

#[derive(Clone, Deserialize)]
struct RawData {
    version: String,
    default_platform: String,
    platforms: Vec<RawPlatform>,
    hard_rules: String,
    hard_rules_en: String,
    purify_suffix: String,
    purify_suffix_en: String,
    grid_pairs: Vec<RawPair>,
    states: Vec<RawState>,
    video_spells: RawVideo,
    watermark_rois: Vec<CornerRoi>,
    failure_checks: Vec<FailureCheck>,
    /// 知名形象拦截失败文案关键词库（环节十二，可热更）。
    intercept_keywords: Vec<String>,
}

/// 解析一次、全程复用。
///
/// 原实现每次调用都重新解析这份 25KB JSON —— `watermark_rois()` 在向导每导入
/// 一张图时都会调一次，白白重复解析。缓存后既省掉重复解析，也让
/// 「内嵌数据非法」这类构建期错误在首次访问时就一次性暴露。
fn data() -> &'static RawData {
    static CACHE: std::sync::OnceLock<RawData> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| match serde_json::from_str(DATA) {
        Ok(d) => d,
        // 数据随包内嵌、由 `data_json_is_valid` 单测把关，走到这里说明构建产物已损坏。
        Err(e) => panic!("内嵌咒语包 data.json 非法（构建产物损坏）: {e}"),
    })
}

// ---------------------------------------------------------------- 输出结构

#[derive(Clone, Serialize, Deserialize)]
pub struct FailureCheck {
    pub symptom: String,
    pub cause: String,
    pub action: String,
}

#[derive(Clone, Serialize)]
pub struct GridSpell {
    pub id: String,
    pub title: String,
    pub desc: String,
    pub primary: bool,
    pub free: bool,
    pub states: Vec<String>,
    pub state_names: Vec<String>,
    pub rows: u32,
    pub cols: u32,
    pub text: String,
    pub purified: String,
}

#[derive(Clone, Serialize)]
pub struct SpellEntry {
    pub state: String,
    pub state_name: String,
    pub frames: u32,
    pub note: String,
    pub text: String,
    pub purified: String,
    pub rescue: String,
}

/// 脚本里的一个动作片段：既是给平台看的分镜，也是给导入管线用的时间契约。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSegmentSpell {
    pub state: String,
    pub state_name: String,
    pub seconds: f32,
    /// 片段起止，占全片时长的比例（0..1）——导入时按它切段
    pub start: f32,
    pub end: f32,
    pub frames: u32,
    pub desc: String,
}

/// 一份「一次生成、多状态点亮」的脚本方案。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoPlan {
    pub id: String,
    pub title: String,
    pub desc: String,
    pub free: bool,
    pub seconds: f32,
    pub segments: Vec<VideoSegmentSpell>,
    /// 完整咒语（由片段脚本自动拼出，时间轴与 segments 永远一致）
    pub text: String,
}

// camelCase 与内嵌的 `VideoPlan`/`VideoSegmentSpell` 对齐。原实现漏了这一行,
// 于是 `full_setup` 上线成 snake_case,而前端读的是 `fullSetup` ——
// 「脚本路线的面板设置说明」一直是 undefined,从没显示出来过。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoSpell {
    pub label: String,
    pub setup: String,
    pub idle: String,
    pub walk: String,
    /// 一次生成、多状态点亮的脚本方案（新默认路线）
    pub plans: Vec<VideoPlan>,
    /// 脚本路线的面板设置说明
    pub full_setup: String,
}

#[derive(Clone, Serialize)]
pub struct PlatformSpells {
    pub id: String,
    pub name: String,
    /// "default" | "advanced" | "optional"
    pub role: String,
    pub url: String,
    pub tips: Vec<String>,
    pub grid: Vec<GridSpell>,
    pub singles: Vec<SpellEntry>,
    pub video: Option<VideoSpell>,
}

#[derive(Clone, Serialize)]
pub struct SpellbookBundle {
    pub version: String,
    pub default_platform: String,
    pub hard_rules: String,
    pub video_note: String,
    pub platforms: Vec<PlatformSpells>,
    pub failure_checks: Vec<FailureCheck>,
    /// 拦截识别关键词（环节十二）：前端匹配平台报错文案 → 弹本地路线引导卡。
    pub intercept_keywords: Vec<String>,
}

// ---------------------------------------------------------------- 拼装

/// 参考图锚定开场（§5.2-5：所有咒语默认，不再依赖会话记忆）。
const ANCHOR_ZH: &str =
    "严格按照我附上的角色标准形象图来画（这是该角色的唯一标准，绝不能变成另一只）。";
const ANCHOR_EN: &str = "Draw STRICTLY following the attached character reference image (the ONLY standard for this character; NEVER turn it into a different one).";

fn grid_spell_text(d: &RawData, platform: &RawPlatform, pair: &RawPair, purify: bool) -> String {
    let cols = platform.grid_cols;
    let rows = pair.states.len() as u32;
    let state_of = |key: &str| d.states.iter().find(|s| s.id == key).cloned();

    let mut text = String::new();
    if platform.english {
        text.push_str(ANCHOR_EN);
        text.push_str(&format!(
            "\n{} containing an animation sprite grid of this character: EXACTLY {rows} rows × {cols} columns, evenly arranged.\n\n",
            platform.canvas_line
        ));
        for (i, key) in pair.states.iter().enumerate() {
            if let Some(s) = state_of(key) {
                text.push_str(&format!(
                    "Row {} (left to right, {cols} cells): {}.\n\n",
                    i + 1,
                    s.grid_desc_en.replace("{n}", &cols.to_string())
                ));
            }
        }
        text.push_str(&d.hard_rules_en);
        if purify {
            text.push_str(&d.purify_suffix_en);
        }
    } else {
        text.push_str(ANCHOR_ZH);
        text.push_str(&format!(
            "\n{}，内容是该角色的动画帧网格，共 {rows} 行、每行 {cols} 格，横向均匀排列：\n\n",
            platform.canvas_line
        ));
        for (i, key) in pair.states.iter().enumerate() {
            if let Some(s) = state_of(key) {
                text.push_str(&format!(
                    "第 {} 行（从左到右 {cols} 格）：{}。\n\n",
                    i + 1,
                    s.grid_desc.replace("{n}", &cols.to_string())
                ));
            }
        }
        text.push_str(&d.hard_rules);
        if !platform.suffix.is_empty() {
            text.push('\n');
            text.push_str(&platform.suffix);
        }
        if purify {
            text.push_str(&d.purify_suffix);
        }
    }
    text
}

/// 单行动画条咒语（v1.5 保底 + 专家模式），参考图锚定为默认。
fn single_spell_text(
    d: &RawData,
    platform: &RawPlatform,
    state: &RawState,
    purify: bool,
) -> String {
    let f = state.frames;
    let mut text = String::new();
    text.push_str(ANCHOR_ZH);
    match platform.id.as_str() {
        "chatgpt" => text.push_str(&format!(
            "\n请生成一张 1536×1024 的横版图片：图中内容为横向 1 行 {f} 格的「{}」动画条，格子居中排布，画布其余部分全部填满纯绿：\n- {f} 格连续表现：{}；\n",
            state.name, state.action
        )),
        _ => text.push_str(&format!(
            "\n生成一张横版图：内容为横向 1 行 {f} 格的「{}」动画条，格子居中排布，画布上下与四周全部填满纯绿：\n- {f} 格连续表现：{}；\n",
            state.name, state.action
        )),
    }
    text.push_str(&d.hard_rules);
    if platform.english {
        text.push_str(&format!(
            "\n(EXACT hex #00FF00 solid background, 1 row × {f} frames sprite strip, chibi flat-color style, the SAME character in every frame, white sticker outline, keep all four corners empty green, NO shadows, NO text)"
        ));
    }
    if purify {
        text.push_str(&d.purify_suffix);
    }
    text
}

/// 救帧 v2（附录 A-5）：参考图默认附带，只重生成失败的那一行/那一条。
pub fn rescue_spell(state_key: &str, frames_override: Option<u32>) -> String {
    let d = data();
    let state = d
        .states
        .iter()
        .find(|s| s.id == state_key)
        .cloned()
        .unwrap_or_else(|| RawState {
            id: state_key.into(),
            name: state_key.into(),
            frames: 6,
            action: "同一角色的该状态动作".into(),
            grid_desc: String::new(),
            grid_desc_en: String::new(),
            note: String::new(),
        });
    let f = frames_override.unwrap_or(state.frames);
    format!(
        "严格按照我附上的角色标准形象图，重新画一张单行 {f} 格的「{}」动画：\n{}。\n角色的外形、五官、配色、描边、头身比必须与参考图完全一致，绝对不能变成另一只。\n{}{}",
        state.name, state.action, d.hard_rules, d.purify_suffix
    )
}

/// 由片段脚本拼出「一次生成、多状态点亮」的咒语，并同时算出时间比例契约。
///
/// 咒语里写给平台看的秒数、和导入时用来切段的 `start`/`end` 比例出自同一份 segments，
/// 因此不可能出现"文案说 3 秒、代码按 4 秒切"的错位。
fn build_video_plan(
    d: &RawData,
    plan: &RawVideoPlan,
    state_name: &dyn Fn(&str) -> String,
) -> VideoPlan {
    let f = &d.video_spells.full;
    let total: f32 = plan
        .segments
        .iter()
        .map(|s| s.seconds)
        .sum::<f32>()
        .max(0.1);

    let mut text = String::new();
    text.push_str(ANCHOR_ZH);
    text.push_str(&format!(
        "\n用它生成一段约 {:.0} 秒的视频。\n\n{}\n\n按下面的顺序连续演完这几个动作：\n",
        total, f.rules
    ));

    let mut segments = Vec::with_capacity(plan.segments.len());
    let mut acc = 0f32;
    for (i, s) in plan.segments.iter().enumerate() {
        let start = acc;
        let end = acc + s.seconds;
        text.push_str(&format!(
            "{}. 【第 {:.0}–{:.0} 秒】{}。\n",
            i + 1,
            start,
            end,
            s.desc
        ));
        segments.push(VideoSegmentSpell {
            state: s.state.clone(),
            state_name: state_name(&s.state),
            seconds: s.seconds,
            start: start / total,
            end: end / total,
            frames: s.frames,
            desc: s.desc.clone(),
        });
        acc = end;
    }
    text.push('\n');
    text.push_str(&f.outro);

    VideoPlan {
        id: plan.id.clone(),
        title: plan.title.clone(),
        desc: plan.desc.clone(),
        free: plan.free,
        seconds: total,
        segments,
        text,
    }
}

/// 平台角标 ROI（供导入管线使用，随 data.json 热更）。
pub fn watermark_rois() -> Vec<CornerRoi> {
    data().watermark_rois.clone()
}

/// 这个 URL 是否属于咒语包登记的生成平台。
///
/// 「用系统浏览器打开」的唯一白名单来源:与 data.json 同源,平台增删自动跟上,
/// 不需要在别处再维护一份域名表。全等匹配 —— 只放行咒语包里那几个入口页。
pub fn is_platform_url(url: &str) -> bool {
    data().platforms.iter().any(|p| p.url == url)
}

/// 组装完整咒语包（前端直接展示）。
pub fn bundle() -> SpellbookBundle {
    let d = data();
    let state_name = |key: &str| {
        d.states
            .iter()
            .find(|s| s.id == key)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| key.to_string())
    };

    let platforms = d
        .platforms
        .iter()
        .map(|p| PlatformSpells {
            id: p.id.clone(),
            name: p.name.clone(),
            role: p.role.clone(),
            url: p.url.clone(),
            tips: p.tips.clone(),
            grid: d
                .grid_pairs
                .iter()
                .map(|pair| GridSpell {
                    id: pair.id.clone(),
                    title: pair.title.clone(),
                    desc: pair.desc.clone(),
                    primary: pair.primary,
                    free: pair.free,
                    states: pair.states.clone(),
                    state_names: pair.states.iter().map(|s| state_name(s)).collect(),
                    rows: pair.states.len() as u32,
                    cols: p.grid_cols,
                    text: grid_spell_text(d, p, pair, false),
                    purified: grid_spell_text(d, p, pair, true),
                })
                .collect(),
            singles: d
                .states
                .iter()
                .map(|s| SpellEntry {
                    state: s.id.clone(),
                    state_name: s.name.clone(),
                    frames: s.frames,
                    note: s.note.clone(),
                    text: single_spell_text(d, p, s, false),
                    purified: single_spell_text(d, p, s, true),
                    rescue: rescue_spell(&s.id, None),
                })
                .collect(),
            video: match p.id.as_str() {
                "jimeng" | "doubao" => {
                    let raw = if p.id == "jimeng" {
                        &d.video_spells.jimeng
                    } else {
                        &d.video_spells.doubao
                    };
                    Some(VideoSpell {
                        label: raw.label.clone(),
                        setup: raw.setup.clone(),
                        idle: raw.idle.clone(),
                        walk: raw.walk.clone(),
                        plans: d
                            .video_spells
                            .full
                            .plans
                            .iter()
                            .map(|plan| build_video_plan(d, plan, &state_name))
                            .collect(),
                        full_setup: d.video_spells.full.setup.clone(),
                    })
                }
                _ => None,
            },
        })
        .collect();

    SpellbookBundle {
        version: d.version.clone(),
        default_platform: d.default_platform.clone(),
        hard_rules: d.hard_rules.clone(),
        video_note: d.video_spells.note.clone(),
        platforms,
        failure_checks: d.failure_checks.clone(),
        intercept_keywords: d.intercept_keywords.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 线上字段名就是前端读的键名 —— 一处忘了 `rename_all` 就是界面上一片
    /// `undefined`,而且不会有任何编译期或运行期报错。此测钉死整包的 key 形状。
    #[test]
    fn wire_key_shapes_are_stable() {
        let json = serde_json::to_value(bundle()).unwrap();
        for k in [
            "version",
            "default_platform",
            "hard_rules",
            "video_note",
            "platforms",
            "failure_checks",
            "intercept_keywords",
        ] {
            assert!(json.get(k).is_some(), "顶层缺 {k}");
        }
        let p = &json["platforms"][0];
        for k in ["id", "name", "role", "url", "tips", "grid", "singles"] {
            assert!(p.get(k).is_some(), "platform 缺 {k}");
        }
        assert!(
            p["grid"][0].get("state_names").is_some(),
            "GridSpell 用 snake_case"
        );
        assert!(
            p["singles"][0].get("state_name").is_some(),
            "SpellEntry 用 snake_case"
        );

        // video 分支是 camelCase(与内嵌的 plan/segment 一致);
        // `fullSetup` 曾因漏写 rename_all 上线成 full_setup,前端一直读到 undefined。
        let video = bundle()
            .platforms
            .into_iter()
            .find_map(|p| p.video)
            .expect("至少一个平台带视频咒语");
        let v = serde_json::to_value(video).unwrap();
        assert!(
            v.get("fullSetup").is_some(),
            "VideoSpell.fullSetup 必须是 camelCase"
        );
        assert!(v.get("full_setup").is_none());
        let seg = &v["plans"][0]["segments"][0];
        assert!(
            seg.get("stateName").is_some(),
            "VideoSegmentSpell 用 camelCase"
        );
    }

    /// 外链白名单只认咒语包登记的那几个入口页 —— 多一个字符都不放行。
    #[test]
    fn platform_url_whitelist_is_exact() {
        let bundle = bundle();
        assert!(!bundle.platforms.is_empty(), "至少要有一个平台");
        for p in &bundle.platforms {
            assert!(is_platform_url(&p.url), "登记的平台 {} 应放行", p.id);
        }
        assert!(!is_platform_url("https://evil.example.com/"));
        assert!(!is_platform_url(""));
        // 前缀/后缀拼接不得绕过
        let first = &bundle.platforms[0].url;
        assert!(!is_platform_url(&format!("{first}../../etc")));
        assert!(!is_platform_url(&format!(
            "https://evil.example.com/?u={first}"
        )));
    }

    /// 一次生成、多状态点亮：咒语里的时间轴必须与切段用的比例契约完全一致。
    #[test]
    fn video_plans_timeline_matches_segment_contract() {
        let b = bundle();
        let doubao = b.platforms.iter().find(|p| p.id == "doubao").unwrap();
        let v = doubao.video.as_ref().expect("豆包应有视频咒语");
        assert!(!v.plans.is_empty(), "应至少有一份脚本方案");

        for plan in &v.plans {
            assert!(plan.segments.len() >= 2, "{} 应至少两段", plan.title);
            // 比例契约必须首尾闭合、逐段相接
            assert!(
                plan.segments[0].start.abs() < 1e-6,
                "{} 首段应从 0 开始",
                plan.title
            );
            assert!(
                (plan.segments.last().unwrap().end - 1.0).abs() < 1e-6,
                "{} 末段应到 1.0 结束",
                plan.title
            );
            for w in plan.segments.windows(2) {
                assert!(
                    (w[0].end - w[1].start).abs() < 1e-6,
                    "{} 分段之间不应有缝",
                    plan.title
                );
            }
            // 每段都必须是合法状态
            for s in &plan.segments {
                assert!(
                    crate::petkit::StateId::from_key(&s.state).is_some(),
                    "{} 里的 {} 不是合法状态",
                    plan.title,
                    s.state
                );
            }
            // 咒语正文必须逐段写出秒区间（时间轴与契约同源）
            let mut acc = 0f32;
            for s in &plan.segments {
                let line = format!("【第 {:.0}–{:.0} 秒】", acc, acc + s.seconds);
                assert!(
                    plan.text.contains(&line),
                    "{} 咒语缺少时间段 {line}",
                    plan.title
                );
                acc += s.seconds;
            }
            assert!(
                plan.text.contains(&s_round(plan.seconds)),
                "{} 咒语应写明总时长",
                plan.title
            );
        }

        // 免费方案只能碰免费状态，否则用户走完流程会在进化那一步被拦
        for plan in v.plans.iter().filter(|p| p.free) {
            for s in &plan.segments {
                assert!(
                    ["idle", "walk-right"].contains(&s.state.as_str()),
                    "免费方案 {} 不应包含孵化师版状态 {}",
                    plan.title,
                    s.state
                );
            }
        }
    }

    fn s_round(v: f32) -> String {
        format!("{v:.0} 秒")
    }

    /// 走路/奔跑段必须要求「原地踏步」。
    ///
    /// 回归背景：脚本曾写成「朝画面右侧行走」——字面上就是让模型把角色横穿画面，
    /// 结果角色走出取景框，抠出来的精灵后半身被切成一条直边，像素无法补回。
    /// 而软件本来就用质心居中抹掉位移、靠移动窗口实现桌面行走，素材要的只是步态本身。
    #[test]
    fn locomotion_segments_demand_walking_in_place() {
        let b = bundle();
        for p in b.platforms.iter().filter(|p| p.video.is_some()) {
            let v = p.video.as_ref().unwrap();
            for plan in &v.plans {
                for seg in plan.segments.iter().filter(|s| s.state.starts_with("walk")) {
                    assert!(
                        seg.desc.contains("原地"),
                        "{} / {} 的移动段必须写明「原地」，实际：{}",
                        plan.title,
                        seg.state_name,
                        seg.desc
                    );
                    assert!(
                        !seg.desc.contains("朝画面右侧行走")
                            && !seg.desc.contains("朝画面左侧行走"),
                        "{} / {} 不得使用「朝画面某侧行走」这种会让角色横穿画面的措辞",
                        plan.title,
                        seg.state_name
                    );
                }
            }
            // 单状态视频咒语（「只重做一个状态」走的就是它）同样要求原地
            assert!(
                v.walk.contains("原地"),
                "{} 的单状态走路咒语必须写明「原地」",
                p.name
            );
        }
    }

    /// 每份脚本都必须带上「全身完整在框内」的构图约束与自查清单。
    #[test]
    fn video_plans_carry_framing_constraints() {
        let b = bundle();
        let v = b
            .platforms
            .iter()
            .find(|p| p.id == "doubao")
            .unwrap()
            .video
            .as_ref()
            .unwrap();
        for plan in &v.plans {
            for needle in [
                "全身完整",
                "画面正中",
                "留出",
                "务必避免",
                "走出画面",
                "固定镜头",
            ] {
                assert!(
                    plan.text.contains(needle),
                    "{} 的咒语缺少构图约束「{needle}」",
                    plan.title
                );
            }
        }
    }

    #[test]
    fn bundle_covers_4_platforms_doubao_default_first() {
        let b = bundle();
        assert_eq!(b.default_platform, "doubao");
        assert_eq!(b.platforms.len(), 4);
        assert_eq!(b.platforms[0].id, "doubao", "豆包必须排第一（默认平台）");
        assert_eq!(b.platforms[0].role, "default");
        assert_eq!(b.platforms[1].id, "jimeng");
    }

    #[test]
    fn every_platform_has_3_grids_and_8_singles() {
        for p in &bundle().platforms {
            assert_eq!(p.grid.len(), 3, "{} 网格对数", p.id);
            assert_eq!(p.singles.len(), 8, "{} 单行咒语数", p.id);
        }
    }

    #[test]
    fn doubao_primary_grid_matches_layout_spec() {
        let b = bundle();
        let g = &b.platforms[0].grid[0];
        assert!(g.primary && g.free);
        assert_eq!((g.rows, g.cols), (2, 6), "豆包 2 行 × 6 列");
        assert!(g.text.contains("16:9"));
        assert!(g.text.contains("共 2 行、每行 6 格"));
        assert!(g.text.contains("第 1 行"));
        assert!(g.text.contains("第 2 行"));
        assert!(g.text.contains("硬性规则"));
        assert!(g.text.contains("角色标准形象图"), "参考图锚定必须默认");
        assert!(g.text.contains("四个角落"), "角标规避条款（§5.4 第 1 层）");
        assert!(g.text.contains("白色贴纸式描边"));
    }

    #[test]
    fn chatgpt_grid_is_2x5_and_ratio_bug_removed() {
        let b = bundle();
        let chatgpt = b.platforms.iter().find(|p| p.id == "chatgpt").unwrap();
        let g = &chatgpt.grid[0];
        assert_eq!(
            (g.rows, g.cols),
            (2, 5),
            "ChatGPT 2 行 × 5 列（3:2 画幅内）"
        );
        assert!(g.text.contains("1536×1024"));
        assert!(!g.text.contains("1152:208"), "F2 确定性 bug 必须删除");
        assert!(g.text.contains("每行整整 5 格"));
        for s in &chatgpt.singles {
            assert!(!s.text.contains("1152:208"));
        }
    }

    #[test]
    fn gemini_grid_is_english_with_caps_emphasis() {
        let b = bundle();
        let gemini = b.platforms.iter().find(|p| p.id == "gemini").unwrap();
        let g = &gemini.grid[0];
        assert!(g.text.contains("EXACTLY 2 rows"));
        assert!(g.text.contains("#00FF00"));
        assert!(g.text.contains("ALL FOUR CORNERS"));
        assert!(g.text.contains("reference image"));
    }

    #[test]
    fn video_spells_on_doubao_and_jimeng_only() {
        let b = bundle();
        for p in &b.platforms {
            match p.id.as_str() {
                "doubao" | "jimeng" => {
                    let v = p.video.as_ref().expect("应有视频咒语");
                    assert!(v.idle.contains("固定镜头"));
                    assert!(v.walk.contains("原地踏步"));
                    assert!(v.idle.contains("回到与开头完全相同的姿势"));
                }
                _ => assert!(p.video.is_none()),
            }
        }
        assert!(b.video_note.contains("真人照片"));
    }

    #[test]
    fn rescue_v2_anchors_reference_image() {
        let s = rescue_spell("happy", Some(5));
        assert!(s.contains("角色标准形象图"));
        assert!(s.contains("单行 5 格"));
        assert!(s.contains("绝对不能变成另一只"));
    }

    #[test]
    fn failure_checks_cover_known_symptoms() {
        let b = bundle();
        assert!(b.failure_checks.len() >= 10, "自查表条目只增不减");
        let joined: String = b
            .failure_checks
            .iter()
            .map(|c| c.symptom.clone())
            .collect::<Vec<_>>()
            .join("|");
        for expect in ["棋盘格", "行序颠倒", "角标", "侵权", "碎片", "没有动"] {
            assert!(joined.contains(expect), "自查表缺少「{expect}」条目");
        }
    }

    #[test]
    fn intercept_keywords_present() {
        let b = bundle();
        assert!(b.intercept_keywords.iter().any(|k| k.contains("侵权")));
        assert!(b.intercept_keywords.iter().any(|k| k == "无法返回"));
    }

    #[test]
    fn watermark_rois_cover_four_corners() {
        let rois = watermark_rois();
        assert_eq!(rois.len(), 4);
        assert!(rois.iter().all(|r| r.w > 0.0 && r.h > 0.0));
    }
}
