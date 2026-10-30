// The attention lists and artist splits.
import type { TranslationValue } from "..";
import type { attention as en } from "../en/attention";

export const attention: Record<keyof typeof en, TranslationValue> = {
  "attention.lists": "목록",
  "attention.total": {
    one: "확인할 항목 {count}개",
    other: "확인할 항목 {count}개",
  },
  "attention.nothing": "확인할 항목이 없어요.",
  "attention.notYours":
    "이 보관함에서 확인할 항목은 소유자와 매니저만 볼 수 있어요.",
  "attention.empty": "이 목록은 비어 있어요.",
  "attention.tracksNote":
    "곡 메뉴에서 수정하세요. 수정한 곡은 목록에서 빠져요.",
  "attention.creditsNote":
    "아래 크레딧은 하나의 문자열에 여러 아티스트를 담고 있어요. 이름에 담긴 아티스트별로 분리하거나, 한 아티스트의 이름이라면 하나로 유지하세요. 이후의 모든 스캔에 이 결정이 적용돼요.",
  "attention.pending": "결정 필요",
  "attention.decided": "결정됨",
  "attention.split": "분리",
  "attention.confirmSplit": "확인: {count}명으로 분리",
  "attention.keep": "하나로 유지",
  "attention.splitDone": "{name} 크레딧을 분리했어요.",
  "attention.keepDone": "{name} 크레딧을 하나로 유지해요.",
  "attention.undone": "{name}에 대한 결정을 취소했어요.",
  "attention.alreadyDecided":
    "그사이 이 크레딧에 대한 결정이 내려졌어요. 목록을 다시 불러왔어요.",
  "attention.failed": "실패했어요. 다시 시도하세요.",
  "attention.noCredits": "결정을 기다리는 크레딧이 없어요.",
  "attention.noDecisions": "아직 내린 결정이 없어요.",
  "attention.fragments": "{name} 크레딧의 분리 결과",
  "attention.knownFragment": "이미 이 보관함에 있는 아티스트",
  "attention.decidedSplit": "{parts}(으)로 분리됨",
  "attention.decidedKeep": "하나로 유지됨",
  "attention.decidedBy": "{who}, {when}",
  "attention.undo": "실행 취소",
  "attention.duplicateGroup": "중복 의심 {group}",
  "attention.split_credits": "분리할 아티스트",
  "attention.missing_artist": "아티스트 누락",
  "attention.missing_album": "앨범 누락",
  "attention.missing_year": "연도 누락",
  "attention.missing_track_number": "트랙 번호 누락",
  "attention.missing_cover": "커버 누락",
  "attention.album_year_conflict": "연도가 여러 개인 앨범",
  "attention.compilation_no_album_artist": "앨범 아티스트가 없는 컴필레이션",
  "attention.duplicate_track_number": "중복된 트랙 번호",
  "attention.track_number_gap": "건너뛴 트랙 번호",
  "attention.probable_duplicate": "중복 의심",
  "attention.probable_fake_lossless": "가짜 무손실 의심",
  "artistSplit.offer": "아티스트 {count}명으로 분리",
  "artistSplit.confirm": "확인: {count}명으로 분리",
  "artistSplit.done": "{parts}(으)로 분리했어요.",
  "artistSplit.failed": "이 크레딧을 분리할 수 없어요. 다시 시도하세요.",
};
