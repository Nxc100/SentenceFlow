//! Prompt assembly (spec §7.4, layout §11.D).
//!
//! The prompt is split into a byte-stable **prefix** (system message: role,
//! LevelSpec text, output schema, few-shots — identical for every request at a
//! given level and prompt version) and a **variable tail** (user message:
//! scene, count, avoid-fingerprints). Stability buys output consistency on
//! free channels and prefix-cache hits on paid ones (同一设计双重红利, §8).

use crate::simhash::fingerprint16;
use sf_core::spec::LevelSpec;

/// Version tag baked into the prefix; bump when few-shots/rules change so the
/// factory can regression-test prompt versions against the gold set (§8).
pub const PROMPT_VERSION: &str = "v1";

/// Output schema description embedded verbatim in the prefix.
const SCHEMA: &str = r#"[{"en":"英文句","zh":"中文翻译","pattern":"句型公式","words":[{"w":"单词","ipa":"英式音标(无斜杠)","pos":"pron|n|v|aux|modal|adj|wh|adv|prep|art|conj|num|propn|part"}],"chunks":[{"r":"subj|pred|link|obj|comp|advl|objc|marker","i":[词序号,从0起]}],"note":"一句话讲解"}]"#;

/// Three positive few-shots + two negatives (越级/翻译腔), per §11.D.
const FEW_SHOTS: &str = r#"合格示例 1:
{"en":"May I see your passport, please?","zh":"请出示您的护照。","pattern":"情态动词疑问句","words":[{"w":"May","ipa":"meɪ","pos":"modal"},{"w":"I","ipa":"aɪ","pos":"pron"},{"w":"see","ipa":"siː","pos":"v"},{"w":"your","ipa":"jɔː","pos":"pron"},{"w":"passport","ipa":"ˈpɑːspɔːt","pos":"n"},{"w":"please","ipa":"pliːz","pos":"adv"}],"chunks":[{"r":"marker","i":[0]},{"r":"subj","i":[1]},{"r":"pred","i":[2]},{"r":"obj","i":[3,4]},{"r":"marker","i":[5]}],"note":"May I…? 是礼貌请求的固定句式。"}
合格示例 2:
{"en":"I am in the same grade.","zh":"我们在同一个年级。","pattern":"主+系+介词短语","words":[{"w":"I","ipa":"aɪ","pos":"pron"},{"w":"am","ipa":"æm","pos":"aux"},{"w":"in","ipa":"ɪn","pos":"prep"},{"w":"the","ipa":"ðə","pos":"art"},{"w":"same","ipa":"seɪm","pos":"adj"},{"w":"grade","ipa":"ɡreɪd","pos":"n"}],"chunks":[{"r":"subj","i":[0]},{"r":"link","i":[1]},{"r":"advl","i":[2,3,4,5]}],"note":"in the same… 表示\"在同一个…\"。"}
合格示例 3:
{"en":"Where is the check-in counter?","zh":"值机柜台在哪里?","pattern":"特殊疑问句","words":[{"w":"Where","ipa":"weə","pos":"wh"},{"w":"is","ipa":"ɪz","pos":"aux"},{"w":"the","ipa":"ðə","pos":"art"},{"w":"check-in","ipa":"ˈtʃekɪn","pos":"n"},{"w":"counter","ipa":"ˈkaʊntə","pos":"n"}],"chunks":[{"r":"marker","i":[0]},{"r":"link","i":[1]},{"r":"subj","i":[2,3,4]}],"note":"Where is…? 用于询问位置。"}
反例 1(越级——低级别出现超纲词,禁止):
{"en":"The bureaucracy expedited my visa application."}
反例 2(翻译腔——中文生硬直译,禁止):
{"zh":"我可以看你的护照吗,请?"}"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptParts {
    /// Byte-stable prefix — send as the system message.
    pub system: String,
    /// Variable tail — send as the user message.
    pub user: String,
}

/// Build the full prompt for a generation request.
///
/// * `spec` — target level (its YAML re-serialization is embedded verbatim).
/// * `scene` — 用户场景描述 (自由文本) or factory scene tag.
/// * `count` — 句数.
/// * `avoid` — simhash fingerprints of already-accepted sentences.
pub fn build_prompt(spec: &LevelSpec, scene: &str, count: u32, avoid: &[u64]) -> PromptParts {
    let spec_yaml = serde_yaml::to_string(spec).expect("spec serialization cannot fail");
    let system = format!(
        "你是英语教研内容生成器,只输出 JSON 数组,不输出任何其他文字。\n\
         [prompt-version: {PROMPT_VERSION}]\n\n\
         ## 等级规格(必须严格遵守词表带、语法白名单与句长上限)\n{spec_yaml}\n\
         ## 输出 schema\n{SCHEMA}\n\n\
         ## 示例\n{FEW_SHOTS}\n\n\
         ## 规则\n\
         - 句子必须是自然口语,场景真实可用;\n\
         - 中文必须是日常表达,禁止翻译腔;\n\
         - 音标用英式 IPA,不带斜杠;\n\
         - words 必须与 en 逐词一致(句末标点不算词);\n\
         - chunks 必须覆盖每个词恰好一次;\n\
         - 只输出 JSON 数组。"
    );
    let mut user = format!("场景:{scene};等级 {};生成 {count} 句。", spec.id);
    if !avoid.is_empty() {
        let fps: Vec<String> = avoid.iter().map(|h| fingerprint16(*h)).collect();
        user.push_str(&format!("\n避开与以下指纹相似的句子:{}", fps.join(",")));
    }
    PromptParts { system, user }
}

/// Repair prompt: only the diff travels (修补调用仅传差异, §7.4).
pub fn build_repair_prompt(en: &str, issues_zh: &[String]) -> PromptParts {
    PromptParts {
        system: format!(
            "你是英语教研内容修补器,只输出一个符合 schema 的 JSON 对象,不输出任何其他文字。\n\
             ## 输出 schema\n{SCHEMA}\n(输出单个对象,不是数组)"
        ),
        user: format!(
            "修补这句的标注问题:\n句子:{en}\n问题:{}",
            issues_zh.join("；")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> LevelSpec {
        LevelSpec::from_yaml(
            r#"
id: L3
cefr: "A2"
vocab_band: 1500
max_words: 12
grammar_whitelist: [past_simple]
can_do: ["点餐"]
practice:
  flow: typing
  review_listening_ratio: 0.3
  dictation_min_box: 0
  hints: { ipa: on_click, first_letter: false, zh_hideable: false }
  judge: { strict: true }
  srs:
    daily_new_default: 20
    daily_new_range: [5, 50]
    review_cap: 60
    box_intervals_days: [1, 3, 7, 14]
    box5_recheck_days: 30
    listening_weight: 1.5
"#,
        )
        .unwrap()
    }

    #[test]
    fn prefix_is_stable_across_requests() {
        let s = spec();
        let a = build_prompt(&s, "机场值机", 10, &[1, 2]);
        let b = build_prompt(&s, "餐厅点餐", 30, &[9]);
        assert_eq!(a.system, b.system, "prefix must be byte-stable for caching");
        assert_ne!(a.user, b.user);
    }

    #[test]
    fn tail_carries_scene_count_and_fingerprints() {
        let p = build_prompt(&spec(), "机场值机", 10, &[0xABCD]);
        assert!(p.user.contains("机场值机"));
        assert!(p.user.contains("10"));
        assert!(p.user.contains("000000000000abcd"));
    }

    #[test]
    fn repair_prompt_carries_only_diff() {
        let p = build_repair_prompt("I am fine.", &["缺少音标".into()]);
        assert!(p.user.contains("I am fine."));
        assert!(p.user.contains("缺少音标"));
        assert!(!p.user.contains("few-shot"));
    }
}
