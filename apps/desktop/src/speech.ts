/**
 * 桌面端语音:piper 音频包在场时走 tts_speak(离线 wav),
 * 否则回退 WebView 的 speechSynthesis(§7.1)。
 */

import { convertFileSrc } from "@tauri-apps/api/core";
import type { SpeakOptions, SpeechService } from "@sentenceflow/ui";
import { ipc } from "./ipc";

let audio: HTMLAudioElement | null = null;
let piperMissing = false;

/**
 * 每次 speak / stop 自增。`onEnd` 只在自己那一代仍然当值时才回调 ——
 * 否则连点两句时,第一句的 ended 会晚一步把第二句的"正在朗读"抹掉。
 * piper 走的是 IPC + 文件播放,这个时间差是常态而不是极端情况。
 */
let generation = 0;

function webFallback(text: string, options: SpeakOptions | undefined, done: () => void) {
  if (!("speechSynthesis" in window)) {
    done();
    return;
  }
  window.speechSynthesis.cancel();
  const u = new SpeechSynthesisUtterance(text);
  u.lang = options?.voice === "us" ? "en-US" : "en-GB";
  u.rate = Math.min(1.4, Math.max(0.6, options?.rate ?? 1.0));
  u.onend = done;
  // 出错也要收尾,不然界面上的"正在朗读"没人熄灯
  u.onerror = done;
  window.speechSynthesis.speak(u);
}

export const desktopSpeech: SpeechService = {
  speak(text: string, options?: SpeakOptions) {
    const rate = options?.rate ?? 1.0;
    const us = options?.voice === "us";
    const mine = ++generation;
    // 同一次 speak 最多回调一次,且被后来的 speak/stop 取代后就不再回调
    let settled = false;
    const done = () => {
      if (settled || mine !== generation) return;
      settled = true;
      options?.onEnd?.();
    };

    if (piperMissing) {
      webFallback(text, options, done);
      return;
    }
    void ipc
      .ttsSpeak(text, us, rate)
      .then((path) => {
        if (mine !== generation) return; // 这次已经被取代,别再抢音频通道
        if (path === null) {
          piperMissing = true;
          webFallback(text, options, done);
          return;
        }
        audio?.pause();
        audio = new Audio(convertFileSrc(path));
        audio.onended = done;
        audio.onerror = done;
        void audio.play().catch(done);
      })
      .catch(() => {
        webFallback(text, options, done);
      });
  },
  stop() {
    generation += 1; // 作废在途的 onEnd
    audio?.pause();
    audio = null;
    if ("speechSynthesis" in window) window.speechSynthesis.cancel();
  },
};
