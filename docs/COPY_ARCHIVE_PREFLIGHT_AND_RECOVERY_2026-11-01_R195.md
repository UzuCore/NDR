# NDR r195 — 선택 복사 실패 오판, 경로 충돌, 재귀 복사 부작용 수정

날짜: 2026-11-01. 기반: r194 / Git `0b1812a`. 대상: NDR Rust 파일관리자, Windows x64.

## 점검 결과 및 수정

### 1. 압축파일의 존재하지 않는 내부 항목 복사 요청이 성공으로 끝나는 버그

`src/archive.rs::extract_entries_with_program()`는 요청한 `keys`가 실제 목록에 없는 경우에도 선택 결과가 0개가 되므로, 오류 없이 끝났다. 클립보드·복사·이동 요청이 **파일을 복사하지 않고도 성공으로 표시**될 수 있었다. 이제 요청 항목 각각이 실제 압축파일 내부 항목이나 명시되지 않은 부모 폴더인지 확인한 후 임시 폴더를 생성한다. 부분적으로 존재하지 않는 다중 선택도 전체 오류로 처리한다. 빈 요청은 기존의 **전체 압축파일 추출** 의미를 유지한다.

### 2. 대소문자가 다른 내부 경로 선택 시 잘못된 목적지 폴더가 생성되는 버그

조회·선택 매칭에는 소문자 비교를 사용하지만, 원본 내부 경로에서 선택된 부모 경로를 벗기는 과정은 대소문자를 구별했다. 예: 원본 `Assets/Fonts/CaseSensitive.TXT`에서 `assets/fonts/casesensitive.txt`를 선택하면 `목적지/CaseSensitive.TXT` 대신 `목적지/Assets/Fonts/CaseSensitive.TXT`로 풀 수 있었다.

`selected_output_relative()`에서 **대소문자를 구별하지 않고 컴포넌트 단위로** 부모 경로를 제거하도록 수정했다. Unicode case-folding에 의해 바이트 길이가 달라질 수 있으므로, UTF-8 문자열을 소문자 길이로 잘라내지 않는다.

### 3. 서로 다른 내부 폴더에서 파일명 중복 선택 시 중간 작업 실패

선택한 `One/duplicate.txt`와 `Two/duplicate.txt`는 같은 목적지 `duplicate.txt`를 향한다. 기존에는 첫 번째 내부 파일을 추출한 후 뒤늦게 충돌하여 일부만 추출된 것처럼 작업이 끝날 수 있었다. 이제 선택한 루트 간 이름 충돌과 부모·자식 중복 선택을 **작업 시작 전** 감지해 명시적으로 거부한다. 기존 파일 덮어쓰기 여부는 계속 기존 충돌 다이얼로그가 처리한다.

### 4. 폴더를 자기 하위 위치로 복사할 때 빈 디렉터리를 만들어 놓는 버그

`src/fsops.rs::run(Operation::Copy)`는 목적지 디렉터리를 먼저 생성하고, 나중에야 원본과 목적지의 포함 관계를 검사했다. `source`를 `source/nested/new/copies`로 복사·이동하려면 원래 작업을 거부해야 하지만, **거부되기 전에 `source/nested/new/copies`가 생성**되는 부작용이 있었다.

이제 모든 디렉터리 원본에 대해 목적지에 대한 포함 관계를 먼저 검사하고, 자기 하위 복사·이동이 발견되면 전체 작업을 중단한다. 일반 목적지로의 복사는 기존대로 허용한다.

## 실행 검수

- `src/archive.rs::entry_tests::selective_copy_refuses_missing_overlapping_or_colliding_keys_and_preserves_casefolded_paths`: 실제 ZIP을 생성하고 내부 선택 누락/일부 누락/빈 이름/부모·자식 중복/서로 다른 폴더의 같은 이름/대소문자 불일치 복사를 검증한다. 거부된 작업은 출력 폴더를 만들지 않고 ZIP 원본 바이트를 유지한다.
- `src/fsops.rs::tests::recursive_copy_and_move_are_rejected_before_creating_destination_folders`: 임시 디렉터리와 파일을 생성하고 자기 하위 위치로의 복사와 이동을 각각 거부하는지, 사후에 목적지와 부모 폴더가 새로 생기지 않았는지 검사한다. 정상적인 복사는 계속 성공한다.
- `tests/archive_local_bk3.rs`: 사용자 보유 `C:\Users\byunj\Downloads\bk3.7z`의 실제 내부 파일을 대문자 경로로 요청했을 때 바깥 폴더로 제대로 복사되는지, 존재하지 않는 항목이 거부되는지 검증한다. 원본 SHA-256 전후 일치 및 Windows 예약 장치명 `nul` 차단은 유지한다.
- 기존 웹 실물 압축파일 59개/로컬 ISO 2개 및 GUI Ctrl+C/V·Ctrl+X/V·F3/F4 회귀 테스트를 그대로 유지한다.

## 범위·제한

이번 작업은 위에 기재한 **실제로 확인한 오류 경로**를 수정한 것이다. 전체 사용자 파일·압축 포맷·클립보드 프로그램·네트워크·파일시스템 동작의 완전 전수 검증을 의미하지 않는다. 원본 다운로드 파일을 수정하거나 GitHub에 업로드하지 않는다.
