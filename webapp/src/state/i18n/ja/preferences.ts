// Preferences, ReplayGain, the settings page and themes.
import type { TranslationValue } from "..";
import type { preferences as en } from "../en/preferences";

export const preferences: Record<keyof typeof en, TranslationValue> = {
  "preferences.theme": "テーマ",
  "preferences.language": "言語",
  "preferences.themeLight": "ライト",
  "preferences.themeDark": "ダーク",
  "preferences.followSystem": "システムに合わせる",
  "preferences.followSystemDetail":
    "デバイスの切り替えに合わせて、テーマのライト版とダーク版を切り替えます。",
  "preferences.followingLight":
    "デバイスがライトモードなので、ライト版を表示しています。テーマを選ぶと、そのライト版とダーク版の両方が選ばれます。",
  "preferences.followingDark":
    "デバイスがダークモードなので、ダーク版を表示しています。テーマを選ぶと、そのライト版とダーク版の両方が選ばれます。",
  "preferences.standInLight":
    "{theme}にはライト版がないため、デバイスがライトモードの間は{fallback}を代わりに使います。",
  "preferences.standInDark":
    "{theme}にはダーク版がないため、デバイスがダークモードの間は{fallback}を代わりに使います。",
  "replayGain.label": "ReplayGain",
  "replayGain.off": "オフ",
  "replayGain.track": "トラックゲイン",
  "replayGain.album": "アルバムゲイン",
  "replayGain.preamp": "プリアンプ",
  "replayGain.detail":
    "タグに記録された測定値に合わせて各曲の音量をそろえるため、アルバムごとに音量が変わることはありません。アルバムゲインでは、アルバム内の音量差はそのまま保たれます。タグのない曲はそのまま再生されます。再生は Web Audio を経由するため、一部のスマートフォンでは画面が消えると一時停止します。",
  "settings.detail": "外観、言語、ライブラリの読み込みを許可したクライアント。",
  "settings.appearance": "外観",
  "settings.playback": "再生",
  "settings.sections": "設定の項目",
  "settings.section.account": "アカウント",
  "settings.playbackDetail":
    "このブラウザでの曲の再生方法です。アカウントではなく、このブラウザに保存されます。",
  "settings.levelling": "音量の均一化",
  "appearance.themes.default": "エメラルド（ライト）",
  "appearance.themes.midnightLight": "ミッドナイト（ライト）",
  "appearance.themes.sunsetLight": "サンセット（ライト）",
  "appearance.themes.lavenderLight": "ラベンダー（ライト）",
  "appearance.themes.crimsonLight": "クリムゾン（ライト）",
  "appearance.themes.oceanLight": "オーシャン（ライト）",
  "appearance.themes.defaultDark": "エメラルド（ダーク）",
  "appearance.themes.oled": "OLED ブラック",
  "appearance.themes.midnight": "ミッドナイト",
  "appearance.themes.sunset": "サンセット",
  "appearance.themes.lavender": "ラベンダー",
  "appearance.themes.crimson": "クリムゾン",
  "appearance.themes.ocean": "オーシャン",
  "appearance.themes.neon": "ネオン",
};
