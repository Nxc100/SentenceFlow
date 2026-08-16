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

已含 NGSL 全量 2801 头词(band = 官方词频 rank);种子句涉及词带人工 IPA与释义,其余词条 IPA/释义留空(词典对账仅在有值时覆写)。后续按需补
词典对账扩充(`tools/audit/` 有对账脚本位)。
