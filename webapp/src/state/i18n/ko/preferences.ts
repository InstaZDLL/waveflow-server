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
    "태그에 기록된 음량 측정값을 바탕으로 곡마다 음량을 조절해 음반 간 음량 차이를 줄여요. 앨범 게인은 앨범 안의 음량 차이를 유지해요. 태그가 없는 곡은 그대로 재생돼요. 재생이 Web Audio를 거치므로 일부 휴대폰에서는 화면이 꺼지면 재생이 멈출 수 있어요.",
  "settings.detail":
    "화면 표시, 언어, 내 보관함을 읽을 수 있는 클라이언트를 설정해요.",
  "settings.appearance": "화면 표시",
  "settings.playback": "재생",
  "settings.sections": "설정 항목",
  "settings.section.account": "계정",
  "settings.playbackDetail":
    "곡 재생 방식을 설정해요. 음량 평준화 설정은 이 브라우저에, 트랜스코딩 설정은 계정에 저장돼요.",
  "settings.levelling": "음량 평준화",
  "transcoding.title": "앱을 위한 트랜스코딩",
  "transcoding.detail":
    "계정에 저장돼요. 형식이나 비트레이트를 직접 요청하지 않는 앱에 적용돼요. 요청하는 앱은 항상 요청한 그대로 받아요. 이미 이 비트레이트 이하인 곡은 그대로 재생돼요. 이 웹 플레이어는 계속 원본을 재생해요.",
  "transcoding.seeking":
    "곡을 변환하는 동안에는 파일 위치로 탐색하는 앱에서 탐색이 안 될 수 있어요. 사용하는 앱에서 그렇다면 아래에서 원본으로 추가하세요.",
  "transcoding.allApps": "모든 앱",
  "transcoding.format": "형식",
  "transcoding.original": "원본",
  "transcoding.bitrate": "최대 비트레이트 (kbit/s)",
  "transcoding.bitrateDefault": "형식의 기본값",
  "transcoding.save": "저장",
  "transcoding.saved": "저장했어요.",
  "transcoding.oneApp": "특정 앱",
  "transcoding.oneAppDetail":
    "이 앱에만 위 설정보다 우선 적용돼요. 앱이 서버에 알리는 이름을 입력하세요. 형식이나 비트레이트를 요청하지 않고 재생한 앱이 후보로 표시돼요.",
  "transcoding.client": "앱 이름",
  "transcoding.add": "추가",
  "transcoding.remove": "제거",
  "transcoding.none": "개별 설정이 있는 앱이 없어요.",
  "transcoding.rate": "{format} · {bitrate} kbit/s",
  "transcoding.bitrateRange": "비트레이트는 32에서 512 kbit/s 사이여야 해요.",
  "transcoding.error": "저장하지 못해서 아무것도 바뀌지 않았어요.",
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
