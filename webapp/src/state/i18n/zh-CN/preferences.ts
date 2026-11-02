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
    "曲目的播放方式。音量均衡保存在此浏览器中，转码设置保存在你的账户中。",
  "settings.levelling": "音量均衡",
  "transcoding.title": "为你的应用转码",
  "transcoding.detail":
    "保存在你的账户中。适用于不自行指定格式或比特率的应用：指定了的应用始终得到它所指定的内容。比特率已不高于此值的曲目按原样播放。此网页播放器继续播放原始文件。",
  "transcoding.seeking":
    "曲目仍在转换时，按文件位置跳转的应用可能无法在其中跳转。如果你的某个应用出现这种情况，请在下方以“原始”添加它。",
  "transcoding.allApps": "所有应用",
  "transcoding.format": "格式",
  "transcoding.original": "原始",
  "transcoding.bitrate": "最高比特率 (kbit/s)",
  "transcoding.bitrateDefault": "格式的默认值",
  "transcoding.save": "保存",
  "transcoding.saved": "已保存。",
  "transcoding.oneApp": "针对单个应用",
  "transcoding.oneAppDetail":
    "按该应用告知服务器的名称，替换上面的设置。曾在此播放且未指定任何内容的应用会列为建议。",
  "transcoding.client": "应用名称",
  "transcoding.add": "添加",
  "transcoding.remove": "移除",
  "transcoding.none": "没有应用拥有自己的设置。",
  "transcoding.rate": "{format} · {bitrate} kbit/s",
  "transcoding.bitrateRange": "比特率必须在 32 到 512 kbit/s 之间。",
  "transcoding.error": "无法保存，因此没有任何更改。",
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
