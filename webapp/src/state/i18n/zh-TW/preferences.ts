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
    "曲目的播放方式。音量平衡儲存在這個瀏覽器中，轉碼設定儲存在你的帳號中。",
  "settings.levelling": "音量平衡",
  "transcoding.title": "為你的應用程式轉碼",
  "transcoding.detail":
    "儲存在你的帳號中。適用於不自行指定格式或位元率的應用程式：有指定的應用程式一律得到它所指定的內容。位元率已不高於此值的曲目會按原樣播放。這個網頁播放器會繼續播放原始檔案。",
  "transcoding.seeking":
    "曲目還在轉換時，依檔案位置跳轉的應用程式可能無法在其中跳轉。如果你的某個應用程式發生這種情況，請在下方以「原始」新增它。",
  "transcoding.allApps": "所有應用程式",
  "transcoding.format": "格式",
  "transcoding.original": "原始",
  "transcoding.bitrate": "最高位元率 (kbit/s)",
  "transcoding.bitrateDefault": "格式的預設值",
  "transcoding.save": "儲存",
  "transcoding.saved": "已儲存。",
  "transcoding.oneApp": "針對單一應用程式",
  "transcoding.oneAppDetail":
    "依該應用程式告知伺服器的名稱，取代上面的設定。曾在此播放且未指定任何內容的應用程式會列為建議。",
  "transcoding.client": "應用程式名稱",
  "transcoding.add": "新增",
  "transcoding.remove": "移除",
  "transcoding.none": "沒有應用程式擁有自己的設定。",
  "transcoding.rate": "{format} · {bitrate} kbit/s",
  "transcoding.bitrateRange": "位元率必須介於 32 到 512 kbit/s 之間。",
  "transcoding.error": "無法儲存，因此沒有任何變更。",
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
