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

export const webSpeech: SpeechService = {
  speak(text: string, options?: SpeakOptions) {
    if (!("speechSynthesis" in window)) return;
    window.speechSynthesis.cancel();
    const u = new SpeechSynthesisUtterance(text);
    const voice = pickVoice(options?.voice ?? "gb");
    if (voice) u.voice = voice;
    u.lang = voice?.lang ?? "en-GB";
    u.rate = Math.min(1.4, Math.max(0.6, options?.rate ?? 1.0));
    window.speechSynthesis.speak(u);
  },
  stop() {
    if ("speechSynthesis" in window) window.speechSynthesis.cancel();
  },
};

// 部分浏览器 voices 异步加载;预热一次。
if (typeof window !== "undefined" && "speechSynthesis" in window) {
  window.speechSynthesis.getVoices();
  window.speechSynthesis.onvoiceschanged = () => window.speechSynthesis.getVoices();
}
