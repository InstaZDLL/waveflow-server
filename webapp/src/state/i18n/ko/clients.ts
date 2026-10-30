// Registered clients, the Subsonic password and OAuth consent.
import type { TranslationValue } from "..";
import type { clients as en } from "../en/clients";

export const clients: Record<keyof typeof en, TranslationValue> = {
  "clients.title": "등록된 클라이언트",
  "clients.detail":
    "내 보관함에 접근할 자격 증명을 가진 기기와 스크립트예요. 폐기하면 즉시 접근이 끊겨요.",
  "clients.name": "용도",
  "clients.register": "등록",
  "clients.once":
    "지금 복사하세요. 해시만 저장되기 때문에 이번 한 번만 표시되고 다시는 볼 수 없어요:",
  "clients.revoke": "폐기",
  "clients.none": "내 보관함에 접근할 자격 증명을 가진 클라이언트가 없어요.",
  "clients.error": "실패했어요. 아무것도 바뀌지 않았어요.",
  "clients.registered": "{when} 등록",
  "clients.used": "마지막 사용 {when}",
  "clients.unused": "사용한 적 없음",
  "subsonic.title": "Subsonic 비밀번호",
  "subsonic.detail":
    "Subsonic 클라이언트가 로그인할 때 쓰는 별도의 비밀번호예요. 비밀번호를 바꾸면 API 키도 함께 바뀌어서 유출된 키는 더 이상 작동하지 않아요.",
  "subsonic.set": "{when} 설정",
  "subsonic.changed": "마지막 변경 {when}",
  "subsonic.password": "새 비밀번호",
  "subsonic.rotate": "변경",
  "subsonic.revoke": "끄기",
  "subsonic.confirm": "네, 끌게요",
  "subsonic.once":
    "지금 이 API 키를 복사하세요. 해시만 저장되기 때문에 이번 한 번만 표시되고 다시는 볼 수 없어요:",
  "subsonic.gone":
    "각 Subsonic 클라이언트에 비밀번호를 다시 설정하기 전까지는 클라이언트가 작동하지 않아요.",
  "subsonic.none":
    "Subsonic 비밀번호가 없어서 어떤 Subsonic 클라이언트도 내 보관함을 읽을 수 없어요. 관리자에게 요청하세요.",
  "subsonic.short": "12자 이상 입력하세요.",
  "subsonic.error": "실패했어요. 아무것도 바뀌지 않았어요.",
  "credential.password": "Subsonic 전용 비밀번호",
  "credential.rotate": "자격 증명 교체",
  "credential.copy": "지금 이 API 키를 복사하세요:",
  "credential.error": "자격 증명을 교체하지 못했어요.",
  "authorize.title": "권한 부여",
  "authorize.invalid":
    "이 권한 부여 링크가 불완전하거나, 신뢰할 수 없는 곳으로 보내려고 해요.",
  "authorize.client": "{client}에 권한 부여",
  "authorize.detail":
    "보관함 탐색, 음악 재생, 플레이리스트·즐겨찾기·별점 관리 등 이 계정으로 할 수 있는 모든 작업이 가능해져요.",
  "authorize.redirect": "{url}(으)로 돌아가는 중",
  "authorize.error": "앱이 처리할 수 없는 권한 요청을 보냈어요.",
  "authorize.busy": "권한 부여 중…",
  "authorize.approve": "권한 부여",
};
