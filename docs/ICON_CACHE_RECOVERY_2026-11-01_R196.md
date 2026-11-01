# NDR r196 — 폴더 아이콘이 사라지거나 다른 아이콘으로 바뀌는 현상

점검일: 2026-11-01 / 대상: NDR Windows x64, r195 기반

## 실제 실행 화면 증거

- 현재 사용자 실행 프로세스: `D:\2026-10-29\NDR.exe` (Windows 프로세스 NDR, 창 제목 NDR)
- 실행 파일 SHA-256은 `D:\NDR\release\NDR.exe` r195와 일치했다. 별도 디렉터리에서 실행하더라도 같은 r195 바이너리다.
- 전경 NDR 화면 캡처(창은 유지, 다른 프로그램은 종료하지 않음): `target/icon-qa/ndr-running-20261101.png`
- 화면 왼쪽 `C:\Users\byunj\Downloads\bk3.7z` 내부의 `assets`, `build`, `config` 등은 이름이 보이지만 **폴더 아이콘이 비어 있음**.
- 오른쪽 `D:\NDR\release`의 폴더 아이콘은 정상 표시. 사용자 설정 `ShowIcons=true`, `UseOverlayIcons=true`.

## 원인 A — 일반 폴더와 드라이브의 서로 다른 아이콘을 동일 캐시 키로 저장

`src/ui/views.rs::any_path_icon()`은 `directory=true`인 모든 아이콘을 `<DIR>` 하나에 저장하면서, 이미지 조회에는 첫 번째로 전달된 **실제 경로**를 사용했다. `drive_icon()`도 드라이브 경로를 인자로 넘긴다. 따라서 드라이브의 일반 아이콘을 먼저 가져오면 모든 폴더에 드라이브 아이콘이 적용된다.

실제 사용자 Windows 환경에서 Win32 `SHGetFileInfoW(SHGFI_USEFILEATTRIBUTES)` 결과 확인:

| 조회 경로 | Shell 아이콘 번호 |
|---|---:|
| 합성 경로 `folder` / 일반 폴더 | 3 |
| `C:\` 드라이브 | 158 |
| `D:\` 드라이브 | 8 |

이들은 같은 아이콘이 아니다. 따라서 `<DIR>` 캐시가 드라이브 경로로 시작하는 것은 오류다.

**수정:** 모든 일반 폴더 `<DIR>` 캐시 요청을 고정된 가상 경로 `folder`와 디렉터리 속성으로 조회하게 했다. 실제 드라이브·특수 폴더 아이콘은 **기존 개별 경로 비동기 아이콘 조회**를 통해 별도로 표시한다.

## 원인 B — Shell 조회가 한 번 실패하면 영구히 빈 아이콘으로 유지

기존 `file_icons: HashMap<String, Option<TextureHandle>>`와 `overlay_icons: HashMap<PathBuf, Option<TextureHandle>>`는 일시적인 아이콘 조회 실패 결과 `None`을 성공과 똑같이 영구 캐시했다. 이후에는 같은 폴더·확장자·특수 경로에 대해 재조회하지 않았다. 특히 압축파일/FTP는 개별 경로 오버레이 대신 일반 폴더 `<DIR>`를 사용하므로 모든 폴더가 한꺼번에 비는 문제가 발생할 수 있었다.

**수정:**

1. 일반 파일/폴더 및 개별 경로 아이콘 실패 시간을 추적하고, 3초 후에만 조회를 재시도한다. 매 프레임 Shell을 호출하지 않는다.
2. 픽셀 데이터 크기가 맞지 않거나 **전체 알파가 0인 완전히 투명한 이미지**는 성공으로 저장하지 않는다.
3. 기본 폴더 아이콘 조회가 실패할 때 OS의 `SIID_FOLDER` 스톡 아이콘으로 대체한다. 이를 통해 압축파일 내부 폴더 목록도 일시적 Shell 오류에 영향을 받지 않는다.
4. 개별 경로 아이콘이 다시 성공하면 복구된 텍스처로 대체하고 실패 시각을 삭제한다. 512개 캐시 초기화 시 실패 시각도 정리한다.

## 테스트

- `ui::main_window_tests::archive_folder_icon_cache_never_uses_a_drive_root_or_virtual_entry_path`: 처음에 `C:\`를 조회하더라도 Shell 조회 경로가 `folder` 한 가지이며, 이후 압축파일/로컬 폴더가 같은 일반 폴더 아이콘을 받는지 확인한다.
- `ui::main_window_tests::transient_missing_or_invisible_folder_icon_recovers_instead_of_being_cached_forever`: 첫 번째 조회 실패, 두 번째 전체 투명 이미지, 세 번째 정상 이미지를 주입한다. 실패 재시도는 시간 제한이 있고 정상 복구되면 유지됨을 검증한다.
- `ui::main_window_tests::transient_local_overlay_failure_retries_and_replaces_the_generic_icon`: 개별 경로 아이콘 실패가 3초 후 재시도되어 기본 폴더 아이콘 대신 고유 아이콘이 나타나는지 확인한다.
- `native::tests::windows_stock_folder_icon_is_visible_and_has_valid_alpha`: OS 스톡 폴더 아이콘이 실제로 보이는 비트맵인지 검증한다.
- `ui::shortcut_tests::actual_bk3_archive_opens_and_navigates_without_top_panel_error`: 실물 `bk3.7z` 임시 복사본의 `assets` 폴더를 탐색할 때 **테스트용 가짜 아이콘이 아닌 실제 Windows Shell API**를 사용해 16×16 폴더 텍스처가 반환되는지 확인한다.
- 기존 압축파일 59개, `bk3.7z`, ISO 2개, 파일 복사·이동·삭제·폴더 트리·UI 단축키 회귀 테스트 유지.

## 사용자 실행 파일 및 배포 주의

원래 실행 중인 `D:\2026-10-29\NDR.exe` 프로세스는 **사용자가 열어 둔 창**이므로 강제 종료하거나 업데이트 파일을 실행 중 덮어쓰지 않는다. 수정된 프로그램은 별도로 `D:\NDR\release\NDR.exe`로 배포한다. 사용자 실행 폴더를 최신으로 만들려면 프로세스 종료 후 새 실행파일을 해당 위치에 복사·교체해야 한다. 실행 중 화면이 스스로 최신 코드로 바뀐다고 주장해서는 안 된다.

이번 수정은 화면에서 확인한 아이콘 캐시 오류에 대한 것이며, 모든 Shell 확장 및 모든 사용자 파일 아이콘의 전수 테스트를 의미하지 않는다.
