// Folders, filing and uploads.
import type { TranslationValue } from "..";
import type { folders as en } from "../en/folders";

export const folders: Record<keyof typeof en, TranslationValue> = {
  "folders.count": {
    one: "이 보관함에 만든 폴더 {count}개",
    other: "이 보관함에 만든 폴더 {count}개",
  },
  "folders.where": "폴더 경로",
  "folders.subfolders": "폴더",
  "folders.noLibrary": "이 계정에서 사용할 수 있는 보관함이 없어요.",
  "folders.none": "이 보관함에 아직 만든 폴더가 없어요.",
  "folders.system": "WaveFlow에서 관리",
  "folders.systemNote":
    "미분류에는 어느 폴더에도 들어 있지 않은 모든 곡이 모여요. WaveFlow가 직접 관리하기 때문에 위의 폴더 수에 포함되지 않고, 이름을 바꾸거나 이동하거나 삭제할 수 없어요.",
  "filed.unfiled": "미분류",
  "filed.automatic": "자동",
  "filed.topLevel": "최상위",
  "filed.here": "현재 위치",
  "filed.destinations": "선택할 수 있는 폴더",
  "filed.loadError": "폴더를 불러올 수 없어요.",
  "filed.moveFailed": "이동하지 못했어요. 다시 시도하세요.",
  "filed.moveTrack": "‘{title}’ 곡을 폴더로 이동",
  "filed.moveFolder": "‘{name}’ 이동",
  "filed.note":
    "WaveFlow 안에만 있는 폴더예요. 여기에 곡을 분류해도 파일은 이동하지 않아요.",
  "filed.newFolder": "새 폴더",
  "filed.name": "폴더 이름",
  "filed.renameLabel": "새 이름",
  "filed.create": "만들기",
  "filed.save": "저장",
  "filed.cancel": "취소",
  "filed.rename": "이름 바꾸기",
  "filed.move": "이동…",
  "filed.delete": "삭제",
  "filed.confirmDelete":
    "‘{name}’ 폴더와 하위 폴더를 삭제할까요? 들어 있던 곡은 미분류로 돌아가고, 파일은 건드리지 않아요.",
  "filed.created": "‘{name}’ 폴더를 만들었어요.",
  "filed.renamed": "‘{name}’(으)로 이름을 바꿨어요.",
  "filed.deleted": {
    one: "폴더를 삭제했어요. {count}곡이 미분류로 돌아갔어요.",
    other: "폴더를 삭제했어요. {count}곡이 미분류로 돌아갔어요.",
  },
  "filed.filed": {
    one: "{count}곡을 {folder}에 분류했어요.",
    other: "{count}곡을 {folder}에 분류했어요.",
  },
  "filed.folderMoved": "‘{name}’ 폴더를 이동했어요.",
  "filed.nameTaken": "같은 이름의 폴더가 이미 여기에 있어요.",
  "filed.refused":
    "그 폴더는 그곳으로 옮길 수 없어요. 이름을 확인하세요. 폴더 구조가 너무 깊거나 가득 찼을 수도 있어요.",
  "filed.failed": "실패했어요. 다시 시도하세요.",
  "filed.missing": "이 폴더는 더 이상 없어요.",
  "filed.empty": "아직 여기에 분류된 항목이 없어요.",
  "filed.allFiled": "모든 곡이 분류됐어요.",
  "filed.uploadHere": "여기에 업로드",
  "filed.unfiledNote":
    "아직 어느 폴더에도 들어 있지 않은 모든 곡이에요. WaveFlow가 이 목록을 직접 관리해요. 곡은 분류되는 즉시 목록에서 빠지고, 이 목록은 이름을 바꾸거나 삭제할 수 없어요. 곡을 분류하려면 곡 메뉴를 열고 ‘폴더로 이동…’을 선택하세요.",
  "filed.unfiledNoteReader":
    "아직 어느 폴더에도 들어 있지 않은 모든 곡이에요. WaveFlow가 이 목록을 직접 관리해요. 곡은 분류되는 즉시 목록에서 빠져요.",
  "upload.into": "받은 파일은 {folder} 폴더에 들어가요.",
  "upload.doneUnfiled": "보관함의 미분류에 추가됨",
  "upload.folderMissing":
    "이 업로드를 위해 열었던 폴더를 찾을 수 없어요. 받은 파일은 미분류로 들어가요.",
  "upload.folderUnchecked":
    "이 업로드를 위해 열었던 폴더를 확인할 수 없어서 아직 파일을 받지 않아요.",
  "upload.checkAgain": "다시 확인",
  "upload.title": "{library}에 업로드",
  "upload.detail":
    "각 파일은 먼저 브라우저에서 지문을 만들어요. 그래서 보관함에 이미 있는 파일은 다시 보내지 않고, 중단된 전송은 멈춘 지점부터 이어서 진행해요.",
  "upload.pick": "오디오 파일 선택",
  "upload.formats": "보관함에서 인식할 수 있는 형식: {extensions}",
  "upload.noLibrary": "먼저 보관함을 선택하세요.",
  "upload.notAllowed": "이 보관함의 소유자나 매니저만 파일을 추가할 수 있어요.",
  "upload.closed":
    "이 보관함은 파일을 받지 않아요. 서버 운영자가 정하는 설정이에요.",
  "upload.waiting": "대기 중",
  "upload.hashing": "지문 만드는 중…",
  "upload.negotiating": "서버에 확인하는 중…",
  "upload.uploading": "보내는 중…",
  "upload.committing": "보관함에 추가하는 중…",
  "upload.done": "보관함에 추가됨",
  "upload.present": "이미 보관함에 있음",
  "upload.refused": "거부됨: {reason}",
  "upload.failed": "실패: {reason}",
  "upload.progress": "진행률: {name}",
  "upload.reason.unsupportedFormat":
    "보관함에서 인식할 수 없는 오디오 형식이에요",
  "upload.reason.tooLarge": "서버가 받을 수 있는 크기보다 커요",
  "upload.reason.quotaExceeded": "보관함에 남은 공간이 없어요",
  "upload.reason.libraryClosed": "보관함이 파일을 받지 않아요",
  "upload.reason.tooManySessions":
    "이미 열린 전송이 너무 많아요. 잠시 후 다시 시도하세요",
  "upload.reason.refusedFile":
    "받은 내용이 파일과 일치하지 않거나 보관함에서 읽을 수 있는 오디오가 아니에요",
  "upload.reason.unavailable": "지금은 서버가 받을 수 없어요. 다시 시도하세요",
  "upload.reason.network": "연결이 끊겼어요",
  "upload.reason.notAllowed": "이제 이 보관함에 파일을 추가할 수 없어요",
  "upload.reason.error": "문제가 발생했어요",
  "upload.reason.hashing": "이 브라우저에서 파일의 지문을 만들 수 없어요",
};
