/**
 * 桌面端语音:piper 音频包在场时走 tts_speak(离线 wav),
 * 否则回退 WebView 的 speechSynthesis(§7.1)。
 */

import { convertFileSrc } from "@tauri-apps/api/core";
import type { SpeakOptions, SpeechService } from "@sentenceflow/ui";
import { ipc } from "./ipc";

let audio: HTMLAudioElement | null = null;
let piperMissing = false;

function webFallback(text: string, options?: SpeakOptions) {
  if (!("speechSynthesis" in window)) return;
  window.speechSynthesis.cancel();
  const u = new SpeechSynthesisUtterance(text);
  u.lang = options?.voice === "us" ? "en-US" : "en-GB";
  u.rate = Math.min(1.4, Math.max(0.6, options?.rate ?? 1.0));
  window.speechSynthesis.speak(u);
}

export const desktopSpeech: SpeechService = {
  speak(text: string, options?: SpeakOptions) {
    const rate = options?.rate ?? 1.0;
    const us = options?.voice === "us";
    if (piperMissing) {
      webFallback(text, options);
      return;
    }
    void ipc
      .ttsSpeak(text, us, rate)
      .then((path) => {
        if (path === null) {
          piperMissing = true;
          webFallback(text, options);
          return;
        }
        audio?.pause();
        audio = new Audio(convertFileSrc(path));
        void audio.play();
      })
      .catch(() => {
        webFallback(text, options);
      });
  },
  stop() {
    audio?.pause();
    audio = null;
    if ("speechSynthesis" in window) window.speechSynthesis.cancel();
  },
};
