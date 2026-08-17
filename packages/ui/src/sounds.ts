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
  master(): void;
}

export const silentSounds: SoundPlayer = {
  key() {},
  error() {},
  correct() {},
  master() {},
};

/** −14dB ≈ 0.2 线性增益(§6.2 正确音基准) */
const DING_GAIN = 0.2;

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

  /** 按键音:随机 ±3% 音高防机械重复感 */
  key() {
    const kind = this.settings.keySound;
    if (kind === "off") return;
    const pitch = 1 + (Math.random() * 0.06 - 0.03);
    if (kind === "soft") {
      this.tone(1750 * pitch, 28, 0.05 * this.volume(), "sine");
    } else {
      // 机械:亮起振 + 短促底噪感(方波泛音自带"咔")
      this.tone(2300 * pitch, 22, 0.045 * this.volume(), "square");
      this.tone(920 * pitch, 30, 0.03 * this.volume(), "triangle");
    }
  }

  /** 错误低音:比正确音刻意更轻(§6.5) */
  error() {
    this.tone(220, 90, 0.09 * this.volume(), "sine", 0, true);
  }

  /** 正确"叮":880Hz · 90ms · −14dB 基准(§6.2) */
  correct() {
    this.tone(880, 90, DING_GAIN * this.volume(), "sine", 0, true);
  }

  /** 掌握上行双音(§6.5) */
  master() {
    this.tone(660, 80, 0.15 * this.volume(), "sine", 0, true);
    this.tone(880, 110, 0.15 * this.volume(), "sine", 90);
  }
}
