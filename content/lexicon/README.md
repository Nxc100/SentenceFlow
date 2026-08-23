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

### 三张表

| 文件 | 内容 | 定什么 |
|------|------|--------|
| `base.tsv` | NGSL 头词 | `band` = 官方词频 rank |
| `supplement.tsv` | NGSL 之外、日常场景绕不开的词 | `band`,依据「最早出现在哪级场景」 |
| `overrides.tsv` | **我们不同意 NGSL 的地方** | 只改 `teach_band`,三列含理由 |

### 两个数,别混

| | 谁在用 | 含义 |
|---|--------|------|
| `band` | **定级测试** | NGSL 词频 rank —— 按它分层估算考生词汇量 |
| `teach_band` | **内容校验** | 这个词从哪一级起能进练习句;缺省 = `band` |

拆开是必须的:`hello` 的 NGSL rank 是 2811(新闻/学术语料里罕见),于是
「你好」在 L1–L5 全算越级;`milk` 950、`red` 700、`bag` 600、`white` 516
—— 初学者最先学的具体名词在 L1 一个都用不了。但**直接改 `band` 会毁掉定级
测试**:认识 `milk` 只记 220 词,考生被低估、误路由到更低的起步等级。

实测(2026-08-23,L1,每场景 10 句,本级入库数):

| 场景 | 覆盖前 | 覆盖后 |
|------|--------|--------|
| 超市购物 | 3 | 6 |
| 闲聊天气 | 5 | 7 |
| 餐厅点餐 | — | 8 |
| 问价与讲价 | — | 7 |

`overrides.tsv` 里的词**必须已经在** base/supplement 里 —— 不在的话它是新词,
属于 `supplement.tsv`,加载时报错而不是悄悄新增。理由列必填:半年后没人记得
当初为什么把某个词提到 L1。

加载时三张表依次应用(`sf` CLI 的 `load_lexicon()`):base + supplement 合并建表,再把 overrides 的 `teach_band` 盖上去。**`supplement.tsv` 不得重定义
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
