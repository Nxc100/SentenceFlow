/** Web Speech API 实现 SpeechService(en-GB 优先,§7.9 试用版语音)。 */

import type { SpeechService, SpeakOptions } from "@sentenceflow/ui";

function pickVoice(prefer: "gb" | "us"): SpeechSynthesisVoice | null {
  const voices = window.speechSynthesis?.getVoices() ?? [];
  const lang = prefer === "us" ? "en-US" : "en-GB";
  return (
    voices.find((v) => v.lang === lang) ??
    voices.find((v) => v.lang.startsWith("en")) ??
    null
  );
}

/** 每次 speak / stop 自增:作废在途的 onEnd,免得上一句的收尾把下一句的
 *  "正在朗读"提前熄灭(见 SpeakOptions.onEnd 的约定)。 */
let generation = 0;

export const webSpeech: SpeechService = {
  speak(text: string, options?: SpeakOptions) {
    const mine = ++generation;
    let settled = false;
    const done = () => {
      if (settled || mine !== generation) return;
      settled = true;
      options?.onEnd?.();
    };
    if (!("speechSynthesis" in window)) {
      done(); // 没有语音能力也要收尾,否则调用方的朗读指示一直亮着
      return;
    }
    window.speechSynthesis.cancel();
    const u = new SpeechSynthesisUtterance(text);
    const voice = pickVoice(options?.voice ?? "gb");
    if (voice) u.voice = voice;
    u.lang = voice?.lang ?? "en-GB";
    u.rate = Math.min(1.4, Math.max(0.6, options?.rate ?? 1.0));
    u.onend = done;
    u.onerror = done;
    window.speechSynthesis.speak(u);
  },
  stop() {
    generation += 1;
    if ("speechSynthesis" in window) window.speechSynthesis.cancel();
  },
};

// 部分浏览器 voices 异步加载;预热一次。
if (typeof window !== "undefined" && "speechSynthesis" in window) {
  window.speechSynthesis.getVoices();
  window.speechSynthesis.onvoiceschanged = () => window.speechSynthesis.getVoices();
}
