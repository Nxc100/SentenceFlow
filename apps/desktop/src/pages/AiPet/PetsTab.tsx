/**
 * 我的宠物:宠物库、设为当前、认主校准、骨骼烘焙、`.petkit` 导入导出、出生视频。
 *
 * 名片/水印/AI 标识三张贴纸由 canvas 现画成 PNG data URL 交给 Rust 侧合成 ——
 * 「内容由 AI 辅助生成」的标识按《标识办法》全版本叠加,不做可选项。
 */

import { useCallback, useEffect, useState } from "react";
import { Button, Modal, useToast } from "@sentenceflow/ui";
import { petEvents, petIpc } from "../../pet/ipc";
import type { PetMeta } from "../../pet/types";
import { CalibrateModal } from "./CalibrateModal";
import { Orb, Progress, ReportView, SectionHead } from "./common";
import { AdoptSlot, HOME_TYPES, PetHome } from "./PetHome";
import { useDropZone } from "./useDropZone";
import type { PetTab } from "./index";
import { errText, usePetSettings } from "./usePetSettings";
import type { Report } from "../../pet/types";

export function PetsTab({ onGoto }: { onGoto: (t: PetTab) => void }) {
  const { settings } = usePetSettings();
  const [pets, setPets] = useState<PetMeta[]>([]);
  const [thumbs, setThumbs] = useState<Record<string, string>>({});
  const [calibrating, setCalibrating] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<Confirm | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [importReport, setImportReport] = useState<Report | null>(null);
  const [exportPct, setExportPct] = useState<number | null>(null);
  const { show: toast } = useToast();

  const reload = useCallback(async () => {
    try {
      const list = await petIpc.list();
      setPets(list);
      const entries = await Promise.all(
        list.map(async (p) => {
          try {
            return [p.id, await petIpc.thumb(p.id)] as const;
          } catch {
            return [p.id, ""] as const;
          }
        }),
      );
      setThumbs(Object.fromEntries(entries));
    } catch (e) {
      toast(errText(e), "error");
    }
  }, [toast]);

  useEffect(() => {
    void reload();
    const un = petEvents.onChanged(() => void reload());
    return () => void un.then((f) => f());
  }, [reload]);

  useEffect(() => {
    const un = petEvents.onExportProgress((p) =>
      setExportPct(p.total > 0 ? (p.done / p.total) * 100 : 0),
    );
    return () => void un.then((f) => f());
  }, []);

  const doImport = useCallback(
    async (path: string) => {
      setImportReport(null);
      try {
        const report = await petIpc.kitValidate(path);
        if (!report.ok) {
          setImportReport(report);
          toast("宠物包未通过校验", "error");
          return;
        }
        const warns = report.findings.filter((f) => f.level === "warning");
        if (warns.length > 0) setImportReport(report);
        await petIpc.kitImport(path);
        toast("导入成功,已上桌面", "success");
        void reload();
      } catch (e) {
        toast(errText(e), "error");
      }
    },
    [reload, toast],
  );

  const activeId = settings?.active_pet ?? null;

  return (
    <div className="aipet-panels">
      <section className="aipet-panel">
        <SectionHead
          icon="🏠"
          title={`我的宠物(${pets.length})`}
          desc="设为当前的那只会出现在桌面上;其余的在库里等着。"
        />

        {/* 栖息地网格:每只宠物一间会动的 3D 房间,最后一格是领养位(与原版同构) */}
        <div className="aipet-homes-grid">
          {pets.length === 0 && (
            <div className="aipet-home-empty">
              <div className="aipet-empty-egg">🥚</div>
              <div>
                空荡荡的 ·{" "}
                <button type="button" className="aipet-link" onClick={() => onGoto("wizard")}>
                  去孵一只
                </button>
              </div>
            </div>
          )}

          {pets.map((pet, i) => (
            <PetHome
              key={pet.id}
              name={pet.name}
              tier={pet.tier}
              meta={`${pet.states.length} 个状态${pet.created ? ` · ${pet.created.slice(0, 10)}` : ""}`}
              thumb={thumbs[pet.id] || undefined}
              active={pet.id === activeId}
              type={HOME_TYPES[i % HOME_TYPES.length] ?? "room"}
              actions={
                <>
                  {pet.id !== activeId && (
                    <Orb
                      icon="🏠"
                      label="设为当前"
                      kind="primary"
                      onClick={() => {
                        void (async () => {
                          try {
                            // pet_set_active 内部已写回设置分节并广播 pet://settings,
                            // 这里再 patch 一次只会制造一次多余的往返与竞态。
                            await petIpc.setActive(pet.id);
                            toast(`${pet.name} 已上桌面`, "success");
                          } catch (e) {
                            toast(errText(e), "error");
                          }
                        })();
                      }}
                    />
                  )}
                  <Orb icon="🖐" label="认主校准" onClick={() => setCalibrating(pet.id)} />
                  <Orb
                    icon="✨"
                    label="本地动画"
                    onClick={() =>
                      setConfirming({
                        title: "本地零生成动画",
                        body: `「${pet.name}」将在本地生成会走、会跳、会呼吸的骨骼动画(不经过任何平台,约几秒)。这会覆盖它现有的动画帧。`,
                        confirmLabel: "开始烘焙",
                        run: async () => {
                          setBusy(`正在为 ${pet.name} 烘焙骨骼动画…`);
                          try {
                            await petIpc.rigBake(pet.id);
                            toast("烘焙完成!它现在会走会跳啦 ✨", "success");
                            void reload();
                          } finally {
                            setBusy(null);
                          }
                        },
                      })
                    }
                  />
                  <Orb
                    icon="🎬"
                    label="出生视频"
                    onClick={() => void exportVideo(pet, setBusy, setExportPct, toast)}
                  />
                  <Orb
                    icon="📤"
                    label="导出宠物包"
                    onClick={() => {
                      void (async () => {
                        try {
                          const dest = await petIpc.savePath(
                            "导出宠物包",
                            `${safeName(pet.name)}.petkit`,
                            "宠物包",
                            ["petkit"],
                          );
                          if (!dest) return;
                          await petIpc.kitExport(pet.id, dest);
                          toast("宠物包已导出,可以分享给朋友了", "success");
                          void petIpc.revealInDir(dest).catch(() => undefined);
                        } catch (e) {
                          toast(errText(e), "error");
                        }
                      })();
                    }}
                  />
                  <Orb
                    icon="🗑"
                    label="删除"
                    kind="danger"
                    onClick={() =>
                      setConfirming({
                        title: "删除宠物",
                        body: `确定删除「${pet.name}」吗?它的图集与素材源帧会一并抹掉,不可恢复。`,
                        confirmLabel: "删除",
                        danger: true,
                        run: async () => {
                          await petIpc.remove(pet.id);
                          toast("已删除", "success");
                          void reload();
                        },
                      })
                    }
                  />
                </>
              }
            />
          ))}

          <AdoptSlot
            useDrop={useDropZone}
            onFiles={(paths) => {
              const first = paths[0];
              if (first) void doImport(first);
            }}
            onPick={() => {
              void (async () => {
                const picked = await petIpc.pickFile("选择宠物包", "宠物包", ["petkit", "zip"]);
                if (picked) void doImport(picked);
              })();
            }}
          />
        </div>

        <ReportView report={importReport} />
      </section>

      {calibrating && (
        <CalibrateModal
          petId={calibrating}
          onClose={() => setCalibrating(null)}
          onDone={() => {
            setCalibrating(null);
            void reload();
          }}
        />
      )}

      {confirming && (
        <ConfirmModal
          confirm={confirming}
          onClose={() => setConfirming(null)}
          onError={(m) => toast(m, "error")}
        />
      )}

      <Modal open={busy !== null} title="请稍候" onClose={() => undefined}>
        <p className="aipet-busy">{busy}</p>
        {exportPct !== null && (
          <Progress pct={exportPct} label={`${Math.round(exportPct)}%`} />
        )}
      </Modal>
    </div>
  );
}

/* ---------------------------------------------------------------- 确认框 */

interface Confirm {
  title: string;
  body: string;
  confirmLabel: string;
  danger?: boolean;
  run: () => Promise<void>;
}

function ConfirmModal({
  confirm,
  onClose,
  onError,
}: {
  confirm: Confirm;
  onClose: () => void;
  onError: (msg: string) => void;
}) {
  const [running, setRunning] = useState(false);
  return (
    <Modal open title={confirm.title} onClose={onClose}>
      <p className="aipet-confirm">{confirm.body}</p>
      <div className="aipet-actions aipet-actions--end">
        <Button variant="secondary" onClick={onClose} disabled={running}>
          取消
        </Button>
        <Button
          variant={confirm.danger ? "secondary" : "primary"}
          disabled={running}
          onClick={async () => {
            setRunning(true);
            try {
              await confirm.run();
              onClose();
            } catch (e) {
              onError(errText(e));
              onClose();
            } finally {
              setRunning(false);
            }
          }}
        >
          {confirm.confirmLabel}
        </Button>
      </div>
    </Modal>
  );
}

/* ---------------------------------------------------------------- 出生视频 */

async function exportVideo(
  pet: PetMeta,
  setBusy: (s: string | null) => void,
  setPct: (n: number | null) => void,
  toast: (m: string, k?: "info" | "success" | "error") => void,
): Promise<void> {
  try {
    const status = await petIpc.exporterStatus();
    if (!status.found) {
      toast("未找到 ffmpeg:先安装,或在「宠物设置」里手动指定路径", "error");
      return;
    }
    const dest = await petIpc.savePath(
      "保存出生视频",
      `${safeName(pet.name)}_出生视频.mp4`,
      "视频",
      ["mp4"],
    );
    if (!dest) return;

    setPct(0);
    setBusy(`正在为 ${pet.name} 合成出生视频…`);
    await petIpc.exportBirthVideo(pet.id, dest, makeNameCard(pet.name), null, makeAiNotice());
    toast("出生视频导出完成 🎉", "success");
    void petIpc.revealInDir(dest).catch(() => undefined);
  } catch (e) {
    toast(errText(e), "error");
  } finally {
    setBusy(null);
    setPct(null);
  }
}

/** Windows 文件名禁用字符(取各平台的超集)。 */
function safeName(name: string): string {
  const s = name.replace(/[/\\:*?"<>|]/g, "").trim();
  return s || "宠物";
}

function canvasPng(w: number, h: number, draw: (ctx: CanvasRenderingContext2D) => void): string {
  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  draw(ctx);
  return canvas.toDataURL("image/png");
}

/** 落版名片:白卡 + 宠物名 + 出生日期。 */
function makeNameCard(name: string): string {
  return canvasPng(560, 250, (ctx) => {
    ctx.fillStyle = "rgba(255,255,255,0.92)";
    ctx.beginPath();
    ctx.roundRect(6, 6, 548, 238, 28);
    ctx.fill();
    ctx.strokeStyle = "#543A2C";
    ctx.lineWidth = 5;
    ctx.stroke();

    ctx.fillStyle = "#3C2A20";
    ctx.textAlign = "center";
    ctx.font = "bold 64px 'Microsoft YaHei UI', 'PingFang SC', sans-serif";
    ctx.fillText(name, 280, 118);
    ctx.font = "26px 'Microsoft YaHei UI', sans-serif";
    ctx.fillStyle = "#8a7360";
    const t = new Date();
    ctx.fillText(`· 出生于 ${t.getFullYear()}.${t.getMonth() + 1}.${t.getDate()} ·`, 280, 172);
    ctx.font = "22px 'Segoe UI Emoji', sans-serif";
    ctx.fillText("🥚 ✨ 🐣", 280, 214);
  });
}

/** AI 生成内容显式标识(《标识办法》):所有导出一律叠加,不做开关。 */
function makeAiNotice(): string {
  return canvasPng(190, 40, (ctx) => {
    ctx.globalAlpha = 0.75;
    ctx.fillStyle = "rgba(0,0,0,0.45)";
    ctx.beginPath();
    ctx.roundRect(0, 0, 190, 40, 12);
    ctx.fill();
    ctx.globalAlpha = 1;
    ctx.fillStyle = "#fff";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.font = "17px 'Microsoft YaHei UI', sans-serif";
    ctx.fillText("内容由 AI 辅助生成", 95, 21);
  });
}
