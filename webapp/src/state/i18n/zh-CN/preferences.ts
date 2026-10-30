// Preferences, ReplayGain, the settings page and themes.
import type { TranslationValue } from "..";
import type { preferences as en } from "../en/preferences";

export const preferences: Record<keyof typeof en, TranslationValue> = {
  "preferences.theme": "主题",
  "preferences.language": "语言",
  "preferences.themeLight": "浅色",
  "preferences.themeDark": "深色",
  "preferences.followSystem": "跟随系统",
  "preferences.followSystemDetail":
    "设备切换浅色或深色模式时，主题也随之切换到对应的版本。",
  "preferences.followingLight":
    "你的设备处于浅色模式，因此显示浅色版本。选择一个主题会同时选中它的两个版本。",
  "preferences.followingDark":
    "你的设备处于深色模式，因此显示深色版本。选择一个主题会同时选中它的两个版本。",
  "preferences.standInLight":
    "{theme} 没有浅色版本，设备处于浅色模式时将以 {fallback} 代替。",
  "preferences.standInDark":
    "{theme} 没有深色版本，设备处于深色模式时将以 {fallback} 代替。",
  "replayGain.label": "回放增益 (ReplayGain)",
  "replayGain.off": "关闭",
  "replayGain.track": "曲目增益",
  "replayGain.album": "专辑增益",
  "replayGain.preamp": "前置放大",
  "replayGain.detail":
    "按照标签中测得的响度播放每首曲目，让不同唱片的音量保持一致。专辑增益会保留同一张专辑内曲目之间的响度差异。没有增益标签的曲目按原样播放。开启后播放会经过 Web Audio，部分手机在锁屏时会因此暂停播放。",
  "settings.detail": "外观、语言，以及允许读取你音乐库的客户端。",
  "settings.appearance": "外观",
  "settings.playback": "播放",
  "settings.sections": "设置分区",
  "settings.section.account": "账户",
  "settings.playbackDetail":
    "此浏览器播放曲目的方式。仅保存在此浏览器中，不会同步到你的账户。",
  "settings.levelling": "音量均衡",
  "appearance.themes.default": "翡翠浅色",
  "appearance.themes.midnightLight": "午夜浅色",
  "appearance.themes.sunsetLight": "日落浅色",
  "appearance.themes.lavenderLight": "薰衣草浅色",
  "appearance.themes.crimsonLight": "绯红浅色",
  "appearance.themes.oceanLight": "海洋浅色",
  "appearance.themes.defaultDark": "翡翠深色",
  "appearance.themes.oled": "OLED 纯黑",
  "appearance.themes.midnight": "午夜",
  "appearance.themes.sunset": "日落",
  "appearance.themes.lavender": "薰衣草",
  "appearance.themes.crimson": "绯红",
  "appearance.themes.ocean": "海洋",
  "appearance.themes.neon": "霓虹",
};
