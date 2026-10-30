// Podcast subscriptions and episodes.
import type { TranslationValue } from "..";
import type { podcasts as en } from "../en/podcasts";

export const podcasts: Record<keyof typeof en, TranslationValue> = {
  "podcasts.detail": "이 서버가 모든 계정을 위해 구독하는 프로그램",
  "podcasts.off":
    "이 서버에서는 팟캐스트가 꺼져 있어요. 이전에 받아 둔 내용은 계속 볼 수 있지만 새로 가져오지는 않아요.",
  "podcasts.url": "피드 주소",
  "podcasts.shared":
    "모든 계정에 공개돼요. 이 서버의 누구나 프로그램을 보고 서버가 내려받은 에피소드를 재생할 수 있어요. 피드 주소는 관리자만 볼 수 있어요.",
  "podcasts.subscribe": "구독",
  "podcasts.refresh": "모든 피드 새로고침",
  "podcasts.refreshing": "피드를 다시 가져오는 중이에요.",
  "podcasts.reload": "다시 불러오기",
  "podcasts.empty": "아직 팟캐스트가 없어요.",
  "podcasts.emptyDetail":
    "관리자가 여기에서 프로그램의 피드를 구독하면 서버가 에피소드를 내려받고, 이 서버의 모든 계정에서 들을 수 있어요.",
  "podcasts.addFirst": "첫 팟캐스트 추가",
  "podcasts.channels": "프로그램",
  "podcasts.showing": "{title}의 에피소드를 표시하고 있어요.",
  "podcasts.noEpisodes": "이 피드에는 에피소드가 없어요.",
  "podcasts.episodes": {
    one: "에피소드 {count}개",
    other: "에피소드 {count}개",
  },
  "podcasts.failed": "마지막 가져오기 실패: {reason}",
  "podcasts.open": "에피소드",
  "podcasts.hide": "숨기기",
  "podcasts.unsubscribe": "구독 취소",
  "podcasts.download": "다운로드",
  "podcasts.play": "{title} 재생",
  "podcasts.pause": "{title} 일시정지",
  "podcasts.resumeAt": "{time}부터 이어서 재생",
  "podcasts.status.new": "다운로드 안 됨",
  "podcasts.status.downloading": "다운로드 중",
  "podcasts.status.completed": "준비됨",
  "podcasts.status.error": "다운로드 실패",
  "podcasts.status.deleted": "삭제됨",
  "podcasts.conflict": "이 서버는 이미 그 피드를 구독하고 있어요.",
  "podcasts.refused":
    "피드를 가져오거나 읽을 수 없어요. 또는 이 서버가 접근할 수 없는 주소예요.",
  "podcasts.subscribeError": "피드를 추가할 수 없어요.",
  "podcasts.mutationError": "실패했어요. 다시 시도하세요.",
  "podcasts.playbackError": "이 에피소드를 재생할 수 없어요.",
};
