# NDR r191 — 실제 웹 압축파일 복사·이동·삭제 검수

검수일: 2026-11-01. Windows x64, NDR Rust + 배포 동봉 `7zip+ 26.05`.

## 실제 검수한 범위

공개 테스트 저장소 [libarchive](https://github.com/libarchive/libarchive/tree/348e5a2e27503f5bf114e298e2bd9ddcd59ab48e/libarchive/test), [SharpCompress](https://github.com/adamhathcock/sharpcompress/tree/6128ec51eec4ca9f7b7b9f44b77bdb0cd8b3bd97/tests/TestArchives/Archives)의 **실제 압축파일 59개**를 사용했다. r190의 58건에 수정 가능한 정상 TAR 1개를 추가했다. 두 원본 Git 커밋과 모든 바이너리의 SHA-256, 다운로드 URL은 `target/archive-web-corpus/inventory.csv`에 고정돼 있다.

기존의 단순 `7z l` / `7z t` 검사가 아니라, `tests/archive_web_corpus_operations.rs`가 실제 NDR Rust API를 사용하여 다음을 검사했다.

- **압축파일 → 폴더 복사:** UI의 Ctrl+C/Ctrl+X 클립보드 준비가 사용하는 `extract_entries_with_program()`으로 원본 아카이브의 *선택된 파일*을 추출한다. 실제 파일 생성과 원본 유지 검사.
- **폴더 → 압축파일 복사:** ZIP·7z·TAR 테스트 복사본에 `add_entries()`로 테스트 파일을 추가하고 목록에서 확인한다.
- **폴더 → 압축파일 이동:** 추가 성공·목록 반영을 검증한 *뒤에만* 격리된 로컬 소스 파일을 삭제한다. 실패 시 소스 보존 검사.
- **압축파일 → 폴더 이동:** 선택 파일 추출 성공 후, 압축파일의 별도 테스트 복사본에서 그 항목을 삭제한다. 이동된 출력과 원래 항목의 부재 검사.
- **압축파일 내부 항목 삭제:** 별도 복사본에서 `delete_entries()`를 실행한 뒤 항목 제거·형제 항목 보존 검사.
- **읽기 전용 형식:** 내부 추가/삭제가 거부되고 원본과 테스트 복사본의 SHA-256이 바뀌지 않는지 검사.

**주의:** 다운로드한 원본 압축파일 자체에 어떠한 쓰기 작업도 하지 않았다. 모든 추가·이동·삭제는 `target/archive-web-corpus/ops-isolated-...` 아래의 일회용 복사본만을 대상으로 진행했다. 테스트 종료 시 이 임시 디렉터리만 정리했다. 다른 프로그램/실행 중인 NDR 인스턴스를 종료하지 않았다.

## 최종 결과 (실제 NDR API 실행)

| 항목 | 결과 | 해석 |
|---|---:|---|
| 다운로드 원본 압축파일 | **59건** | SHA-256 및 원본 무결성 확인 |
| NDR 목록 허용 | **44건** | 나머지 15건은 미지원·안전성 문제로 차단 |
| 선택 파일 복사 성공 | **42건** | 파일의 실제 선택 추출과 출력 확인 |
| 선택 복사 실패 | **1건** | `test_read_format_zip.zip` — 원본 자체가 7zip+ `t`에서 `Headers Error`를 반환하는 비정상 ZIP |
| 선택 복사 대상 없음 | **1건** | `test_read_format_iso.iso` — 정상적으로 비어 있는 ISO 픽스처 |
| 압축파일 내부 추가/복사 성공 | **4건** | 정상 ZIP 1건, 7z 2건, TAR 1건 |
| 압축파일 안으로 이동 성공 | **4건** | 정상 ZIP 1건, 7z 2건, TAR 1건 |
| 압축파일 밖으로 이동 성공 | **4건** | 정상 ZIP 1건, 7z 2건, TAR 1건 |
| 압축파일 내부 삭제 성공 | **4건** | 정상 ZIP 1건, 7z 2건, TAR 1건 |
| 읽기 전용 포맷 내부 수정 거부 | **50건** | RAR·RAR5·ZIPX·CAB·LZH·CPIO·이미지 등에서 추가와 삭제 차단 |
| 목록 단계 선차단 | **15건** | ACE, Lzip, 7z-ZSTD 확장 코덱, 손상 ZIPX, 비호환 파일명, 링크 포함 등 |

하나의 정상 압축파일에 대해 복사, 내부 추가, 안으로 이동, 밖으로 이동, 삭제는 **각각 독립적인 복사본**에서 확인했다. 50건의 읽기 전용 샘플에서는 내부 추가/삭제 거부가 의도된 결과이며, 압축파일 자체를 일반 파일로 복사·이동·삭제하는 Windows 파일 시스템 작업과는 다르다.

실제 결과는 `target/archive-web-corpus/ndr-operations-inventory.json`에 샘플별 `copy_out`, `copy_in`, `move_out`, `move_in`, `delete_inside`가 저장되어 있다.

## r191에서 발견·수정한 결함

1. **단일 스트림 목록만 성공하는 문제:** 번들 `7zip+`가 XZ/TXZ, Zstandard/TAR.ZST, compress `.Z` 등에서 `l -slt -ba` 결과에 `Path =`를 빼거나 `Size =`만 기록하여, NDR에서는 목록은 성공했지만 내부 항목이 0개여서 선택 복사를 시작할 수 없었다. 출력 파일명의 실제 접미사 규칙을 적용하여 **실재하는 단일 멤버 이름**을 복원하도록 수정했다.
2. **RPM bzip2:** `-ba` 결과에는 이름이 없으나 일반 상세 목록에는 RPM의 내부 cpio.bz2 파일명이 있다. 별도의 상세 목록에서 이 파일명을 읽고 실제 추출 파일명인 `.cpio`로 복원한다. RPM 바깥 파일명을 내부 파일명으로 추정하지 않는다.
3. **연속 GZIP 멤버의 실제 해제 크기 불일치:** `test_compat_gzip_1.tgz`는 `7z l`의 크기 168바이트와 실제 `7z x` 결과 7,168바이트가 다르다. 잘못된 크기 기준 때문에 `Decoded entry mismatch`로 정상 복사가 실패했다. GZIP 스트림의 이 크기를 알 수 없는 크기로 취급하고, 실제 추출 결과·경로·유효성을 검사하도록 수정했다.
4. **손상된 ZIP은 복사를 중단:** `test_read_format_zip.zip`은 7zip+ 목록이 출력되나 실제 검사에서 `Headers Error`를 낸다. NDR에서 실패를 알리고 부분 출력이나 원본 손상을 남기지 않음을 확인했다. 정상 ZIP으로 취급하지 않는다.

## 반복 검증

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\archive_web_corpus.ps1
cargo test --release --test archive_web_corpus_operations -- --nocapture
cargo test --release --test archive_web_corpus -- --nocapture
cargo test --release
```

참고: 이 테스트는 **실제 파일·압축 데이터를 사용한 NDR Rust 핵심 작업 API 검수**이며, 모든 59개 파일을 화면에서 직접 클릭한 GUI 수동 테스트는 아니다. GUI 키보드 Ctrl+C/Ctrl+X/Ctrl+V는 기존 `src/ui/shortcut_tests.rs` 테스트로 별도 검수된다. ZIPX·RAR을 비롯한 수정 불가 형식은 *압축파일 안의 항목 삭제/이동 자체를 허용하지 않는 정책*을 유지한다.

### 전체 회귀 검수에서 추가 확인한 단축키 문제

전체 테스트 도중 압축파일과 무관한 **Shift+드라이브 문자** 단축키 검수가 실패하여 단독 재현하였다. `egui::Event::Text("C")`가 Shift+C와 함께 전달될 때, 빠른 검색이 문자 입력을 먼저 소비하여 드라이브 이동 명령이 실행되지 않았다. 실제 드라이브가 있고 해당 기능이 활성화된 경우에는 빠른 검색보다 원래 KeyDown의 드라이브 이동 명령을 먼저 처리하도록 `src/ui/keyboard.rs`를 최소 수정했다. 기존 빠른 검색 및 단축키 회귀 검사도 전체 빌드 검수에 포함한다.
