// Preferences, ReplayGain, the settings page and themes.
import type { TranslationValue } from "..";
import type { preferences as en } from "../en/preferences";

export const preferences: Record<keyof typeof en, TranslationValue> = {
  "preferences.theme": "主題",
  "preferences.language": "語言",
  "preferences.themeLight": "淺色",
  "preferences.themeDark": "深色",
  "preferences.followSystem": "跟隨系統",
  "preferences.followSystemDetail":
    "裝置切換淺色或深色模式時，主題也會跟著切換到對應的版本。",
  "preferences.followingLight":
    "你的裝置目前是淺色模式，所以顯示淺色版本。選擇一個主題會同時選取它的兩個版本。",
  "preferences.followingDark":
    "你的裝置目前是深色模式，所以顯示深色版本。選擇一個主題會同時選取它的兩個版本。",
  "preferences.standInLight":
    "{theme} 沒有淺色版本，裝置處於淺色模式時會以 {fallback} 代替。",
  "preferences.standInDark":
    "{theme} 沒有深色版本，裝置處於深色模式時會以 {fallback} 代替。",
  "replayGain.label": "ReplayGain",
  "replayGain.off": "關閉",
  "replayGain.track": "曲目增益",
  "replayGain.album": "專輯增益",
  "replayGain.preamp": "前級增益",
  "replayGain.detail":
    "依照標籤中測得的響度播放每首曲目，讓不同唱片的音量保持一致。專輯增益會保留同一張專輯內曲目之間的響度差異。沒有增益標籤的曲目會照原樣播放。開啟後播放會經過 Web Audio，部分手機在螢幕關閉時會因此暫停播放。",
  "settings.detail": "外觀、語言，以及可以讀取你音樂庫的用戶端。",
  "settings.appearance": "外觀",
  "settings.playback": "播放",
  "settings.sections": "設定項目",
  "settings.section.account": "帳號",
  "settings.playbackDetail":
    "這個瀏覽器播放曲目的方式。只會儲存在這個瀏覽器中，不會同步到你的帳號。",
  "settings.levelling": "音量平衡",
  "appearance.themes.default": "翡翠淺色",
  "appearance.themes.midnightLight": "午夜淺色",
  "appearance.themes.sunsetLight": "日落淺色",
  "appearance.themes.lavenderLight": "薰衣草淺色",
  "appearance.themes.crimsonLight": "緋紅淺色",
  "appearance.themes.oceanLight": "海洋淺色",
  "appearance.themes.defaultDark": "翡翠深色",
  "appearance.themes.oled": "OLED 純黑",
  "appearance.themes.midnight": "午夜",
  "appearance.themes.sunset": "日落",
  "appearance.themes.lavender": "薰衣草",
  "appearance.themes.crimson": "緋紅",
  "appearance.themes.ocean": "海洋",
  "appearance.themes.neon": "霓虹",
};
