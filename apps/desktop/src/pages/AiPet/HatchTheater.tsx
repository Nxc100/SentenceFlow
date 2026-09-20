/**
 * 孵化剧场 —— 移植自孵宠 HatchDesk 的 `wizard.ts::hatchStage / playHatchTheater`。
 *
 * 两件东西共用同一只纯 CSS 画的蛋:
 * · `HatchStage`  首次孵化页顶部的舞台头图(夜空 + 星 + 聚光 + 双幕布 + 摇晃的蛋);
 * · `HatchTheater` 孵化成功那一刻的全屏登场动画,2.6s 一条时间线:
 *   摇晃加剧 → 裂纹淡入 → 闪白 → 十二片彩纸放射 → 宠物弹跳登场 → 落字 → 淡出。
 *
 * 这是整个模块的签名时刻:用户拖进一张图、等了几十秒,这 2.6 秒是回报。
 * 动画节拍全部写在 CSS 的 animation-delay 里(与原版一致),这里只管挂载与收场。
 */

import { useEffect, useState } from "react";

/** 纯 CSS 蛋:蛋身 + 两只豆眼 + 两片腮红(+ 可选裂纹)。 */
function Egg({ cracked }: { cracked?: boolean }) {
  return (
    <div className="aipet-egg-body">
      <span className="aipet-egg-eye aipet-egg-eye--l" />
      <span className="aipet-egg-eye aipet-egg-eye--r" />
      <span className="aipet-egg-cheek aipet-egg-cheek--l" />
      <span className="aipet-egg-cheek aipet-egg-cheek--r" />
      {cracked && <span className="aipet-egg-crack" />}
    </div>
  );
}

/** 舞台头图:首次孵化页的"欢迎来到剧场"。 */
export function HatchStage() {
  return (
    <div className="aipet-hatch-stage" aria-hidden>
      <span className="aipet-stage-star" />
      <span className="aipet-stage-star" />
      <span className="aipet-stage-star" />
      <span className="aipet-stage-star" />
      <span className="aipet-stage-star" />
      <div className="aipet-stage-spotlight" />
      <div className="aipet-stage-curtain aipet-stage-curtain--l" />
      <div className="aipet-stage-curtain aipet-stage-curtain--r" />
      <div className="aipet-stage-floor" />
      <div className="aipet-stage-egg">
        <Egg />
      </div>
    </div>
  );
}

/** 登场动画的总时长(ms):2.6s 正片 + 0.42s 淡出,与 CSS 里的节拍对齐。 */
export const HATCH_THEATER_MS = 2600;
const FADE_MS = 420;

/**
 * 全屏登场动画。挂载即开演,演完调 `onDone`。
 * 调用方只需在孵化成功后渲染它,并在 `onDone` 里做页面跳转。
 */
export function HatchTheater({
  name,
  thumb,
  onDone,
}: {
  name: string;
  thumb?: string | null;
  onDone: () => void;
}) {
  const [leaving, setLeaving] = useState(false);

  useEffect(() => {
    const t1 = window.setTimeout(() => setLeaving(true), HATCH_THEATER_MS);
    const t2 = window.setTimeout(onDone, HATCH_THEATER_MS + FADE_MS);
    return () => {
      window.clearTimeout(t1);
      window.clearTimeout(t2);
    };
  }, [onDone]);

  return (
    <div
      className={`aipet-ht${leaving ? " aipet-ht--done" : ""}`}
      role="status"
      aria-label={`${name} 登场`}
    >
      <div className="aipet-ht__scene">
        <div className="aipet-ht__spot" />
        <div className="aipet-ht__egg">
          <Egg cracked />
        </div>
        <div className="aipet-ht__flash" />
        <div className="aipet-ht__pet">
          {thumb ? (
            <img className="aipet-ht__pet-img" src={thumb} alt={name} />
          ) : (
            <div className="aipet-ht__pet-emoji">🐣</div>
          )}
        </div>
        <div className="aipet-ht__confetti">
          {Array.from({ length: 12 }, (_, i) => (
            <span
              key={i}
              className="aipet-ht__bit"
              // 每片彩纸一个出射角,12 片刚好铺满一圈
              style={{ ["--a" as string]: `${(i / 12) * 360}deg` }}
            />
          ))}
        </div>
        <div className="aipet-ht__caption">{name} 登场!</div>
      </div>
    </div>
  );
}
