# 词典数据(lemma 表源文件)

`base.tsv` 是 content.db `lemma` 表与校验器词表带检查的数据源。

格式(TSV,五列):

```
lemma<TAB>band<TAB>ipa_gb<TAB>ipa_us<TAB>zh_gloss
```

- `band`:NGSL 频率带(1 起,数值 = "位于前 n 词"的 n 上取整;
  L1=500 / L2=1000 / L3=1500 / L4=2000 / L5=2800)
- `ipa_gb` / `ipa_us`:英式/美式 IPA,无斜杠
- `zh_gloss`:一词一义的常用中文释义

词表来源:NGSL(New General Service List,CC BY 3.0,Browne, Culligan &
Phillips)。应用内"关于 → 内容来源"页承载署名(§4.9)。

### 两张表

| 文件 | 内容 | band 依据 |
|------|------|-----------|
| `base.tsv` | NGSL 头词 | 官方词频 rank |
| `supplement.tsv` | NGSL 之外、日常场景绕不开的词 | 该词最早出现在哪个等级的场景 |

加载时两张表合并(`sf` CLI 的 `lexicon_tsv()`)。**`supplement.tsv` 不得重定义
`base.tsv` 已有的词** —— `sf gold run` 会检查;悄悄改一个已有词的 band,
等级归属就会莫名其妙地漂移(踩过:补充表把 `noodle` 从 1500 改成 1550,
种子句 "We ordered two bowls of noodles." 当场在 L3 判越级)。

`supplement.tsv` 的 band 定得偏高是有意的:宁可在低等级判越级(分诊会改存到
合适的等级),也不要让 L1 冒生词。

### 已知历史

`base.tsv` 一度声称"已含 NGSL 全量 2801 头词",2026-08-23 实测缺 110 个 rank,
其中包括**不定冠词 `an`**、`three/five/six/eight/twenty/hundred/thousand`
等基数词、全部序数词、全部星期与大部分月份。任何句子含这些词都会被判
"词表外"直接丢弃 —— 这是 AI 造句入库率偏低的头号原因。已补回 57 条
(其余为 is/was/does 这类屈折形式,`lexicon.rs` 的 irregular 表已覆盖)。

种子句涉及词带人工 IPA 与释义,其余词条 IPA/释义留空(词典对账仅在有值时
覆写)。

### 改完要重建 content.db

桌面端**不读**这两个文件,只读 `content.db` 的 `lemma` 表。改完词表必须跑
`sf factory build`,否则 CLI 校验通过的句子到了应用里照样判"词表外"。
