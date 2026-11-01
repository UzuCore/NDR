# NDR r190 — 공개 웹 압축파일 실물 샘플 검수

검수 일자: 2026-11-01. 대상: NDR Rust 및 실제 동봉된 `7zip+ 26.05` (7z.exe, 7z.dll).

## 샘플 출처

원본 압축파일은 임의 실행파일 배포 사이트가 아닌 두 공개 테스트 프로젝트에서 다운로드했다.

- [libarchive/libarchive, 실제 형식별 테스트 픽스처](https://github.com/libarchive/libarchive/tree/348e5a2e27503f5bf114e298e2bd9ddcd59ab48e/libarchive/test) — `348e5a2e27503f5bf114e298e2bd9ddcd59ab48e`; `.uu` 파일은 내장된 원래 파일명을 확인한 후 일반 UUdecode 알고리즘으로 변환했다.
- [SharpCompress 테스트 픽스처](https://github.com/adamhathcock/sharpcompress/tree/6128ec51eec4ca9f7b7b9f44b77bdb0cd8b3bd97/tests/TestArchives/Archives) — `6128ec51eec4ca9f7b7b9f44b77bdb0cd8b3bd97`; ACE, ARJ, **WinZip에서 실제 생성한 ZIPX** 등의 샘플을 사용했다.

파일은 `target/archive-web-corpus/samples/` 아래에 별도로 저장했다. 사용한 URL·확장자·해시·크기·7-Zip 테스트 결과는 `target/archive-web-corpus/inventory.csv` 및 `inventory.json`에 기록했다. 이 디렉터리의 바이너리 샘플은 Git이나 릴리즈 ZIP에 포함하지 않는다.

## 외부 샘플 실제 검증 결과

| 범위 | 건수 | 결과 |
|---|---:|---|
| 실제 확보한 샘플 | **58** | 다운로드 및 원본 검증 자료 보존 |
| 동봉된 7zip+의 목록 조회·`t` 무결성 검사 모두 통과 | **47** | 읽을 수 있는 형식 및 압축 코덱 |
| 목록 조회 또는 무결성 검사 실패 | **11** | 정상 지원을 표시해서는 안 되는 파일·메서드 확인 |

대표 검수에 포함된 형식: RAR4·RAR5, ZIP, 다양한 ZIPX(BZip2/LZMA/PPMd/XZ/ZSTD), 7z(LZMA2/Copy/ZSTD 확장 코덱), CAB, LZH/LHA, CPIO, AR, ARJ, ISO9660, RPM, TAR, TGZ, TBZ, TXZ, Zstandard TAR, `.Z`, Lzip(`.lz`/`.tlz`), ACE.

엔진에서 재현한 실패 유형:

| 실패 유형 | 샘플 수 | 분류 |
|---|---:|---|
| ACE 파일 | 2 | 현 배포 엔진에 읽기 핸들러 없음 — 기존 정책대로 차단 |
| Lzip 파일 (`.lz`, `.tlz`, `.tar.lz`) | 3 | 엔진이 지원하지 않음 — 기존 정책대로 차단 |
| 7z 내부 Zstandard `Method = 04F71101` | 3 | **목록 조회는 통과하지만 실제 해제는 `Unsupported Method`**. r190부터 목록 단계에서 명시적으로 거부 |
| 특수 ZIPX (WinZip26.nocomp.multi) | 1 | 번들 엔진 목록 조회부터 실패 — 열기 중단 |
| libarchive 테스트 ZIP (`test_read_format_zip.zip`) | 1 | 목록은 되지만 `Headers Error`로 무결성 검사 실패. 모든 ZIP에 일반화하지 않음 |
| libarchive LZMA 이름이 `.tlz`인 테스트파일 | 1 | 목록은 되지만 `payload` 뒤의 추가 데이터 오류. 정확한 원본 컨테이너 의미를 따로 검토 필요 |

### 핵심 정정

`zstd`라고 일괄 차단하면 안 된다. **ZIPX-ZSTD와 TAR.ZST는 동봉 엔진으로 실제 목록/무결성 검사를 통과**했으나, 별도 확장 방식인 **7z-Zstandard(04F71101)** 만 실패했다. 따라서 r190은 `.7z` 목록에서 해당 메서드를 발견할 때만 초기에 거부한다. 알 수 없는 나머지 방법은 실제 해제/무결성 결과에 따라 판단해야 한다.

일반 포맷은 파일별 압축 방식, 암호화, 손상 여부가 다를 수 있으므로 76개 확장자 중 일부 정상 샘플을 검증했다고 해서 그 확장자의 모든 파일이 정상이라는 의미는 아니다. 이름이 다른 별칭도 동일 데이터의 재패키징이 아니라 별도로 분류해야 한다.

## NDR 경로 직접 검수

`tests/archive_web_corpus.rs`는 같은 58개 원본을 NDR 자체의 `archive::list()`로 검사하고, 작은 정상 픽스처에서 `archive::extract()`까지 테스트한다. 결과는 `target/archive-web-corpus/ndr-inventory.json`에 저장한다. 추가로 `src/archive.rs`의 회귀 테스트는 공개 7z-Zstandard 3건을 **사전 차단**하면서 ZIPX-Zstandard 2건은 계속 허용되는지 확인한다.

**실제 실행 결과:** 총 58개 중 NDR 내부 목록 조회 **43건 성공**, **15건 거부**, 대표 압축파일 **3건 실제 추출 성공**. 15건 거부는 무조건 결함이 아니다. ZIPX 특수 변종의 엔진 목록 오류, 7z ZSTD의 비지원 코덱, ACE/Lzip 미지원, 소스에 포함된 심볼릭 링크나 `|` 포함 Windows 비호환 파일명 등을 분리해서 기록했다. 위험한 링크/경로는 자동 우회해서 추출하지 않았다.

## 반복 실행 절차

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\archive_web_corpus.ps1
cargo test --release --test archive_web_corpus -- --nocapture
cargo test --release --lib real_web_corpus_rejects_7z_zstd_codec_before_opening_but_keeps_zipx_zstd
```

안전 정책: 외부 샘플은 최대 다운로드/해제 크기 상한을 두고, UUdecoded 바이너리를 실행하거나 설치하지 않는다. 외부 테스트는 내용 검사와 **격리된 디렉터리 안에서만 일부 선택 추출**을 수행한다. 시스템 폴더·다른 NDR 인스턴스는 변경하지 않는다.

한계: 58개 실물 샘플은 76개 등록 확장자 전체에 대해 일대일 대응하지 않으며, VHD/VHDX/DMG/QCOW2/WIM/다른 디스크 이미지 등은 별도 공개 소형 샘플이나 r189의 자체 생성 테스트를 계속 활용해야 한다.
