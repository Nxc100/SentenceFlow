/**
 * 练习音效(§6.5):WebAudio 合成,无音频资源文件。
 * - 按键音三音色(关 / 软触 / 机械),随机 ±3% 音高;
 * - 正确"叮" 880Hz · 90ms · −14dB;错误低音 220Hz,刻意更轻;
 * - 掌握上行双音(§6.3 激活/掌握用)。
 * AudioContext 惰性创建于首次用户输入(浏览器自动播放策略);
 * 效果音不叠播:同类音效重触发时先截断上一次(§6.5)。
 */

export type KeySoundKind = "off" | "soft" | "mechanical";

export interface SoundSettings {
  keySound: KeySoundKind;
  /** 0–100 (§4.8 效果音量,默认 70) */
  fxVolume: number;
}

/** 组件层依赖的最小接口(便于测试与静音注入) */
export interface SoundPlayer {
  key(): void;
  error(): void;
  correct(): void;
  /** 签名时刻的庆祝音(上行三音);单点"叮"撑不起"整句答对"的分量 */
  celebrate(): void;
  master(): void;
}

export const silentSounds: SoundPlayer = {
  key() {},
  error() {},
  correct() {},
  celebrate() {},
  master() {},
};

/**
 * 增益基准。早期版本按 −14dB(0.2)直译规范导致整体听感过小
 * (用户反馈),现整体提升并保持规范的相对关系:错误音刻意比正确音轻,
 * 按键音最轻但清晰可闻;系统/应用音量在此之上自然叠加。
 */
const DING_GAIN = 0.5;
const KEY_GAIN = 0.28;
const ERROR_GAIN = 0.3;
const MASTER_GAIN = 0.42;
const CELEBRATE_GAIN = 0.46;

export class WebAudioSounds implements SoundPlayer {
  private ctx: AudioContext | null = null;
  private settings: SoundSettings;
  /** 每类音效当前的增益节点,重触发时截断(不叠播) */
  private active: Partial<Record<"fx", GainNode>> = {};

  constructor(settings?: Partial<SoundSettings>) {
    this.settings = { keySound: "soft", fxVolume: 70, ...settings };
  }

  setSettings(settings: Partial<SoundSettings>) {
    this.settings = { ...this.settings, ...settings };
  }

  private context(): AudioContext | null {
    if (typeof window === "undefined" || !("AudioContext" in window)) return null;
    this.ctx ??= new AudioContext();
    if (this.ctx.state === "suspended") void this.ctx.resume();
    return this.ctx;
  }

  private volume(): number {
    return Math.max(0, Math.min(1, this.settings.fxVolume / 100));
  }

  /** 单音:频率/时长/峰值增益/波形,指数衰减包络 */
  private tone(
    freq: number,
    durMs: number,
    peak: number,
    type: OscillatorType = "sine",
    startDelayMs = 0,
    cutPrevious = false,
  ) {
    const ctx = this.context();
    if (!ctx || peak <= 0) return;
    if (cutPrevious && this.active.fx) {
      this.active.fx.gain.cancelScheduledValues(ctx.currentTime);
      this.active.fx.gain.setValueAtTime(0, ctx.currentTime);
    }
    const t0 = ctx.currentTime + startDelayMs / 1000;
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = type;
    osc.frequency.value = freq;
    gain.gain.setValueAtTime(0, t0);
    gain.gain.linearRampToValueAtTime(peak, t0 + 0.004);
    gain.gain.exponentialRampToValueAtTime(0.0001, t0 + durMs / 1000);
    osc.connect(gain).connect(ctx.destination);
    osc.start(t0);
    osc.stop(t0 + durMs / 1000 + 0.02);
    if (cutPrevious) this.active.fx = gain;
  }

  /** 按键音:随机 ±3% 音高防机械重复感;双振荡叠加增加厚度 */
  key() {
    const kind = this.settings.keySound;
    if (kind === "off") return;
    const pitch = 1 + (Math.random() * 0.06 - 0.03);
    if (kind === "soft") {
      this.tone(1500 * pitch, 34, KEY_GAIN * this.volume(), "sine");
      this.tone(750 * pitch, 40, KEY_GAIN * 0.5 * this.volume(), "triangle");
    } else {
      // 机械:亮起振 + 短促底噪感(方波泛音自带"咔")
      this.tone(2300 * pitch, 26, KEY_GAIN * 0.9 * this.volume(), "square");
      this.tone(900 * pitch, 36, KEY_GAIN * 0.7 * this.volume(), "triangle");
    }
  }

  /** 错误低音:比正确音刻意更轻(§6.5 相对关系) */
  error() {
    this.tone(220, 110, ERROR_GAIN * this.volume(), "sine", 0, true);
  }

  /** 正确"叮":880Hz · 90ms(§6.2) */
  correct() {
    this.tone(880, 90, DING_GAIN * this.volume(), "sine", 0, true);
  }

  /**
   * 签名时刻:C6–E6–G6 上行分解和弦,每音 90ms 间隔 80ms。
   * 只有第一音 cutPrevious —— 否则后两音会把自己前一音掐掉(§6.5 不叠播
   * 针对的是"同类音效重触发",一串音内部不算)。
   */
  celebrate() {
    const v = CELEBRATE_GAIN * this.volume();
    this.tone(1046, 90, v, "sine", 0, true);
    this.tone(1318, 90, v, "sine", 80);
    this.tone(1568, 170, v * 1.05, "sine", 160);
  }

  /** 掌握上行双音(§6.5) */
  master() {
    this.tone(660, 80, MASTER_GAIN * this.volume(), "sine", 0, true);
    this.tone(880, 110, MASTER_GAIN * this.volume(), "sine", 90);
  }
}
