/**
 * 宠物「动画之家」—— 逐元素移植自孵宠 HatchDesk 的 `src/studio/tabs/pets.ts`。
 *
 * 一只宠物 = 一间会动的 3D 栖息地(屋子 / 牢笼 / 玻璃罩,按序轮换):
 * 分层 translateZ 透视 + 光标跟随旋转;夜空圆窗、摇摆吊灯与光锥、地板地毯、
 * 呼吸起伏的宠物与同步缩放的影子、飘浮光尘;当前宠挂「当前」牌,其余打盹 Zzz。
 * 动作以「光珠」(圆形图标按钮 + 浮起气泡标签)呈现,不用传统按钮。
 *
 * 与原版的差异仅在工程层:类名加 `aipet-` 前缀防碰撞、色值走 `--sf-home-*` 令牌
 * (四主题各一套)。几何、层级、时长、缓动一字未改。
 */

import { useCallback, useRef } from "react";
import type { PointerEvent as ReactPointerEvent, ReactNode, RefObject } from "react";
import { HintDot } from "./common";

export type HomeType = "room" | "cage" | "jar";

/** 按索引轮换栖息地类型,与原版一致(TYPES[i % 3])。 */
export const HOME_TYPES: HomeType[] = ["room", "cage", "jar"];

const times = (n: number): number[] => Array.from({ length: n }, (_, i) => i);

/** 笼顶穹顶弧肋(金属)。 */
function CageDome() {
  return (
    <div className="aipet-cage-dome">
      <svg viewBox="0 0 100 56" fill="none">
        <path d="M4 56 C4 8 96 8 96 56" />
        <path d="M22 56 C24 14 76 14 78 56" />
        <path d="M50 56 L50 5" />
        <path d="M6 30 Q50 21 94 30" />
      </svg>
    </div>
  );
}

/** 藤蔓:两股茎 + 叶,垂缠在笼子上。 */
function CageVine() {
  return (
    <div className="aipet-cage-vine">
      <svg viewBox="0 0 140 112" fill="none">
        <path className="vs" d="M10 4 C40 24 24 50 46 66 S72 84 62 108" />
        <path className="vs" d="M132 8 C108 30 124 52 104 68 S88 90 96 106" />
        <g className="vl">
          <ellipse cx="34" cy="30" rx="8" ry="5" transform="rotate(-30 34 30)" />
          <ellipse cx="30" cy="52" rx="8" ry="5" transform="rotate(22 30 52)" />
          <ellipse cx="52" cy="78" rx="8" ry="5" transform="rotate(-24 52 78)" />
          <ellipse cx="61" cy="100" rx="7" ry="4.5" transform="rotate(30 61 100)" />
          <ellipse cx="118" cy="34" rx="8" ry="5" transform="rotate(28 118 34)" />
          <ellipse cx="110" cy="60" rx="8" ry="5" transform="rotate(-24 110 60)" />
          <ellipse cx="99" cy="88" rx="7" ry="4.5" transform="rotate(24 99 88)" />
        </g>
      </svg>
    </div>
  );
}

/** 宠物之后的一层(透视时在宠物后方):各栖息地的后半结构 + 柔光陈设。 */
function BackDecor({ type }: { type: HomeType }) {
  if (type === "cage") {
    return (
      <>
        <div className="aipet-cage-halo" />
        <div className="aipet-cage-bars aipet-cage-bars-back" />
        <div className="aipet-cage-base" />
      </>
    );
  }
  if (type === "room") {
    return (
      <>
        <div className="aipet-room-frame">
          <span className="aipet-room-frame-art">🌸</span>
        </div>
        <div className="aipet-room-plant">
          <span className="aipet-plant-pot" />
          <span className="aipet-plant-leaf" />
          <span className="aipet-plant-leaf" />
          <span className="aipet-plant-leaf" />
        </div>
      </>
    );
  }
  return (
    <>
      <div className="aipet-jar-base" />
      <div className="aipet-jar-moss">
        {times(3).map((i) => (
          <span key={i} className="aipet-pebble" />
        ))}
      </div>
      <div className="aipet-jar-mushroom aipet-jar-mushroom-a" />
      <div className="aipet-jar-mushroom aipet-jar-mushroom-b" />
    </>
  );
}

/** 栖息地前景框(宠物之前的最高层,卖出「关在里面」的立体感)。 */
function FrontDecor({ type }: { type: HomeType }) {
  if (type === "cage") {
    return (
      <>
        <CageVine />
        <CageDome />
        <div className="aipet-cage-finial" />
        <div className="aipet-cage-bars aipet-cage-bars-front" />
        <div className="aipet-cage-rim" />
        <div className="aipet-cage-flies">
          {times(3).map((i) => (
            <span key={i} className="aipet-firefly" />
          ))}
        </div>
      </>
    );
  }
  if (type === "jar") {
    return (
      <>
        <div className="aipet-jar-glass">
          <div className="aipet-jar-shine" />
          <div className="aipet-jar-rim" />
        </div>
        <div className="aipet-jar-spores">
          {times(3).map((i) => (
            <span key={i} className="aipet-spore" />
          ))}
        </div>
      </>
    );
  }
  return (
    <div className="aipet-room-lights">
      {times(6).map((i) => (
        <span key={i} className="aipet-bulb" />
      ))}
    </div>
  );
}


/** 领养位:栖息地网格里的最后一格空房间(拖入 / 点选 .petkit 宠物包)。 */
export function AdoptSlot({
  onFiles,
  onPick,
  useDrop,
}: {
  onFiles: (paths: string[]) => void;
  onPick: () => void;
  useDrop: (ref: RefObject<HTMLElement | null>, onDrop: (paths: string[]) => void) => void;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useDrop(ref, onFiles);
  return (
    <div
      ref={ref}
      className="aipet-home-adopt"
      role="button"
      tabIndex={0}
      onClick={onPick}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onPick();
        }
      }}
    >
      <div className="aipet-adopt-plus">＋</div>
      <div className="aipet-adopt-title">
        领养
        <HintDot text="拖入 .petkit 宠物包,或点这里选择文件" />
      </div>
    </div>
  );
}

export function PetHome({
  name,
  tier,
  meta,
  thumb,
  active,
  type,
  actions,
}: {
  name: string;
  tier: string;
  meta: string;
  thumb?: string;
  active: boolean;
  type: HomeType;
  actions: ReactNode;
}) {
  const boxRef = useRef<HTMLDivElement>(null);

  /** 光标跟随倾斜:3D box 随鼠标在场景内的位置旋转 → 每间栖息地像可绕看的立体物。 */
  const onMove = useCallback((e: ReactPointerEvent<HTMLDivElement>) => {
    const box = boxRef.current;
    if (!box) return;
    const r = e.currentTarget.getBoundingClientRect();
    const px = (e.clientX - r.left) / r.width - 0.5;
    const py = (e.clientY - r.top) / r.height - 0.5;
    box.style.transform = `rotateX(${7 - py * 13}deg) rotateY(${px * 18}deg)`;
  }, []);

  const onLeave = useCallback(() => {
    if (boxRef.current) boxRef.current.style.transform = "";
  }, []);

  return (
    <div className={`aipet-pet-home aipet-home-type-${type}${active ? " is-active" : ""}`}>
      <div className="aipet-home-3d" onPointerMove={onMove} onPointerLeave={onLeave}>
        <div className="aipet-home-box" ref={boxRef}>
          <div className="aipet-hl aipet-hl-wall" />
          <div className="aipet-hl aipet-hl-decor">
            <div className="aipet-home-window">
              <span className="aipet-home-moon" />
              <span className="aipet-home-star" />
              <span className="aipet-home-star" />
              <span className="aipet-home-star" />
            </div>
            <div className="aipet-home-lamp" />
            <div className="aipet-home-motes">
              {times(4).map((i) => (
                <span key={i} className="aipet-mote" />
              ))}
            </div>
          </div>
          <div className="aipet-hl aipet-hl-floor">
            <div className="aipet-home-floor" />
            <div className="aipet-home-rug" />
          </div>
          <div className="aipet-hl aipet-hl-back">
            <BackDecor type={type} />
          </div>
          <div className="aipet-hl aipet-hl-pet">
            <div className="aipet-home-pet">
              {thumb ? (
                <img className="aipet-home-pet-img" src={thumb} alt={name} />
              ) : (
                <span className="aipet-home-pet-img aipet-home-pet-egg" aria-hidden>
                  🥚
                </span>
              )}
              <div className="aipet-pet-shadow" />
            </div>
          </div>
          <div className="aipet-hl aipet-hl-front">
            <FrontDecor type={type} />
            {active ? (
              <div className="aipet-home-tag">当前</div>
            ) : (
              <div className="aipet-home-zzz">
                <span>z</span>
                <span>z</span>
                <span>z</span>
              </div>
            )}
          </div>
        </div>
      </div>

      <div className="aipet-home-plate">
        <span className="aipet-home-name">{name}</span>
        <span className="aipet-home-tier">{tier}</span>
        <span className="aipet-home-meta">{meta}</span>
      </div>

      <div className="aipet-home-actions">{actions}</div>
    </div>
  );
}
