# 种子句库(出厂内容源)

手工标注的出厂句子,`sf factory build --from-seed` 经统一校验管线写入
content.db。**每一句都必须通过 sf-pipeline 校验器**(词表带、chunks 全覆盖、
POS/ROLE 枚举、IPA 字符集、simhash 查重),与 AI 生成内容走同一条入库通道。

## 文件格式(每级一个 YAML)

```yaml
level: L1                    # 目标等级,须与 content/specs/<级>.yaml 匹配
sentences:
  - en: "I am fine."         # 句末标点保留在 en 内(入库时自动拆出)
    zh: "我很好。"
    scene: "问候"             # 场景标签(内容库分组)
    func: "回应问候"          # 交际功能
    pattern: "主+系+表"       # 句型公式
    note: "be 动词 am 跟在 I 后面,表示状态。"
    words:                    # 与 en 逐词一致(不含句末标点)
      - { w: "I",    ipa: "aɪ",   pos: "pron" }
      - { w: "am",   ipa: "æm",   pos: "aux" }
      - { w: "fine", ipa: "faɪn", pos: "adj" }
    chunks:                   # 覆盖每个词恰好一次
      - { r: "subj", i: [0] }
      - { r: "link", i: [1] }
      - { r: "comp", i: [2] }
```

- `pos` 枚举:`pron n v aux modal adj wh adv prep art conj num propn part`
- `r` 枚举:`subj pred link obj comp advl objc marker`
- `ipa`:英式 IPA,不带斜杠;重音符 `ˈ` `ˌ`,长音 `ː`
- 词表:每个非 propn 词必须能在 `content/lexicon/base.tsv` 中查到
  (直接命中、规则屈折或不规则表回退)

## 金标(gold)

`content/gold/` 存放金标句集 — prompt 版本回归的标尺(§8)。种子句库全部
默认进入金标;工厂生产扩充后按 6 级 × 全句型补齐至 200 句。
