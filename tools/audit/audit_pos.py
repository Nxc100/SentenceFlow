#!/usr/bin/env python3
"""spaCy 离线审计(§7.4):抽样核对 content.db 的词性标注。

纯 QA 工具,不在任何运行时依赖树里。用法:

    pip install spacy && python -m spacy download en_core_web_sm
    python tools/audit/audit_pos.py content/build/content.db --sample 50

将我方 POS 短码与 spaCy 的 universal POS 对照,输出可疑标注清单;
分歧不等于错误(教学口径 ≠ 语料库口径),供人工抽审(§8 的 5% 抽审环节)参考。
"""

from __future__ import annotations

import argparse
import json
import random
import sqlite3
import sys

# 我方短码 → 可接受的 spaCy universal POS 集合(教学口径的宽松映射)
POS_MAP: dict[str, set[str]] = {
    "pron": {"PRON", "DET"},          # my/this 教学上归代词
    "n": {"NOUN"},
    "v": {"VERB", "AUX"},
    "aux": {"AUX", "VERB"},
    "modal": {"AUX", "VERB"},
    "adj": {"ADJ"},
    "wh": {"ADV", "PRON", "DET", "SCONJ"},
    "adv": {"ADV", "PART", "INTJ"},
    "prep": {"ADP"},
    "art": {"DET"},
    "conj": {"CCONJ", "SCONJ"},
    "num": {"NUM"},
    "propn": {"PROPN", "NOUN"},
    "part": {"PART", "ADP"},
}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("db", help="content.db path")
    parser.add_argument("--sample", type=int, default=50, help="抽样句数")
    parser.add_argument("--seed", type=int, default=42)
    args = parser.parse_args()

    try:
        import spacy
    except ImportError:
        print("需要 spacy:pip install spacy && python -m spacy download en_core_web_sm")
        return 2
    nlp = spacy.load("en_core_web_sm")

    conn = sqlite3.connect(args.db)
    rows = conn.execute("SELECT id, en, words FROM sentence").fetchall()
    random.Random(args.seed).shuffle(rows)
    rows = rows[: args.sample]

    suspicious = 0
    checked = 0
    for sid, en, words_json in rows:
        ours = json.loads(words_json)
        doc = [t for t in nlp(en) if not t.is_punct]
        if len(doc) != len(ours):
            print(f"[{sid}] 分词不一致({len(doc)} vs {len(ours)}): {en}")
            suspicious += 1
            continue
        for tok, word in zip(doc, ours):
            checked += 1
            allowed = POS_MAP.get(word["pos"], set())
            if tok.pos_ not in allowed:
                print(
                    f"[{sid}] {word['w']!r}: 我方 {word['pos']} vs spaCy {tok.pos_} — {en}"
                )
                suspicious += 1

    total = max(1, checked)
    print(f"\n抽审 {len(rows)} 句 / {checked} 词;可疑 {suspicious}({suspicious / total:.1%})")
    return 0 if suspicious / total <= 0.05 else 1


if __name__ == "__main__":
    sys.exit(main())
