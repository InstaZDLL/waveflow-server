// Scrobbling.
import type { TranslationValue } from "..";
import type { scrobbling as en } from "../en/scrobbling";

export const scrobbling: Record<keyof typeof en, TranslationValue> = {
  "scrobbling.title": "외부 스크로블",
  "scrobbling.detail":
    "여기서 들은 음악을 ListenBrainz, Maloja, Last.fm으로 전송해요.",
  "scrobbling.none":
    "이 서버에는 연결할 수 있는 서비스가 없어요. 서버 운영자가 정하는 설정이에요.",
  "scrobbling.provider.listenbrainz": "ListenBrainz",
  "scrobbling.provider.maloja": "Maloja",
  "scrobbling.provider.lastfm": "Last.fm",
  "scrobbling.instance": "인스턴스 ‘{name}’",
  "scrobbling.unavailable": "여기서는 사용할 수 없음: {reason}",
  "scrobbling.unavailable.no_application_configured":
    "이 서버에 등록된 Last.fm 애플리케이션이 없어요",
  "scrobbling.unavailable.browser_journey_needs_https":
    "브라우저로 연결하려면 이 서버의 공개 주소가 https여야 해요. 운영자는 명령줄에서 계정을 연결할 수 있어요",
  "scrobbling.unavailable.unknown": "서버가 이유를 알려 주지 않았어요",
  "scrobbling.retired":
    "이 서버는 더 이상 이 인스턴스를 지원하지 않아요. 앞으로는 아무것도 전송되지 않아요. 권한은 여전히 철회할 수 있어요.",
  "scrobbling.notLinked": "연결되지 않음.",
  "scrobbling.secret": "API 키",
  "scrobbling.secretHint": "해당 서비스의 내 계정에서 받은 키예요.",
  "scrobbling.link": "연결",
  "scrobbling.working": "처리 중…",
  "scrobbling.linkError":
    "권한 부여가 거부됐어요. 키를 확인하고 다시 시도하세요.",
  "scrobbling.unlink": "연결 해제",
  "scrobbling.unlinkError": "연결을 해제할 수 없어요.",
  "scrobbling.connect": "Last.fm에 연결",
  "scrobbling.connectDetail":
    "Last.fm은 붙여 넣을 키를 주지 않아요. 그래서 Last.fm에서 직접 승인한 뒤 이곳으로 돌아와요.",
  "scrobbling.connectError": "Last.fm 연결 과정을 열 수 없어요.",
  "scrobbling.returned": "Last.fm에 연결됐어요: {name}.",
  "scrobbling.health.healthy": "재생 기록 전송 중",
  "scrobbling.health.degraded": "대기열이 멈춰 있음",
  "scrobbling.health.broken": "작동 안 함",
  "scrobbling.healthDetail.healthy": "막혀 있는 항목이 없어요.",
  "scrobbling.healthDetail.degraded":
    "연결은 응답하지만 재생 기록이 전달되지 않고 있어요. 내 결정을 기다리는 기록이 있거나, 가장 오래된 기록이 너무 오래 기다렸어요.",
  "scrobbling.healthDetail.broken":
    "자격 증명이 거부됐거나, 연결할 때 사용한 이름으로는 이 서버가 더 이상 그 인스턴스에 접근할 수 없어요.",
  "scrobbling.pending": {
    one: "대기 중 {count}개",
    other: "대기 중 {count}개",
  },
  "scrobbling.retrying": {
    one: "재시도 중 {count}개",
    other: "재시도 중 {count}개",
  },
  "scrobbling.uncertainCount": {
    one: "내 결정을 기다리는 기록 {count}개",
    other: "내 결정을 기다리는 기록 {count}개",
  },
  "scrobbling.oldest": "가장 오래된 기록은 {when}부터 기다리고 있어요.",
  "scrobbling.lastSuccess": "마지막 전송 성공: {when}.",
  "scrobbling.lastFailure": "마지막 거부 사유: {reason}.",
  "scrobbling.uncertain": "내가 결정해야 하는 재생 기록",
  "scrobbling.uncertainDetail":
    "이 기록들을 보낸 뒤 연결이 끊겨서, 상대 서비스가 기록했는지 아무도 알 수 없어요. 다시 보내면 다른 사람들이 보는 재생 기록에 두 번 남을 수 있고, 버리면 기록에 빈칸이 생겨요. 이 서버는 대신 결정하지 않으며, 한 번만 물어봐요.",
  "scrobbling.uncertainNone": "내 결정을 기다리는 기록이 없어요.",
  "scrobbling.playedAt": "{when} 재생",
  "scrobbling.attempts": { one: "{count}회 시도", other: "{count}회 시도" },
  "scrobbling.retry": "다시 보내기",
  "scrobbling.discard": "버리기",
  "scrobbling.answered": "처리했어요.",
  "scrobbling.answerError":
    "이 재생 기록을 처리할 수 없어요. 페이지를 새로고침한 뒤 다시 시도하세요.",
  "scrobbling.unnamedTrack": "이 계정에 남아 있는 재생 기록에 없는 곡",
  "scrobbling.reason.rate_limited":
    "상대 서비스가 잠시 기다려 달라고 요청했어요",
  "scrobbling.reason.retryable": "상대 서비스가 받지 않았어요",
  "scrobbling.reason.auth_broken": "권한 부여가 거부됐어요",
  "scrobbling.reason.rejected": "상대 서비스가 확인한 뒤 거부했어요",
  "scrobbling.reason.destination_gone":
    "이 서버가 더 이상 그 이름으로 해당 인스턴스에 접근할 수 없어요",
  "scrobbling.reason.credential_unreadable":
    "이 서버가 저장된 자격 증명을 읽을 수 없었어요",
  "scrobbling.reason.attempts_exhausted": "너무 여러 번 시도했어요",
  "scrobbling.reason.unlinked": "연결이 해제됐어요",
  "scrobbling.reason.ambiguous": "응답이 오지 않았어요",
  "scrobbling.reason.interrupted": "전송 도중 서버가 중지됐어요",
};
