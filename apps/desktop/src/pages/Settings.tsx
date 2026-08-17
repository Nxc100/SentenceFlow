/**
 * 设置(§4.7/§4.8):练习 / 声音 / 外观 / 无障碍 · AI 接入(四通道卡)
 * · 授权(激活卡) · 数据(备份/恢复/试用导入/诊断包)。
 */

import { useCallback, useEffect, useState } from "react";
import { Button, Switch, levelName, useToast } from "@sentenceflow/ui";
import { useApp } from "../appState";
import type { ChannelId, ChannelStatus, ModelInfo, PlacementResult, Settings } from "../ipc";
import { ipc } from "../ipc";

type Section = "general" | "ai" | "license" | "data";

export function SettingsPage({ onPlacement }: { onPlacement?: () => void }) {
  const [section, setSection] = useState<Section>("general");
  return (
    <div className="page page--settings">
      <header className="page__header">
        <h1>设置</h1>
      </header>
      <div className="settings-layout">
        <nav className="settings-nav">
          {(
            [
              ["general", "练习与外观"],
              ["ai", "AI 接入"],
              ["license", "授权"],
              ["data", "数据"],
            ] as Array<[Section, string]>
          ).map(([key, label]) => (
            <button
              key={key}
              type="button"
              className={`settings-nav__item${section === key ? " settings-nav__item--on" : ""}`}
              onClick={() => setSection(key)}
            >
              {label}
            </button>
          ))}
        </nav>
        <div className="settings-body">
          {section === "general" && <GeneralSection onPlacement={onPlacement} />}
          {section === "ai" && <AiSection />}
          {section === "license" && <LicenseSection />}
          {section === "data" && <DataSection />}
        </div>
      </div>
    </div>
  );
}

/* ---------------- general ---------------- */

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="settings-row">
      <span className="settings-row__label">{label}</span>
      <span className="settings-row__control">{children}</span>
    </div>
  );
}

function GeneralSection({ onPlacement }: { onPlacement?: () => void }) {
  const { settings, updateSettings, specFor, level } = useApp();
  const [lastPlacement, setLastPlacement] = useState<PlacementResult | null>(null);
  const set = (patch: (s: Settings) => void) =>
    void updateSettings((s) => {
      patch(s);
      return s;
    });

  useEffect(() => {
    void ipc.placementLast().then(setLastPlacement);
  }, []);

  return (
    <>
      <h2>我的水平</h2>
      <Row label={`当前等级:${levelName(level)}`}>
        <Button variant="secondary" onClick={() => onPlacement?.()}>
          重新测一下我的水平
        </Button>
      </Row>
      <p className="settings-hint">
        {lastPlacement
          ? `上次测试:推荐「${levelName(lastPlacement.level)}」· 词汇量约 ${lastPlacement.vocab_est} 词`
          : "约 3 分钟:认词 + 打整句 + 选语法,测完给出适合你的起点。"}
        {specFor(level) ? ` · 当前等级能做到:${specFor(level)?.can_do.join("、")}` : ""}
      </p>

      <h2>练习</h2>
      <Row label="严格打字(错字不上屏)">
        <Switch
          checked={settings.practice.strict_typing}
          onChange={(v) => set((s) => (s.practice.strict_typing = v))}
        />
      </Row>
      <Row label="自动朗读答案">
        <Switch
          checked={settings.practice.auto_speak_answer}
          onChange={(v) => set((s) => (s.practice.auto_speak_answer = v))}
        />
      </Row>
      <Row label="隐藏中文题面">
        <Switch
          checked={settings.practice.hide_chinese}
          onChange={(v) => set((s) => (s.practice.hide_chinese = v))}
        />
      </Row>
      <Row label="每日新句数(留空 = 随等级)">
        <input
          className="settings-num"
          type="number"
          min={5}
          max={50}
          value={settings.practice.daily_new ?? ""}
          placeholder="自动"
          onChange={(e) =>
            set((s) => (s.practice.daily_new = e.target.value === "" ? null : Number(e.target.value)))
          }
        />
      </Row>

      <h2>声音</h2>
      <Row label="口音">
        <select
          value={settings.sound.accent}
          onChange={(e) => set((s) => (s.sound.accent = e.target.value as "gb" | "us"))}
        >
          <option value="gb">英音</option>
          <option value="us">美音</option>
        </select>
      </Row>
      <Row label={`语速 ${settings.sound.rate.toFixed(1)}×`}>
        <input
          type="range"
          min={0.6}
          max={1.4}
          step={0.1}
          value={settings.sound.rate}
          onChange={(e) => set((s) => (s.sound.rate = Number(e.target.value)))}
        />
      </Row>
      <Row label="按键音">
        <select
          value={settings.sound.key_sound}
          onChange={(e) =>
            set((s) => (s.sound.key_sound = e.target.value as Settings["sound"]["key_sound"]))
          }
        >
          <option value="off">关</option>
          <option value="soft">软触</option>
          <option value="mechanical">机械</option>
        </select>
      </Row>

      <h2>外观</h2>
      <Row label="主题">
        <select
          value={settings.appearance.theme}
          onChange={(e) =>
            set((s) => (s.appearance.theme = e.target.value as Settings["appearance"]["theme"]))
          }
        >
          <option value="system">跟随系统</option>
          <option value="light">浅色</option>
          <option value="dark">深色</option>
        </select>
      </Row>
      <Row label="护眼纸色(浅色下生效)">
        <Switch
          checked={settings.appearance.paper}
          onChange={(v) => set((s) => (s.appearance.paper = v))}
        />
      </Row>

      <h2>无障碍</h2>
      <Row label="减少动效">
        <select
          value={settings.accessibility.reduce_motion}
          onChange={(e) =>
            set(
              (s) =>
                (s.accessibility.reduce_motion = e.target
                  .value as Settings["accessibility"]["reduce_motion"]),
            )
          }
        >
          <option value="system">跟随系统</option>
          <option value="on">开</option>
          <option value="off">关</option>
        </select>
      </Row>
      <Row label="OpenDyslexic 字体(练习区英文)">
        <Switch
          checked={settings.accessibility.dyslexic_font}
          onChange={(v) => set((s) => (s.accessibility.dyslexic_font = v))}
        />
      </Row>
    </>
  );
}

/* ---------------- AI 接入 (§4.7) ---------------- */

const CHANNELS: Array<{ id: ChannelId; name: string; desc: string }> = [
  { id: "opencode", name: "opencode 本地", desc: "驱动本机 opencode CLI,免费模型零费用" },
  { id: "deepseek", name: "DeepSeek 官方", desc: "质量与稳定基准,按量计费" },
  { id: "zen", name: "Zen 直连", desc: "不装 CLI 也想用免费模型的折中" },
  { id: "ollama", name: "Ollama 本地", desc: "全离线,本机模型" },
];

function AiSection() {
  const { settings, updateSettings } = useApp();
  const toast = useToast();
  const [statuses, setStatuses] = useState<Partial<Record<ChannelId, ChannelStatus>>>({});
  const [keys, setKeys] = useState<Partial<Record<ChannelId, string>>>({});
  const [testing, setTesting] = useState<ChannelId | null>(null);

  const probe = useCallback(async (id: ChannelId) => {
    setStatuses((s) => ({ ...s, [id]: undefined }));
    const status = await ipc.probeChannel(id).catch(
      (e): ChannelStatus => ({ state: "error", message: String((e as { message?: string }).message ?? e) }),
    );
    setStatuses((s) => ({ ...s, [id]: status }));
    return status;
  }, []);

  useEffect(() => {
    for (const c of CHANNELS) void probe(c.id);
  }, [probe]);

  // 旧版本存的设置没有 model_label:探测到模型列表后自动补写展示名。
  useEffect(() => {
    const status = settings.ai.channel ? statuses[settings.ai.channel] : undefined;
    if (!settings.ai.model || settings.ai.model_label || status?.state !== "ready") return;
    const name = status.models.find((m) => m.id === settings.ai.model)?.display_name;
    if (name) {
      void updateSettings((s) => {
        s.ai.model_label = name;
        return s;
      });
    }
  }, [statuses, settings.ai.channel, settings.ai.model, settings.ai.model_label, updateSettings]);

  /** 选通道/模型;顺手记下模型展示名,界面各处显示友好名而非原始 id */
  const select = (id: ChannelId, model?: string, models?: ModelInfo[]) =>
    void updateSettings((s) => {
      s.ai.channel = id;
      if (model !== undefined) {
        s.ai.model = model;
        s.ai.model_label = models?.find((m) => m.id === model)?.display_name ?? null;
      }
      return s;
    });

  /** 默认模型:优先直连可用的第一个(面向国内用户;需代理模型仍可手选) */
  const defaultModel = (models: ModelInfo[]) =>
    (models.find((m) => !m.needs_proxy) ?? models[0])?.id;

  const dot = (status: ChannelStatus | undefined) => {
    if (!status) return <span className="ch-dot ch-dot--probing" title="检测中" />;
    switch (status.state) {
      case "ready":
        return <span className="ch-dot ch-dot--ready" title="就绪" />;
      case "not_authed":
        return <span className="ch-dot ch-dot--auth" title="未登录/未配置" />;
      case "not_installed":
        return <span className="ch-dot ch-dot--off" title="未安装/未启动" />;
      case "error":
        return <span className="ch-dot ch-dot--err" title={status.message} />;
    }
  };

  return (
    <>
      <h2>AI 接入</h2>
      <p className="settings-hint">
        任一通道就绪即解锁生成工坊、答疑与周点评。<b>不配置也不影响任何学习功能。</b>
      </p>
      {CHANNELS.map((c) => {
        const status = statuses[c.id];
        const selected = settings.ai.channel === c.id;
        const models: ModelInfo[] = status?.state === "ready" ? status.models : [];
        const needsKey = c.id === "deepseek" || c.id === "zen";
        return (
          <div key={c.id} className={`channel-card${selected ? " channel-card--on" : ""}`}>
            <div className="channel-card__head">
              <label className="channel-card__title">
                <input
                  type="radio"
                  name="channel"
                  checked={selected}
                  disabled={status?.state !== "ready"}
                  onChange={() => select(c.id, defaultModel(models), models)}
                />
                <span className="channel-card__name">{c.name}</span>
                {dot(status)}
              </label>
              <Button variant="ghost" onClick={() => void probe(c.id)}>
                重新检测
              </Button>
            </div>
            <p className="channel-card__desc">{c.desc}</p>

            {c.id === "opencode" && status?.state === "not_installed" && (
              <InlineCommand label="安装命令" cmd="curl -fsSL https://opencode.ai/install | bash" />
            )}
            {c.id === "opencode" && status?.state === "not_authed" && (
              <InlineCommand label="终端登录" cmd="opencode auth login" />
            )}
            {c.id === "ollama" && status?.state === "not_installed" && (
              <p className="channel-card__err">未检测到本地服务(11434) · <a href="https://ollama.com" target="_blank" rel="noreferrer">如何安装</a></p>
            )}
            {status?.state === "error" && <p className="channel-card__err">{status.message}</p>}

            {needsKey && status?.state !== "ready" && (
              <div className="channel-card__keyrow">
                <input
                  type="password"
                  className="key-input"
                  placeholder="sk-••••••••"
                  value={keys[c.id] ?? ""}
                  onChange={(e) => setKeys((k) => ({ ...k, [c.id]: e.target.value }))}
                />
                <Button
                  variant="secondary"
                  disabled={testing === c.id || !(keys[c.id] ?? "").trim()}
                  onClick={async () => {
                    setTesting(c.id);
                    try {
                      const result = await ipc.testChannelKey(c.id, (keys[c.id] ?? "").trim());
                      setStatuses((s) => ({ ...s, [c.id]: result }));
                      if (result.state === "ready") {
                        toast.show("已连通,生成工坊可用");
                        setKeys((k) => ({ ...k, [c.id]: "" }));
                      } else if (result.state === "error") {
                        toast.show(result.message);
                      } else {
                        toast.show("Key 无效,请检查是否复制完整");
                      }
                    } finally {
                      setTesting(null);
                    }
                  }}
                >
                  {testing === c.id ? "测试中…" : "测试连接"}
                </Button>
              </div>
            )}

            {selected &&
              models.length > 0 &&
              (() => {
                const currentId = settings.ai.model ?? defaultModel(models);
                const current = models.find((m) => m.id === currentId);
                const hasProxy = !!settings.ai.proxy_url?.trim();
                return (
                  <>
                    <div className="channel-card__models">
                      <select value={currentId} onChange={(e) => select(c.id, e.target.value, models)}>
                        {models.map((m) => (
                          <option key={m.id} value={m.id}>
                            {m.display_name}
                            {m.needs_proxy ? " 🔒 需代理" : " · 直连可用"}
                          </option>
                        ))}
                      </select>
                      {current?.terms_note && (
                        <span className="channel-card__terms">{current.terms_note}</span>
                      )}
                      {(c.id === "opencode" || c.id === "zen") && (
                        <BenchButton onPicked={(m) => select(c.id, m, models)} />
                      )}
                    </div>
                    {current?.needs_proxy && !hasProxy && (
                      <p className="channel-card__err">
                        该模型国内直连不可用 —— 请换选标注「直连可用」的模型,或在下方
                        「网络」区填写本机代理端口。
                      </p>
                    )}
                  </>
                );
              })()}

            {c.id === "opencode" && (
              <p className="channel-card__fineprint">
                免费模型为限时活动,名单与限速由 Zen 决定 · 本软件不读取你的 opencode 凭据
              </p>
            )}
          </div>
        );
      })}

      <h2>网络</h2>
      <Row label="AI 代理(可选)">
        <input
          className="settings-path"
          style={{ maxWidth: 260 }}
          placeholder="http://127.0.0.1:7890"
          defaultValue={settings.ai.proxy_url ?? ""}
          onBlur={(e) =>
            void updateSettings((s) => {
              const v = e.target.value.trim();
              s.ai.proxy_url = v === "" ? null : v;
              return s;
            })
          }
        />
      </Row>
      <p className="settings-hint">
        直连网络无法访问 opencode / Zen 境外端点时,填写本机代理客户端的 HTTP
        端口(无需开全局代理);仅 AI 请求走此代理。DeepSeek 官方与 Ollama
        本地直连可用,不受影响。改完点[重新检测]生效。
      </p>

      <h2>预算</h2>
      <Row label="单次生成上限(¥)">
        <input
          className="settings-num"
          type="number"
          min={0.1}
          step={0.5}
          value={settings.ai.per_run_budget_cny}
          onChange={(e) =>
            void updateSettings((s) => {
              s.ai.per_run_budget_cny = Number(e.target.value) || 1;
              return s;
            })
          }
        />
      </Row>
      <p className="settings-hint">费用预估仅供参考,以官方账单为准。Key 仅存本机系统钥匙串,不随备份导出。</p>
    </>
  );
}

/** 微基准择优(§3.5):每个候选模型生成 6 句 → 本地打分 → 默认选最高分。 */
function BenchButton({ onPicked }: { onPicked: (model: string) => void }) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  return (
    <Button
      variant="ghost"
      disabled={busy}
      onClick={async () => {
        setBusy(true);
        toast.show("正在为你试用各个模型并挑最合适的,约 30 秒,只需一次");
        try {
          const ranking = await ipc.runBench();
          const best = ranking[0];
          if (best) {
            onPicked(best.model);
            toast.show("已为你选好效果最好的模型");
          } else {
            toast.show("暂时没有可试用的模型");
          }
        } catch (e) {
          toast.show(String((e as { message?: string }).message ?? e));
        } finally {
          setBusy(false);
        }
      }}
    >
      {busy ? "挑选中…" : "帮我选模型"}
    </Button>
  );
}

function InlineCommand({ label, cmd }: { label: string; cmd: string }) {
  const toast = useToast();
  return (
    <div className="inline-cmd">
      <span>{label}:</span>
      <code>{cmd}</code>
      <Button
        variant="ghost"
        onClick={() => {
          void navigator.clipboard.writeText(cmd);
          toast.show("已复制");
        }}
      >
        复制
      </Button>
    </div>
  );
}

/* ---------------- 授权 (§4.6) ---------------- */

function LicenseSection() {
  const { license, refreshLicense } = useApp();
  const toast = useToast();
  const [pasted, setPasted] = useState("");
  const [shake, setShake] = useState(0);
  const [error, setError] = useState<string | null>(null);

  if (license.kind === "licensed") {
    return (
      <div className="license-card license-card--active">
        <span className="license-stamp">已激活</span>
        <p className="license-email">{license.email_masked}</p>
        <p className="license-edition">
          {license.edition} · 覆盖到 v{license.major_max}.x
        </p>
        <Button
          variant="ghost"
          onClick={async () => {
            const doc = await ipc.exportLicense();
            const blob = new Blob([doc], { type: "application/json" });
            const a = document.createElement("a");
            a.href = URL.createObjectURL(blob);
            a.download = "sentenceflow-license.sflic";
            a.click();
          }}
        >
          导出备份
        </Button>
        <p className="settings-hint">换机?把 .sflic 文件拷过去,粘贴激活即可。</p>
      </div>
    );
  }

  return (
    <div key={shake} className={`license-card license-card--empty${shake > 0 ? " license-card--shake" : ""}`}>
      {license.kind === "lapsed" && license.clock_rollback && (
        <p className="license-rollback">
          检测到系统时间曾经回拨,试用已结束。激活后不再受影响。
        </p>
      )}
      <p>把邮件里的许可证内容粘贴到这里:</p>
      <textarea
        rows={5}
        className="license-paste"
        placeholder='{"v":1,"product":"sentenceflow", … }'
        value={pasted}
        onChange={(e) => setPasted(e.target.value)}
      />
      {error && <p className="license-error">{error}</p>}
      <Button
        onClick={async () => {
          setError(null);
          try {
            await ipc.activateLicense(pasted);
            await refreshLicense();
            toast.show("激活成功,感谢支持!");
          } catch (e) {
            setShake((v) => v + 1);
            setError(String((e as { message?: string }).message ?? e));
          }
        }}
        disabled={!pasted.trim()}
      >
        激活
      </Button>
    </div>
  );
}

/* ---------------- 数据 (§4.8) ---------------- */

function DataSection() {
  const toast = useToast();
  const [restorePath, setRestorePath] = useState("");
  const [preview, setPreview] = useState<{ srs_incoming: number; srs_newer: number; logs_incoming: number } | null>(null);

  return (
    <>
      <h2>备份</h2>
      <p className="settings-hint">备份包含学习进度与你的句库,不含任何密钥。</p>
      <Button
        onClick={async () => {
          const name = `sentenceflow-backup-${new Date().toISOString().slice(0, 10)}.zip`;
          const path = await ipc.backupExport(name);
          toast.show(`已导出:${path}`);
        }}
      >
        导出备份 zip
      </Button>

      <h2>恢复</h2>
      <div className="settings-restore">
        <input
          className="settings-path"
          placeholder="备份 zip 的完整路径"
          value={restorePath}
          onChange={(e) => setRestorePath(e.target.value)}
        />
        <Button
          variant="secondary"
          disabled={!restorePath.trim()}
          onClick={async () => {
            try {
              setPreview(await ipc.backupRestore(restorePath.trim(), false));
            } catch (e) {
              toast.show(String((e as { message?: string }).message ?? e));
            }
          }}
        >
          预览差异
        </Button>
      </div>
      {preview && (
        <div className="settings-preview">
          <p>
            将合并 {preview.srs_newer} 条较新进度(共 {preview.srs_incoming} 条)与{" "}
            {preview.logs_incoming} 条练习记录。
          </p>
          <Button
            onClick={async () => {
              await ipc.backupRestore(restorePath.trim(), true);
              setPreview(null);
              toast.show("恢复完成");
            }}
          >
            确认恢复(重复的进度保留较新一条)
          </Button>
        </div>
      )}

      <h2>试用版进度导入</h2>
      <ImportTrial />

      <h2>诊断</h2>
      <Button
        variant="ghost"
        onClick={async () => {
          const d = await ipc.diagnostics();
          const blob = new Blob([JSON.stringify(d, null, 2)], { type: "application/json" });
          const a = document.createElement("a");
          a.href = URL.createObjectURL(blob);
          a.download = "sentenceflow-diagnostics.json";
          a.click();
        }}
      >
        导出匿名诊断包
      </Button>
    </>
  );
}

function ImportTrial() {
  const toast = useToast();
  return (
    <input
      type="file"
      accept="application/json"
      onChange={async (e) => {
        const file = e.target.files?.[0];
        if (!file) return;
        try {
          const json = JSON.parse(await file.text());
          const [merged, skipped] = await ipc.importTrialProgress(json);
          toast.show(`已合并 ${merged} 条进度${skipped > 0 ? `,${skipped} 条未匹配` : ""}`);
        } catch (err) {
          toast.show(String((err as { message?: string }).message ?? err));
        }
      }}
    />
  );
}
