/** 宠物设置:行为 / 性能与隐私 / 视频导出(ffmpeg)。 */

import { useCallback, useEffect, useState } from "react";
import { Button, Switch, useToast } from "@sentenceflow/ui";
import { petIpc } from "../../pet/ipc";
import type { ExporterStatus } from "../../pet/types";
import { SectionHead } from "./common";
import { errText, usePetSettings } from "./usePetSettings";

const SCALES = [0.75, 1, 1.25, 1.5, 2];

export function PetSettingsTab() {
  const { settings, patch } = usePetSettings();
  const { show: toast } = useToast();
  const [ffmpeg, setFfmpeg] = useState<ExporterStatus | null>(null);

  const refreshFfmpeg = useCallback(() => {
    petIpc
      .exporterStatus()
      .then(setFfmpeg)
      .catch(() => setFfmpeg({ found: false, path: null }));
  }, []);

  useEffect(refreshFfmpeg, [refreshFfmpeg]);

  if (!settings) return <p className="aipet-loading">载入中…</p>;

  const on = settings.enabled;

  return (
    <div className="aipet-panels">
      <section className="aipet-panel">
        <SectionHead icon="🐾" title="宠物行为" desc="关掉总开关时这些设置仍会保留,下次开启照旧生效。" />

        <Row label="显示大小">
          <select
            value={String(settings.scale)}
            disabled={!on}
            onChange={(e) => void patch({ scale: Number(e.target.value) })}
          >
            {SCALES.map((s) => (
              <option key={s} value={s}>
                {s}×
              </option>
            ))}
          </select>
        </Row>

        <ToggleRow
          label="随机漫游"
          desc="宠物沿屏幕底边散步;关掉就安静待在原地。"
          value={settings.roam}
          disabled={!on}
          onChange={(v) => void patch({ roam: v })}
        />
        <ToggleRow
          label="点击穿透"
          desc="整窗都不拦鼠标。默认关 —— 关着时只有宠物本体挡鼠标,其余地方本来就点得穿。"
          value={settings.click_through}
          disabled={!on}
          onChange={(v) => void patch({ click_through: v })}
        />
        <ToggleRow
          label="夜间自动睡觉"
          desc="22:00–7:00 自动进入睡眠动画。"
          value={settings.night_sleep}
          disabled={!on}
          onChange={(v) => void patch({ night_sleep: v })}
        />
        <ToggleRow
          label="道具动效"
          desc="吃东西 / 提醒 / 睡觉时的饼干、闹钟、枕头月亮动画。"
          value={settings.props_enabled}
          disabled={!on}
          onChange={(v) => void patch({ props_enabled: v })}
        />
        <Row label="无操作入睡">
          <input
            className="settings-num"
            type="number"
            min={1}
            max={120}
            disabled={!on}
            value={settings.sleep_after_min}
            onChange={(e) => void patch({ sleep_after_min: Number(e.target.value) || 8 })}
          />
          <span className="aipet-unit">分钟后</span>
        </Row>
        <Row label="宠物窗">
          <Button variant="secondary" disabled={!on} onClick={() => void petIpc.visibleSet(true)}>
            显示
          </Button>
          <Button variant="secondary" disabled={!on} onClick={() => void petIpc.visibleSet(false)}>
            收起
          </Button>
        </Row>
      </section>

      <section className="aipet-panel">
        <SectionHead icon="🔒" title="性能与隐私" desc="素材处理与宠物数据全部在本机完成,不联网、不上传。" />
        <ToggleRow
          label="省电模式"
          desc="降到约 12fps 并关闭粒子。笔记本电池模式下建议开。"
          value={settings.performance_mode}
          onChange={(v) => void patch({ performance_mode: v })}
        />
        <ToggleRow
          label="下载目录监听"
          desc="默认关。开启后,仅在孵化向导等待素材期间监听系统下载文件夹,新生成的图/视频落地即自动导入 —— 只上报文件路径,不读取内容。"
          value={settings.watcher_enabled}
          onChange={(v) => void patch({ watcher_enabled: v })}
        />
      </section>

      <section className="aipet-panel">
        <SectionHead icon="🎬" title="视频导出(ffmpeg)" desc="只有「出生视频」用得到;不导视频可以不装。" />
        <div className={`aipet-note aipet-note--${ffmpeg?.found ? "ok" : "warn"}`}>
          {ffmpeg === null
            ? "检测中…"
            : ffmpeg.found
              ? `✅ 已找到 ffmpeg:${ffmpeg.path}`
              : "⚠ 未找到 ffmpeg。Windows 可执行 winget install ffmpeg,macOS 可执行 brew install ffmpeg,或在下面手动指定。"}
        </div>
        {settings.ffmpeg_path && (
          <div className="aipet-path">手动路径:{settings.ffmpeg_path}</div>
        )}
        <div className="aipet-actions">
          <Button
            variant="secondary"
            onClick={async () => {
              try {
                const picked = await petIpc.pickFile("选择 ffmpeg 可执行文件", "ffmpeg", []);
                if (!picked) return;
                await patch({ ffmpeg_path: picked });
                refreshFfmpeg();
                toast("已保存 ffmpeg 路径", "success");
              } catch (e) {
                toast(errText(e), "error");
              }
            }}
          >
            手动指定路径
          </Button>
          {settings.ffmpeg_path && (
            <Button
              variant="ghost"
              onClick={async () => {
                await patch({ ffmpeg_path: null });
                refreshFfmpeg();
                toast("已恢复自动探测", "success");
              }}
            >
              清除手动路径
            </Button>
          )}
          <Button variant="ghost" onClick={refreshFfmpeg}>
            重新检测
          </Button>
        </div>
      </section>

      <section className="aipet-panel">
        <SectionHead icon="🥚" title="关于" />
        <p className="aipet-about">
          素材规范 PetKit v0.1:8×8 图集 · 192×208 单元格 · 8 个生活状态契约 · 绿幕 #00FF00。
          宠物数据存放在句流数据目录的 <code>pet/</code> 子目录下,可用「我的宠物」页导出为
          <code>.petkit</code> 包分享给朋友。
        </p>
        <p className="aipet-about aipet-about--muted">
          注:宠物素材体积与学习数据不在一个量级,不随「设置 · 数据 · 备份」打包 ——
          需要迁移时请导出 .petkit。
        </p>
      </section>
    </div>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="settings-row">
      <span className="settings-row__label">{label}</span>
      <span className="settings-row__control">{children}</span>
    </div>
  );
}

function ToggleRow({
  label,
  desc,
  value,
  disabled,
  onChange,
}: {
  label: string;
  desc: string;
  value: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="settings-row aipet-row--toggle">
      <span className="settings-row__label">
        {label}
        <em className="aipet-desc">{desc}</em>
      </span>
      <span className="settings-row__control">
        <Switch checked={value} disabled={disabled} onChange={onChange} aria-label={label} />
      </span>
    </div>
  );
}
