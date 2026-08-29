/**
 * 认主仪式:点选双眼 / 嘴 / 尾尖,写进 `pet.json` 的 `anchors`(单元格坐标系)。
 *
 * 解锁本地眨眼(双渲染法)、精准嘴锚(吃)、尾摆参考。约 10 秒。
 * 照片类角色可只点嘴、跳过尾尖 —— 运行时对 `pet_photo` 默认关闭卡通眨眼。
 */

import { useCallback, useEffect, useRef, useState } from "react";
import { Button, Modal, useToast } from "@sentenceflow/ui";
import { petIpc } from "../../pet/ipc";
import type { ActivePetAssets } from "../../pet/types";
import { errText } from "./usePetSettings";

interface Pt {
  x: number;
  y: number;
}

interface Step {
  key: string;
  label: string;
  color: string;
  optional?: boolean;
}

const STEPS: Step[] = [
  { key: "eyeL", label: "点它的【左眼】👈", color: "#3FB6C8" },
  { key: "eyeR", label: "点它的【右眼】👉", color: "#3FB6C8" },
  { key: "mouth", label: "点它的【嘴巴】👄", color: "#E8698A" },
  { key: "tail_tip", label: "点【尾巴尖】🐾(没有尾巴就点「跳过」)", color: "#7FB069", optional: true },
];

/** 必填锚点:双眼 + 嘴到位才算认主完成(尾尖可选)。 */
const REQUIRED = ["eyeL", "eyeR", "mouth"];

const DISPLAY_W = 300;
const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

export function CalibrateModal({
  petId,
  onClose,
  onDone,
}: {
  petId: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const imgRef = useRef<HTMLImageElement | null>(null);
  const [assets, setAssets] = useState<ActivePetAssets | null>(null);
  const [anchors, setAnchors] = useState<Record<string, Pt>>({});
  const [step, setStep] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const { show: toast } = useToast();

  // 校准要用宠物自己的图集与几何:先设为当前宠物再取 active assets
  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        await petIpc.setActive(petId);
      } catch {
        /* 已是当前宠物:继续 */
      }
      try {
        const a = await petIpc.activeAssets();
        if (!alive) return;
        if (!a) {
          setError("没有可校准的宠物");
          return;
        }
        const img = new Image();
        await new Promise<void>((res, rej) => {
          img.onload = () => res();
          img.onerror = () => rej(new Error("图集加载失败"));
          img.src = a.sheet;
        });
        if (!alive) return;
        imgRef.current = img;
        setAssets(a);
      } catch (e) {
        if (alive) setError(errText(e));
      }
    })();
    return () => {
      alive = false;
    };
  }, [petId]);

  const cell = assets?.spec.cell;
  const idleRow = assets?.spec.states["idle"]?.row ?? 0;
  const scale = cell ? DISPLAY_W / cell.w : 1;
  const displayH = cell ? Math.round(cell.h * scale) : 0;

  // 重绘:idle 首帧 + 已点锚点的圆环
  useEffect(() => {
    const canvas = canvasRef.current;
    const img = imgRef.current;
    if (!canvas || !img || !cell) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.clearRect(0, 0, DISPLAY_W, displayH);
    ctx.imageSmoothingEnabled = false;
    ctx.drawImage(img, 0, idleRow * cell.h, cell.w, cell.h, 0, 0, DISPLAY_W, displayH);
    for (const s of STEPS) {
      const p = anchors[s.key];
      if (!p) continue;
      ctx.save();
      ctx.strokeStyle = s.color;
      ctx.fillStyle = s.color;
      ctx.lineWidth = 2.5;
      ctx.beginPath();
      ctx.arc(p.x * scale, p.y * scale, 7, 0, Math.PI * 2);
      ctx.stroke();
      ctx.beginPath();
      ctx.arc(p.x * scale, p.y * scale, 2.5, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
    }
  }, [anchors, cell, displayH, idleRow, scale]);

  const onCanvasClick = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      const canvas = canvasRef.current;
      const current = STEPS[step];
      if (!canvas || !cell || !current) return;
      const rect = canvas.getBoundingClientRect();
      const ox = (e.clientX - rect.left) * (DISPLAY_W / rect.width);
      const oy = (e.clientY - rect.top) * (displayH / rect.height);
      setAnchors((prev) => ({
        ...prev,
        [current.key]: {
          x: Math.round(clamp(ox / scale, 0, cell.w)),
          y: Math.round(clamp(oy / scale, 0, cell.h)),
        },
      }));
      setStep((s) => s + 1);
    },
    [step, cell, displayH, scale],
  );

  const finish = useCallback(async () => {
    try {
      await petIpc.setAnchors(petId, anchors);
      toast("认主成功!它现在会朝你眨眼啦～", "success");
      onDone();
    } catch (e) {
      setError(errText(e));
    }
  }, [petId, anchors, onDone, toast]);

  const currentStep = STEPS[step];
  const canFinish = REQUIRED.every((k) => anchors[k]);

  return (
    <Modal open title="摸摸它,认认主人 🖐" onClose={onClose}>
      <p className="aipet-calib__lead">依次点选下面的位置,让它记住自己的五官(约 10 秒)。</p>
      {error && <div className="aipet-note aipet-note--err">{error}</div>}
      <div className="aipet-calib__hint">
        {currentStep ? currentStep.label : "都点好啦,确认一下～"}
      </div>
      <div className="aipet-calib__stage">
        {assets ? (
          <canvas
            ref={canvasRef}
            width={DISPLAY_W}
            height={displayH}
            className="aipet-calib__canvas"
            onClick={onCanvasClick}
          />
        ) : (
          <p className="aipet-loading">载入图集…</p>
        )}
      </div>
      <div className="aipet-actions aipet-actions--center">
        <Button
          variant="ghost"
          disabled={step === 0}
          onClick={() => {
            const prevStep = step - 1;
            const key = STEPS[prevStep]?.key;
            setStep(prevStep);
            if (key) {
              setAnchors((prev) => {
                const next = { ...prev };
                delete next[key];
                return next;
              });
            }
          }}
        >
          上一步
        </Button>
        <Button
          variant="ghost"
          disabled={!currentStep?.optional}
          onClick={() => setStep((s) => s + 1)}
        >
          跳过
        </Button>
        <Button variant="secondary" onClick={onClose}>
          取消
        </Button>
        <Button disabled={!canFinish} onClick={() => void finish()}>
          完成认主
        </Button>
      </div>
    </Modal>
  );
}
