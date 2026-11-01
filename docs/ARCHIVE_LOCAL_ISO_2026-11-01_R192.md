# NDR r192 — 사용자 다운로드 폴더의 실제 PS3 ISO 검수

2026-11-01 / Windows x64 / NDR Rust / 배포 동봉 7zip+ 26.05.

## 원본 읽기 검사 결과

다운로드 폴더에서 2개의 실제 PS3 디스크 이미지를 확인했다. ISO 원본은 **읽기 전용**으로 취급하며 테스트 중 복사본 생성·삭제·이동을 하지 않는다.

| 실제 ISO | 크기 (바이트) | UDF 자동 인식 | `7z l -tiso` | ISO9660 내부 목록 수 |
|---|---:|---|---|---:|
| 사용자 보유 PS3 ISO A | 7,348,289,536 | `Headers Error`, exit 2 | exit 0 | 47 |
| 사용자 보유 PS3 ISO B | 8,401,715,200 | `Headers Error`, exit 2 | exit 0 | 50 |

`7zip+`는 UDF로 자동 인식하고 UDF 메타데이터 오류를 반환하지만, 같은 ISO의 ISO9660 디렉터리는 정상이다. 자동 탐색 결과가 실패한 `.iso`에서만 `-tiso`로 목록을 다시 검사하도록 `archive::list_details()`를 변경했다. **자동 인식이 성공하는 UDF 이미지에는 ISO9660을 강제하지 않는다.** 두 방법이 모두 실패하면 그대로 오류를 반환한다. 내부 항목의 파일명 안전 검사·심볼릭 링크 차단도 유지한다.

## 실제 추출 검증

`7zip+ x -tiso`로 각 ISO의 `PS3_DISC.SFB`(각 1,536바이트)와 `PS3_GAME/ICON0.PNG`(각각 58,310/80,016바이트)를 실제 추출하여 4건 모두 성공했다. 테스트 폴더의 임시 파일만 삭제했다.

NDR의 동일한 핸들러 선택을 다음 코드 경로에 연결했다.

- `archive::list()` / `list_details()` — 목록 검색 시 UDF 실패 후 ISO9660 재시도.
- `archive::extract_entries_with_program()` — Ctrl+C/클립보드 준비와 선택 파일 복사 및 대화형 추출.
- `archive::extract_entry_to_temp()` — 압축 안에서 파일 열기/미리보기.
- `archive::extract_with_conflicts()` — 전체 추출 시에도 동일 핸들러 사용.

`tests/archive_local_iso.rs`는 `NDR_LOCAL_ISO_FILES` 환경변수가 있을 때 두 ISO에 대해 NDR의 실제 내부 목록·선택 파일 복사·미리보기, 내부 변경 불가(`can_modify=false`), 원본 전체 SHA-256의 전후 일치 여부를 검사한다. 환경변수를 지정하지 않으면 개인 PC에 없는 ISO를 필수 테스트로 요구하지 않는다. 손상된 가짜 ISO에서 자동 인식과 강제 ISO9660 인식이 모두 실패해야 한다는 회귀 테스트도 제공한다.

**NDR API 실물 검수 완료:** 두 ISO에서 목록 **47/50개**, 선택 내부 파일 복사 **각 2건(총 4건)**, 내부 파일 열기용 미리보기 **각 1건(총 2건)** 모두 PASS. 원본 ISO 전체의 SHA-256을 작업 전후 각각 계산하여 **2개 모두 일치**함을 확인하였다. 가짜 ISO 거부 테스트도 PASS(`cargo test --release --test archive_local_iso -- --nocapture`: 2 passed, 0 failed). 추출된 소형 파일의 SHA-256은 `7zip+ -tiso`에서 직접 선택 추출한 결과와 일치하였다. 개별 원본 SHA-256과 출력 파일 경로/크기는 Git에 업로드하지 않고 `target/iso-real-qa/ndr-local-iso-results.json`에 기록했다.

ISO는 압축파일 내부 항목을 수정 가능한 형식이 아니다. **ISO 안으로 파일 추가, 내부 항목 삭제 및 내부 이동은 의도적으로 차단**한다. 일반 파일로서 ISO 자체를 복사·이동·삭제하는 파일시스템 작업은 별개다. 사용자가 소장한 원본 ISO에 이러한 파괴적 작업을 시도하지 않았다. 7GB/8GB ISO의 *전체 내용 추출*은 이번 검수에서 시도하지 않았다.

검수 로그는 `target/iso-real-qa/`에 있다. 사용자 소유 ISO 파일은 Git 저장소나 릴리즈 ZIP에 포함하지 않는다.
