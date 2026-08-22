//! Prompt assembly (spec §7.4, layout §11.D).
//!
//! The prompt is split into a byte-stable **prefix** (system message: role,
//! LevelSpec text, output schema, few-shots — identical for every request at a
//! given level and prompt version) and a **variable tail** (user message:
//! scene, count, avoid-fingerprints). Stability buys output consistency on
//! free channels and prefix-cache hits on paid ones (同一设计双重红利, §8).

use sf_core::spec::LevelSpec;

/// Version tag baked into the prefix; bump when few-shots/rules change so the
/// factory can regression-test prompt versions against the gold set (§8).
pub const PROMPT_VERSION: &str = "v1";

/// Output schema description embedded verbatim in the prefix.
const SCHEMA: &str = r#"[{"en":"英文句","zh":"中文翻译","pattern":"句型公式","words":[{"w":"单词","ipa":"英式音标(无斜杠)","pos":"pron|n|v|aux|modal|adj|wh|adv|prep|art|conj|num|propn|part"}],"chunks":[{"r":"subj|pred|link|obj|comp|advl|objc|marker","i":[词序号,从0起]}],"note":"一句话讲解"}]"#;

/// 入门档 few-shots(L1/L2 用):**每个词都在 500 词带内、句长 ≤8**,
/// 所以在最严的 L1 也能原样通过校验。
///
/// 为什么要卡这条线:旧例句里 `passport` 根本不在词表、`grade`(1612)与
/// `counter`(2539)在 L1–L3 全部越级 —— 等于拿"会被自己校验器拒掉的句子"
/// 教模型,模型照着学、再被拒掉。旧例 2 的中英文还对不上(英文 "I am…"、
/// 中文"我们…")。`sf gold run` 现在会逐条校验这些例句,防止再退化。
const FEW_SHOTS_BASIC: &str = r#"合格示例 1:
{"en":"May I have some water, please?","zh":"请给我一杯水。","pattern":"情态动词疑问句","words":[{"w":"May","ipa":"meɪ","pos":"modal"},{"w":"I","ipa":"aɪ","pos":"pron"},{"w":"have","ipa":"hæv","pos":"v"},{"w":"some","ipa":"səm","pos":"adj"},{"w":"water","ipa":"ˈwɔːtə","pos":"n"},{"w":"please","ipa":"pliːz","pos":"adv"}],"chunks":[{"r":"marker","i":[0]},{"r":"subj","i":[1]},{"r":"pred","i":[2]},{"r":"obj","i":[3,4]},{"r":"marker","i":[5]}],"note":"May I…? 是礼貌请求的固定句式。"}
合格示例 2:
{"en":"I am here with my friend.","zh":"我和朋友一起在这儿。","pattern":"主+系+表+介词短语","words":[{"w":"I","ipa":"aɪ","pos":"pron"},{"w":"am","ipa":"æm","pos":"aux"},{"w":"here","ipa":"hɪə","pos":"adv"},{"w":"with","ipa":"wɪð","pos":"prep"},{"w":"my","ipa":"maɪ","pos":"pron"},{"w":"friend","ipa":"frend","pos":"n"}],"chunks":[{"r":"subj","i":[0]},{"r":"link","i":[1]},{"r":"comp","i":[2]},{"r":"advl","i":[3,4,5]}],"note":"with… 说明跟谁一起。"}
合格示例 3:
{"en":"How much is this book?","zh":"这本书多少钱?","pattern":"特殊疑问句","words":[{"w":"How","ipa":"haʊ","pos":"wh"},{"w":"much","ipa":"mʌtʃ","pos":"adv"},{"w":"is","ipa":"ɪz","pos":"aux"},{"w":"this","ipa":"ðɪs","pos":"pron"},{"w":"book","ipa":"bʊk","pos":"n"}],"chunks":[{"r":"marker","i":[0,1]},{"r":"link","i":[2]},{"r":"subj","i":[3,4]}],"note":"How much…? 用于问价钱。"}
反例 1(越级——用了该等级词表外的词,禁止):
{"en":"The bureaucracy expedited my visa application."}
反例 2(翻译腔——中文生硬直译,禁止):
{"zh":"我可以看你的护照吗,请?"}"#;

/// 进阶档 few-shots(L3 及以上):词带 ≤1500、句长 ≤12,在 L3 也合法。
/// 高等级如果只看入门档的 5 词短句,会被带得过于简单 —— 所以分两档。
const FEW_SHOTS_ADVANCED: &str = r#"合格示例 1:
{"en":"May I have some water, please?","zh":"请给我一杯水。","pattern":"情态动词疑问句","words":[{"w":"May","ipa":"meɪ","pos":"modal"},{"w":"I","ipa":"aɪ","pos":"pron"},{"w":"have","ipa":"hæv","pos":"v"},{"w":"some","ipa":"səm","pos":"adj"},{"w":"water","ipa":"ˈwɔːtə","pos":"n"},{"w":"please","ipa":"pliːz","pos":"adv"}],"chunks":[{"r":"marker","i":[0]},{"r":"subj","i":[1]},{"r":"pred","i":[2]},{"r":"obj","i":[3,4]},{"r":"marker","i":[5]}],"note":"May I…? 是礼貌请求的固定句式。"}
合格示例 2:
{"en":"I would like to change my flight, please.","zh":"我想改签航班。","pattern":"would like to + 动词原形","words":[{"w":"I","ipa":"aɪ","pos":"pron"},{"w":"would","ipa":"wʊd","pos":"modal"},{"w":"like","ipa":"laɪk","pos":"v"},{"w":"to","ipa":"tə","pos":"part"},{"w":"change","ipa":"tʃeɪndʒ","pos":"v"},{"w":"my","ipa":"maɪ","pos":"pron"},{"w":"flight","ipa":"flaɪt","pos":"n"},{"w":"please","ipa":"pliːz","pos":"adv"}],"chunks":[{"r":"subj","i":[0]},{"r":"pred","i":[1,2]},{"r":"objc","i":[3,4]},{"r":"obj","i":[5,6]},{"r":"marker","i":[7]}],"note":"would like to… 比 want to 更客气。"}
合格示例 3:
{"en":"Could you tell me where to check in?","zh":"能告诉我在哪里办理登机吗?","pattern":"礼貌请求 + 宾语从句","words":[{"w":"Could","ipa":"kʊd","pos":"modal"},{"w":"you","ipa":"juː","pos":"pron"},{"w":"tell","ipa":"tel","pos":"v"},{"w":"me","ipa":"miː","pos":"pron"},{"w":"where","ipa":"weə","pos":"wh"},{"w":"to","ipa":"tə","pos":"part"},{"w":"check","ipa":"tʃek","pos":"v"},{"w":"in","ipa":"ɪn","pos":"part"}],"chunks":[{"r":"marker","i":[0]},{"r":"subj","i":[1]},{"r":"pred","i":[2]},{"r":"objc","i":[3]},{"r":"obj","i":[4,5,6,7]}],"note":"Could you tell me…? 问路问事都好用。"}
反例 1(越级——用了该等级词表外的词,禁止):
{"en":"The bureaucracy expedited my visa application."}
反例 2(翻译腔——中文生硬直译,禁止):
{"zh":"我可以看你的护照吗,请?"}"#;

/// 按等级挑 few-shot 档位:词带 ≤1000(L1/L2)用入门档,其余用进阶档。
/// 前缀本来就逐等级不同(内嵌 LevelSpec),分档不影响前缀缓存。
fn few_shots_for(spec: &LevelSpec) -> &'static str {
    if spec.vocab_band > 0 && spec.vocab_band <= 1000 {
        FEW_SHOTS_BASIC
    } else {
        FEW_SHOTS_ADVANCED
    }
}

/// 供 gold 回归逐条校验用:所有 few-shot 档位与其"必须能通过的最低等级词带"。
pub const FEW_SHOT_TIERS: [(&str, u32); 2] = [(FEW_SHOTS_BASIC, 500), (FEW_SHOTS_ADVANCED, 1500)];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptParts {
    /// Byte-stable prefix — send as the system message.
    pub system: String,
    /// Variable tail — send as the user message.
    pub user: String,
}

/// Cap on banned words carried in the tail — beyond this the list stops
/// teaching the model anything and just burns tokens.
const MAX_BANNED_WORDS: usize = 30;

/// 尾部"已经写过、别重复"的例句条数上限。
///
/// 此前这里传的是 simhash 的 16 位十六进制**指纹**。模型算不出 simhash,
/// 那串东西对它毫无意义 —— 既起不到避重作用,又每批白烧几十 token。
/// 换成真句子后模型才真的知道要避开什么;条数压在 8 条以内,长了同样是浪费。
const MAX_AVOID_EXAMPLES: usize = 8;

/// 拼出尾部的"避开已有句"段落。空表时整段不出现。
fn avoid_section(avoid: &[&str]) -> String {
    let lines: Vec<&str> = avoid
        .iter()
        .copied()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(MAX_AVOID_EXAMPLES)
        .collect();
    if lines.is_empty() {
        return String::new();
    }
    format!(
        "\n这些句子已经写过,不要重复、也不要只换一两个词:\n{}",
        lines
            .iter()
            .map(|s| format!("- {s}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// Build the full prompt for a generation request.
///
/// * `spec` — target level (its YAML re-serialization is embedded verbatim).
/// * `scene` — 用户场景描述 (自由文本) or factory scene tag.
/// * `count` — 句数.
/// * `avoid` — 已接受句子的英文原文(尾部据此提示模型避开)。
/// * `banned` — 失败样本回灌 (§8 的运行时形态):本次任务里已被词表校验拒绝的
///   单词,显式禁用以提高补足批通过率。空则不出现在 prompt 中。
pub fn build_prompt(
    spec: &LevelSpec,
    scene: &str,
    count: u32,
    avoid: &[&str],
    banned: &[String],
) -> PromptParts {
    let spec_yaml = serde_yaml::to_string(spec).expect("spec serialization cannot fail");
    let few_shots = few_shots_for(spec);
    let system = format!(
        "你是英语教研内容生成器,只输出 JSON 数组,不输出任何其他文字。\n\
         [prompt-version: {PROMPT_VERSION}]\n\n\
         ## 等级规格(必须严格遵守词表带、语法白名单与句长上限)\n{spec_yaml}\n\
         ## 输出 schema\n{SCHEMA}\n\n\
         ## 示例\n{few_shots}\n\n\
         ## 规则\n\
         - 句子必须是自然口语,场景真实可用;\n\
         - 中文必须是日常表达,禁止翻译腔;\n\
         - **只用该等级词表带内的常见词**;想不出常见说法就换个说法,\n\
           不要为了凑场景硬塞生僻名词(校验器会按词表带逐词卡);\n\
         - 人名/地名/品牌名请标 pos 为 propn(它们不受词表带限制);\n\
         - 音标用英式 IPA,不带斜杠;\n\
         - words 必须与 en 逐词一致(句末标点不算词);\n\
         - chunks 必须覆盖每个词恰好一次;\n\
         - 只输出 JSON 数组。"
    );
    let mut user = format!("场景:{scene};等级 {};生成 {count} 句。", spec.id);
    user.push_str(&avoid_section(avoid));
    if !banned.is_empty() {
        let words: Vec<&str> = banned
            .iter()
            .take(MAX_BANNED_WORDS)
            .map(String::as_str)
            .collect();
        user.push_str(&format!(
            "\n以下单词超出该等级词表,禁止使用(换用词表内的常见词表达):{}",
            words.join(", ")
        ));
    }
    PromptParts { system, user }
}

/// 场景对话的输出 schema:在通用 schema 上增加 `speaker`(A/B 轮替)。
const SCENARIO_SCHEMA: &str = r#"[{"speaker":"A或B","en":"英文句","zh":"中文翻译","pattern":"句型公式","words":[{"w":"单词","ipa":"英式音标(无斜杠)","pos":"pron|n|v|aux|modal|adj|wh|adv|prep|art|conj|num|propn|part"}],"chunks":[{"r":"subj|pred|link|obj|comp|advl|objc|marker","i":[词序号,从0起]}],"note":"一句话讲解"}]"#;

/// 场景对话 few-shots:真实口语、高频语块、双角色轮替。
const SCENARIO_FEW_SHOTS: &str = r#"合格示例(咖啡店点单,片段):
{"speaker":"A","en":"Hi, what can I get for you?","zh":"你好,想喝点什么?","pattern":"特殊疑问句","words":[{"w":"Hi","ipa":"haɪ","pos":"part"},{"w":"what","ipa":"wɒt","pos":"wh"},{"w":"can","ipa":"kæn","pos":"modal"},{"w":"I","ipa":"aɪ","pos":"pron"},{"w":"get","ipa":"ɡet","pos":"v"},{"w":"for","ipa":"fə","pos":"prep"},{"w":"you","ipa":"juː","pos":"pron"}],"chunks":[{"r":"marker","i":[0]},{"r":"obj","i":[1]},{"r":"pred","i":[2,3,4]},{"r":"advl","i":[5,6]}],"note":"店员招呼顾客的高频问法。"}
{"speaker":"B","en":"Could I get a medium latte, please?","zh":"请给我一杯中杯拿铁。","pattern":"礼貌请求","words":[{"w":"Could","ipa":"kʊd","pos":"modal"},{"w":"I","ipa":"aɪ","pos":"pron"},{"w":"get","ipa":"ɡet","pos":"v"},{"w":"a","ipa":"ə","pos":"art"},{"w":"medium","ipa":"ˈmiːdiəm","pos":"adj"},{"w":"latte","ipa":"ˈlɑːteɪ","pos":"n"},{"w":"please","ipa":"pliːz","pos":"adv"}],"chunks":[{"r":"marker","i":[0]},{"r":"subj","i":[1]},{"r":"pred","i":[2]},{"r":"obj","i":[3,4,5]},{"r":"marker","i":[6]}],"note":"Could I get… 是点单最常用的说法。"}
反例(书面腔,禁止):
{"en":"I would like to purchase a beverage of medium size."}
反例(自言自语、不成对话,禁止):
{"speaker":"A","en":"Coffee is a popular drink around the world."}"#;

/// 场景对话生成 prompt(《场景练习模块-实现方案》§3.4)。
///
/// 与等级 prompt 的区别:**不带 LevelSpec**(不受词表带/语法白名单约束),
/// 改以"真实口语对话"为约束;`speaker` 字段承载 A/B 轮替。
/// 前缀同样字节稳定(利于缓存),变量只在 user 段。
pub fn build_scenario_prompt(
    scene: &str,
    count: u32,
    avoid: &[&str],
    banned: &[String],
) -> PromptParts {
    let system = format!(
        "你是英语口语对话内容生成器,只输出 JSON 数组,不输出任何其他文字。\n\
         [prompt-version: {PROMPT_VERSION}-scenario]\n\n\
         ## 输出 schema\n{SCENARIO_SCHEMA}\n\n\
         ## 示例\n{SCENARIO_FEW_SHOTS}\n\n\
         ## 规则\n\
         - 生成**一段连续的真实对话**,A 与 B 交替发言,顺序即数组顺序;\n\
         - 用母语者日常口语,优先高频固定说法(语块),不要书面腔;\n\
         - 每句不超过 20 个词,可直接照着说;\n\
         - 中文是地道日常表达,禁止翻译腔;\n\
         - 音标用英式 IPA,不带斜杠;\n\
         - words 必须与 en 逐词一致(句末标点不算词);\n\
         - chunks 必须覆盖每个词恰好一次;\n\
         - 只输出 JSON 数组。"
    );
    let mut user = format!("场景:{scene};生成 {count} 句连续对话(A/B 交替)。");
    user.push_str(&avoid_section(avoid));
    if !banned.is_empty() {
        let words: Vec<&str> = banned
            .iter()
            .take(MAX_BANNED_WORDS)
            .map(String::as_str)
            .collect();
        user.push_str(&format!("\n以下单词请避免使用:{}", words.join(", ")));
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
        let a = build_prompt(&s, "机场值机", 10, &["I am here."], &[]);
        let b = build_prompt(&s, "餐厅点餐", 30, &["Where is it?"], &["burger".into()]);
        assert_eq!(a.system, b.system, "prefix must be byte-stable for caching");
        assert_ne!(a.user, b.user);
    }

    /// 尾部带的是**真句子**,不是 simhash 指纹 —— 模型算不出 simhash,
    /// 指纹既不能避重又白烧 token(见 MAX_AVOID_EXAMPLES 的说明)。
    #[test]
    fn tail_carries_scene_count_and_real_examples() {
        let p = build_prompt(&spec(), "机场值机", 10, &["Where is the gate?"], &[]);
        assert!(p.user.contains("机场值机"));
        assert!(p.user.contains("10"));
        assert!(p.user.contains("Where is the gate?"), "避重段应给出原句");
        assert!(!p.user.contains("禁止使用"), "无禁用词时不出现该段");
    }

    #[test]
    fn avoid_examples_are_capped_and_skipped_when_empty() {
        let many: Vec<String> = (0..20).map(|i| format!("Sentence number {i}.")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        let p = build_prompt(&spec(), "机场值机", 10, &refs, &[]);
        assert!(p.user.contains("Sentence number 0."));
        assert!(
            !p.user.contains("Sentence number 8."),
            "超过 MAX_AVOID_EXAMPLES 的例句应被截断"
        );
        let empty = build_prompt(&spec(), "机场值机", 10, &[], &[]);
        assert!(!empty.user.contains("已经写过"), "无例句时整段不出现");
    }

    /// few-shot 按等级分档:低等级不能看到高带例句(否则模型照抄就越级)。
    #[test]
    fn few_shots_are_tiered_by_level() {
        let mut low = spec();
        low.vocab_band = 500;
        let mut high = spec();
        high.vocab_band = 2800;
        let a = build_prompt(&low, "s", 5, &[], &[]);
        let b = build_prompt(&high, "s", 5, &[], &[]);
        assert!(a.system.contains("How much is this book?"));
        assert!(b.system.contains("would like to change my flight"));
        assert_ne!(a.system, b.system);
    }

    #[test]
    fn tail_carries_banned_words_capped() {
        let banned: Vec<String> = (0..40).map(|i| format!("word{i}")).collect();
        let p = build_prompt(&spec(), "肯德基点餐", 10, &[], &banned);
        assert!(p.user.contains("禁止使用"));
        assert!(p.user.contains("word0"));
        assert!(p.user.contains("word29"));
        assert!(!p.user.contains("word30"), "超过上限的禁用词被截断");
    }

    #[test]
    fn repair_prompt_carries_only_diff() {
        let p = build_repair_prompt("I am fine.", &["缺少音标".into()]);
        assert!(p.user.contains("I am fine."));
        assert!(p.user.contains("缺少音标"));
        assert!(!p.user.contains("few-shot"));
    }
}
