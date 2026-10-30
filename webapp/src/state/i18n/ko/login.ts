// Sign-in and first-run setup.
import type { TranslationValue } from "..";
import type { login as en } from "../en/login";

export const login: Record<keyof typeof en, TranslationValue> = {
  "login.heroEyebrow": "내 음악을 내 방식대로",
  "login.heroTitle": "어둠이 내리면 보관함이 깨어나요.",
  "login.heroDetail": "프라이빗 스트리밍, 하나의 카탈로그, 모든 화면에서.",
  "login.server": "WaveFlow 서버",
  "login.createAdmin": "관리자 만들기",
  "login.welcome": "다시 오신 것을 환영해요",
  "login.setupDetail": "새 서버예요. 첫 번째 관리자 계정을 만드세요.",
  "login.setupToken": "설정 토큰",
  "login.setupTokenHint":
    "서버가 데이터 디렉터리의 setup-token 파일에 토큰을 기록했고, 서버 로그에 그 경로가 나와 있어요. WAVEFLOW_SETUP_TOKEN을 설정했다면 그 값을 입력하세요.",
  "login.setupTokenError":
    "설정 토큰이 올바르지 않아요. setup-token 파일에서 다시 복사하세요.",
  "login.username": "사용자 이름",
  "login.password": "비밀번호",
  "login.setupError":
    "설정하지 못했어요. 올바른 사용자 이름과 12자 이상의 비밀번호를 사용하세요.",
  "login.error": "사용자 이름 또는 비밀번호가 올바르지 않아요.",
  "login.wait": "잠시만 기다려 주세요…",
  "login.create": "만들고 로그인",
  "login.signIn": "로그인",
  "login.offline": {
    one: "이 브라우저에 저장된 {count}곡 듣기",
    other: "이 브라우저에 저장된 {count}곡 듣기",
  },
};
