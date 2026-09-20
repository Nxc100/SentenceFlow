/**
 * 咒语包:网格咒语(主推)+ 视频咒语 + 单行咒语(保底/专家)+ 引导图生成器
 * + 拦截自检 + 失败自查表。全部文案来自 `sf-pet` 内嵌的 data.json,随版本热更。
 */

import { useCallback, useEffect, useMemo, useState } from "react";
import { Button, useToast } from "@sentenceflow/ui";
import { petIpc } from "../../pet/ipc";
import type { PlatformSpells, SpellbookBundle } from "../../pet/types";
import { Orb, SectionHead, copyText } from "./common";
import type { PetTab } from "./index";
import { errText } from "./usePetSettings";

export function SpellbookTab({ onGoto }: { onGoto: (t: PetTab) => void }) {
  const [book, setBook] = useState<SpellbookBundle | null>(null);
  const [platformId, setPlatformId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    petIpc
      .spellbookGet()
      .then((b) => {
        if (!alive) return;
        setBook(b);
        setPlatformId(b.default_platform);
      })
      .catch((e) => alive && setError(errText(e)));
    return () => {
      alive = false;
    };
  }, []);

  const platform = useMemo(
    () => book?.platforms.find((p) => p.id === platformId) ?? book?.platforms[0] ?? null,
    [book, platformId],
  );

  if (error) return <div className="aipet-note aipet-note--err">{error}</div>;
  if (!book || !platform) return <p className="aipet-loading">载入中…</p>;

  return (
    <div className="aipet-panels">
      <section className="aipet-panel">
        <SectionHead
          icon="📜"
          title="咒语包"
          desc="默认路径只要 1 次生成:复制「双行网格」咒语 → 附上角色参考图 → 生成 → 拖回。"
          right={<span className="aipet-badge">模板 {book.version}</span>}
        />
        <PlatformTabs book={book} current={platform.id} onPick={setPlatformId} />
        <ul className="aipet-tips">
          {platform.tips.map((t, i) => (
            <li key={i}>💡 {t}</li>
          ))}
        </ul>
      </section>

      <GridSection platform={platform} />
      <VideoSection platform={platform} note={book.video_note} />
      <SinglesSection platform={platform} />
      <GuideGenerator />
      <InterceptChecker keywords={book.intercept_keywords} onGoto={onGoto} />

      <section className="aipet-panel">
        <SectionHead icon="📐" title="通用硬性规则" desc="所有咒语共用的规则块。" />
        <pre className="aipet-spell">{book.hard_rules}</pre>
      </section>

      <section className="aipet-panel">
        <SectionHead icon="🔎" title="失败自查表" />
        <div className="aipet-table-wrap">
          <table className="aipet-table">
            <thead>
              <tr>
                <th>症状</th>
                <th>原因</th>
                <th>下一步动作</th>
              </tr>
            </thead>
            <tbody>
              {book.failure_checks.map((c, i) => (
                <tr key={i}>
                  <td>{c.symptom}</td>
                  <td>{c.cause}</td>
                  <td>{c.action}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}

/* ---------------------------------------------------------------- 平台切换 */

function PlatformTabs({
  book,
  current,
  onPick,
}: {
  book: SpellbookBundle;
  current: string;
  onPick: (id: string) => void;
}) {
  return (
    <div className="aipet-pills">
      {book.platforms.map((p) => (
        <button
          key={p.id}
          type="button"
          className={`aipet-pill${p.id === current ? " aipet-pill--on" : ""}`}
          onClick={() => onPick(p.id)}
        >
          {p.role === "default" ? `${p.name} ★` : p.name}
        </button>
      ))}
    </div>
  );
}

/* ---------------------------------------------------------------- 各分区 */

function GridSection({ platform }: { platform: PlatformSpells }) {
  const { show: toast } = useToast();
  return (
    <section className="aipet-panel">
      <SectionHead
        icon="🧩"
        title={`网格咒语 · ${platform.name}`}
        desc="一图多状态:第 1 行待机、第 2 行走路。"
      />
      {platform.grid.map((g) => (
        <div key={g.id} className="aipet-spellcard">
          <div className="aipet-spellcard__head">
            <span className="aipet-spellcard__title">{g.title}</span>
            {g.primary && <span className="aipet-badge aipet-badge--primary">默认路径</span>}
            <span className="aipet-spellcard__meta">
              {g.rows} 行 × {g.cols} 格 · {g.desc}
            </span>
          </div>
          <pre className="aipet-spell">{g.text}</pre>
          <div className="aipet-actions aipet-actions--orbs">
            <CopyButton text={g.text} icon="📋" label="复制网格咒语" done="网格咒语已复制(记得附角色参考图)" primary />
            <CopyButton text={g.purified} icon="✨" label="复制净化版" done="净化版已复制" />
            <Orb
              icon="🖼"
              label="存网格引导图"
              onClick={() => void (async () => {
                try {
                  const dest = await petIpc.savePath(
                    "保存网格引导图",
                    `网格引导图_${g.rows}x${g.cols}.png`,
                    "PNG",
                    ["png"],
                  );
                  if (!dest) return;
                  await petIpc.guideSave(g.cols, g.rows, dest);
                  toast("网格引导图已保存", "success");
                } catch (e) {
                  toast(errText(e), "error");
                }
              })()}
            />
            <OpenPlatformButton url={platform.url} name={platform.name} />
          </div>
        </div>
      ))}
    </section>
  );
}

function VideoSection({ platform, note }: { platform: PlatformSpells; note: string }) {
  const v = platform.video;
  if (!v) return null;
  return (
    <section className="aipet-panel">
      <SectionHead icon="🎬" title={`视频咒语(${v.label})`} desc={v.setup} />
      {(
        [
          ["idle", "待机(真呼吸)", v.idle],
          ["walk", "右走(原地踏步)", v.walk],
        ] as const
      ).map(([key, label, text]) => (
        <div key={key} className="aipet-spellcard">
          <div className="aipet-spellcard__head">
            <span className="aipet-spellcard__title">{label}</span>
          </div>
          <pre className="aipet-spell">{text}</pre>
          <div className="aipet-actions aipet-actions--orbs">
            <CopyButton text={text} icon="📋" label="复制视频咒语" done="视频咒语已复制" primary />
          </div>
        </div>
      ))}
      <p className="aipet-note aipet-note--warn">⚠ {note}</p>
    </section>
  );
}

function SinglesSection({ platform }: { platform: PlatformSpells }) {
  const { show: toast } = useToast();
  return (
    <details className="aipet-panel aipet-details">
      <summary>单行咒语(逐状态 · 保底版式 / 专家模式用)</summary>
      {platform.singles.map((spell) => (
        <div key={spell.state} className="aipet-spellcard">
          <div className="aipet-spellcard__head">
            <span className="aipet-spellcard__title">
              {spell.state_name} <code>{spell.state}</code>
            </span>
            <span className="aipet-badge">{spell.frames} 帧</span>
            <span className="aipet-spellcard__meta">{spell.note}</span>
          </div>
          <pre className="aipet-spell">{spell.text}</pre>
          <div className="aipet-actions aipet-actions--orbs">
            <CopyButton text={spell.text} icon="📋" label="复制咒语" done="咒语已复制" primary />
            <CopyButton text={spell.purified} icon="✨" label="净化版" done="净化版已复制" />
            <CopyButton
              text={spell.rescue}
              icon="🩹" label="救帧版"
              done="救帧咒语已复制(附首帧参考图使用)"
            />
            <Orb
              icon="🖼"
              label="存布局引导图"
              onClick={() => {
                void (async () => {
                  try {
                    const dest = await petIpc.savePath(
                      "保存布局引导图",
                      `布局引导图_${spell.state_name}_${spell.frames}帧.png`,
                      "PNG",
                      ["png"],
                    );
                    if (!dest) return;
                    await petIpc.guideSave(spell.frames, 1, dest);
                    toast("引导图已保存", "success");
                  } catch (e) {
                    toast(errText(e), "error");
                  }
                })();
              }}
            />
          </div>
        </div>
      ))}
    </details>
  );
}

function GuideGenerator() {
  const [rows, setRows] = useState(2);
  const [cols, setCols] = useState(6);
  const { show: toast } = useToast();
  return (
    <section className="aipet-panel">
      <SectionHead
        icon="📏"
        title="布局引导图生成器"
        desc="豆包/即梦纯文字即可成立;引导图只在个别平台版式不达标时作为第 2 张图上传。"
      />
      <div className="aipet-form__row">
        <select value={rows} onChange={(e) => setRows(Number(e.target.value))}>
          <option value={1}>1 行(单行条)</option>
          <option value={2}>2 行(网格)</option>
        </select>
        <select value={cols} onChange={(e) => setCols(Number(e.target.value))}>
          {Array.from({ length: 8 }, (_, i) => i + 1).map((n) => (
            <option key={n} value={n}>
              {n} 格
            </option>
          ))}
        </select>
        <Button
          onClick={async () => {
            try {
              const dest = await petIpc.savePath(
                "保存布局引导图",
                `布局引导图_${rows}x${cols}.png`,
                "PNG",
                ["png"],
              );
              if (!dest) return;
              await petIpc.guideSave(cols, rows, dest);
              toast("引导图已保存", "success");
            } catch (e) {
              toast(errText(e), "error");
            }
          }}
        >
          生成并保存
        </Button>
      </div>
    </section>
  );
}

/** 拦截识别:贴入平台报错文案,命中关键词即弹本地路线引导卡。 */
function InterceptChecker({
  keywords,
  onGoto,
}: {
  keywords: string[];
  onGoto: (t: PetTab) => void;
}) {
  const [text, setText] = useState("");
  const hit = keywords.some((k) => text.includes(k));
  return (
    <section className="aipet-panel">
      <SectionHead
        icon="🛡"
        title="生成被拒?拦截自检"
        desc="知名 IP(及长得太像的形象)在所有平台都会被拦 —— 这是平台的法定义务,不可绕。原创形象不受影响。"
      />
      <textarea
        className="aipet-textarea"
        rows={2}
        placeholder="生成被拒了?把平台的报错文字贴进来,我帮你判断是不是知名形象拦截…"
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      {hit && (
        <div className="aipet-intercept">
          <strong>这是平台的知名形象拦截,不是你的操作问题 🛡</strong>
          <p>
            各平台在输出端过滤知名形象,换措辞也没用。好消息:本地路线完全不受影响 ——
            拖一张图就能在你的电脑上让它活起来(会呼吸、会走、会眨眼),全程不经过任何平台。
          </p>
          <div className="aipet-actions aipet-actions--orbs">
            <Button onClick={() => onGoto("wizard")}>🐣 用这张图本地孵化</Button>
            <Button variant="secondary" onClick={() => onGoto("pets")}>
              🖐 已孵好?去认主校准
            </Button>
          </div>
        </div>
      )}
    </section>
  );
}

/* ---------------------------------------------------------------- 小部件 */

function CopyButton({
  text,
  icon,
  label,
  done,
  primary,
}: {
  text: string;
  icon: string;
  label: string;
  done: string;
  primary?: boolean;
}) {
  const { show: toast } = useToast();
  const copy = useCallback(async () => {
    if (await copyText(text)) toast(done, "success");
    else toast("复制失败:请手动全选上面的咒语文本复制", "error");
  }, [text, done, toast]);
  return (
    <Orb
      icon={icon}
      label={label}
      kind={primary ? "primary" : ""}
      onClick={() => void copy()}
    />
  );
}

function OpenPlatformButton({ url, name }: { url: string; name: string }) {
  const { show: toast } = useToast();
  return (
    <Button
      variant="ghost"
      onClick={async () => {
        try {
          await petIpc.openUrl(url);
        } catch (e) {
          toast(errText(e), "error");
        }
      }}
    >
      ↗ 打开{name}
    </Button>
  );
}
