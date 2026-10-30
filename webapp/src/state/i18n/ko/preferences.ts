// Preferences, ReplayGain, the settings page and themes.
import type { TranslationValue } from "..";
import type { preferences as en } from "../en/preferences";

export const preferences: Record<keyof typeof en, TranslationValue> = {
  "preferences.theme": "테마",
  "preferences.language": "언어",
  "preferences.themeLight": "라이트",
  "preferences.themeDark": "다크",
  "preferences.followSystem": "시스템 설정 따르기",
  "preferences.followSystemDetail":
    "기기 설정이 바뀌면 테마의 라이트 버전과 다크 버전을 전환해요.",
  "preferences.followingLight":
    "기기가 라이트 모드라서 라이트 버전이 표시돼요. 테마를 고르면 라이트와 다크 버전이 함께 선택돼요.",
  "preferences.followingDark":
    "기기가 다크 모드라서 다크 버전이 표시돼요. 테마를 고르면 라이트와 다크 버전이 함께 선택돼요.",
  "preferences.standInLight":
    "{theme} 테마에는 라이트 버전이 없어서 기기가 라이트 모드일 때는 {fallback} 테마로 대신 표시돼요.",
  "preferences.standInDark":
    "{theme} 테마에는 다크 버전이 없어서 기기가 다크 모드일 때는 {fallback} 테마로 대신 표시돼요.",
  "replayGain.label": "ReplayGain",
  "replayGain.off": "끄기",
  "replayGain.track": "트랙 게인",
  "replayGain.album": "앨범 게인",
  "replayGain.preamp": "프리앰프",
  "replayGain.detail":
    "태그에 측정된 음량으로 모든 곡을 재생해서 음반마다 소리 크기가 달라지지 않게 해요. 앨범 게인은 앨범 안의 음량 차이를 유지해요. 태그가 없는 곡은 그대로 재생돼요. 재생이 Web Audio를 거치게 되는데, 일부 휴대폰에서는 화면이 꺼지면 재생이 멈출 수 있어요.",
  "settings.detail":
    "화면 표시, 언어, 내 보관함을 읽을 수 있는 클라이언트를 설정해요.",
  "settings.appearance": "화면 표시",
  "settings.playback": "재생",
  "settings.sections": "설정 항목",
  "settings.section.account": "계정",
  "settings.playbackDetail":
    "이 브라우저에서 곡을 재생하는 방식이에요. 계정이 아닌 이 브라우저에 저장돼요.",
  "settings.levelling": "음량 평준화",
  "appearance.themes.default": "에메랄드 라이트",
  "appearance.themes.midnightLight": "미드나이트 라이트",
  "appearance.themes.sunsetLight": "선셋 라이트",
  "appearance.themes.lavenderLight": "라벤더 라이트",
  "appearance.themes.crimsonLight": "크림슨 라이트",
  "appearance.themes.oceanLight": "오션 라이트",
  "appearance.themes.defaultDark": "에메랄드 다크",
  "appearance.themes.oled": "OLED 블랙",
  "appearance.themes.midnight": "미드나이트",
  "appearance.themes.sunset": "선셋",
  "appearance.themes.lavender": "라벤더",
  "appearance.themes.crimson": "크림슨",
  "appearance.themes.ocean": "오션",
  "appearance.themes.neon": "네온",
};
