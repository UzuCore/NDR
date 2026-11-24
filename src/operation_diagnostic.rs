//! Typed app-owned file diagnostics, localized at the boundary that creates the display text.
use crate::compat::Language;
use std::path::Path;

pub(crate) enum OperationDiagnostic<'a> {
    RecursiveTransfer(&'a Path, &'a Path),
    RecursiveItem,
    CopyRoot,
    FileToFolder,
    FolderToFile,
    CopyDepth,
    DeleteDepth,
    SourceChanged,
    ResumeLarger,
    ResumeMismatch,
    SplitSize,
    SplitDestination(&'a Path),
    SplitName,
    SplitCollision(&'a str),
    SplitChanged,
    JoinNoInput,
    JoinMissingInput,
    JoinOverwriteInput,
    JoinMissingDirectory,
    ArchiveUnsupported(&'a Path),
    ArchiveEmptySelection,
    ArchiveMissingSelection(&'a str),
    ArchiveOverlapSelection(&'a str),
    ArchiveDestinationCollision(&'a str),
    ArchiveDecodedMismatch(&'a str),
    ArchiveDecodedParent,
    ArchivePartial(&'a str, usize),
}

impl OperationDiagnostic<'_> {
    pub(crate) fn render(self, language: Option<&Language>) -> String {
        fn format(language: Option<&Language>, key: &str, ko: &str, en: &str, values: &[&str]) -> String {
            let template = language.map_or_else(|| en.to_owned(), |pack| pack.localized(key, ko, en));
            crate::folder_compare::render_message(&template, values)
        }

        match self {
            Self::RecursiveTransfer(a, b) => format(language, "NDR.OP_DIAG_RECURSIVE_TRANSFER", "대상이 원본 폴더이거나 그 하위 폴더입니다: {0} → {1}", "Destination is the source folder or lies inside it: {0} -> {1}", &[&a.to_string_lossy(), &b.to_string_lossy()]),
            Self::RecursiveItem => format(language, "NDR.OP_DIAG_RECURSIVE_ITEM", "대상이 원본이거나 그 안에 있습니다.", "Destination is the source or lies inside it", &[]),
            Self::CopyRoot => format(language, "NDR.OP_DIAG_COPY_ROOT", "파일 시스템의 루트는 복사할 수 없습니다.", "A filesystem root cannot be copied", &[]),
            Self::FileToFolder => format(language, "NDR.OP_DIAG_FILE_TO_FOLDER", "폴더를 파일로 바꿀 수 없습니다.", "Cannot replace a directory with a file", &[]),
            Self::FolderToFile => format(language, "NDR.OP_DIAG_FOLDER_TO_FILE", "파일을 폴더로 바꿀 수 없습니다.", "Cannot replace a file with a directory", &[]),
            Self::CopyDepth => format(language, "NDR.OP_DIAG_COPY_DEPTH", "폴더 깊이가 안전한 복사 한도(256)를 초과했습니다.", "Directory nesting exceeds the safe copy limit (256)", &[]),
            Self::DeleteDepth => format(language, "NDR.OP_DIAG_DELETE_DEPTH", "폴더 깊이가 안전한 삭제 한도(256)를 초과했습니다.", "Directory nesting exceeds the safe delete limit (256)", &[]),
            Self::SourceChanged => format(language, "NDR.OP_DIAG_SOURCE_CHANGED", "복사 중 원본이 바뀌어 기존 대상 파일을 그대로 두었습니다.", "Source changed during copy; existing destination retained", &[]),
            Self::ResumeLarger => format(language, "NDR.OP_DIAG_RESUME_LARGER", "기존 파일이 원본보다 커서 복사를 이어갈 수 없습니다.", "The existing file is larger than the source and cannot be resumed", &[]),
            Self::ResumeMismatch => format(language, "NDR.OP_DIAG_RESUME_MISMATCH", "기존 파일과 원본의 앞부분이 달라 복사를 이어가지 않았습니다.", "The existing file does not match the start of the source; it was not resumed", &[]),
            Self::SplitSize => format(language, "NDR.OP_DIAG_SPLIT_SIZE", "분할 크기는 0보다 커야 합니다.", "Part size must be greater than zero", &[]),
            Self::SplitDestination(a) => format(language, "NDR.OP_DIAG_SPLIT_DESTINATION", "분할 결과를 저장할 폴더가 없습니다: {0}", "Split destination folder does not exist: {0}", &[&a.to_string_lossy()]),
            Self::SplitName => format(language, "NDR.OP_DIAG_SPLIT_NAME", "원본 파일 이름이 없습니다.", "Missing file name", &[]),
            Self::SplitCollision(a) => format(language, "NDR.OP_DIAG_SPLIT_COLLISION", "같은 이름의 분할 결과 파일이 이미 있습니다: {0}", "A split output part with this file name already exists: {0}", &[a]),
            Self::SplitChanged => format(language, "NDR.OP_DIAG_SPLIT_CHANGED", "분할하는 동안 원본 파일이 바뀌었습니다.", "The source file changed while it was being split.", &[]),
            Self::JoinNoInput => format(language, "NDR.OP_DIAG_JOIN_NO_INPUT", "합칠 입력 파일이 없습니다.", "There are no input files to join.", &[]),
            Self::JoinMissingInput => format(language, "NDR.OP_DIAG_JOIN_MISSING_INPUT", "입력 파일을 찾을 수 없습니다.", "An input file could not be found.", &[]),
            Self::JoinOverwriteInput => format(language, "NDR.OP_DIAG_JOIN_OVERWRITE_INPUT", "입력 파일을 결과 파일로 덮어쓸 수 없습니다.", "The output cannot overwrite an input file.", &[]),
            Self::JoinMissingDirectory => format(language, "NDR.OP_DIAG_JOIN_MISSING_DIRECTORY", "결과 폴더를 찾을 수 없습니다.", "The output folder could not be found.", &[]),
            Self::ArchiveUnsupported(a) => format(language, "NDR.OP_DIAG_ARCHIVE_UNSUPPORTED", "내장 7-Zip에서 지원하지 않는 압축 형식입니다: {0}", "Archive format not supported by bundled 7-Zip: {0}", &[&a.to_string_lossy()]),
            Self::ArchiveEmptySelection => format(language, "NDR.OP_DIAG_ARCHIVE_EMPTY_SELECTION", "압축 항목 선택에 빈 항목이 있습니다.", "Empty archive selection is not a valid entry", &[]),
            Self::ArchiveMissingSelection(a) => format(language, "NDR.OP_DIAG_ARCHIVE_MISSING_SELECTION", "선택한 압축 항목을 찾을 수 없습니다: {0}", "Selected archive entry was not found: {0}", &[a]),
            Self::ArchiveOverlapSelection(a) => format(language, "NDR.OP_DIAG_ARCHIVE_OVERLAP_SELECTION", "압축 파일에서 선택한 항목이 중복되거나 상위·하위 항목이 겹칩니다: {0}", "Archive selection contains duplicate or overlapping parent/child entries: {0}", &[a]),
            Self::ArchiveDestinationCollision(a) => format(language, "NDR.OP_DIAG_ARCHIVE_DESTINATION_COLLISION", "선택한 압축 항목이 대상에서 같은 경로를 사용합니다: {0}", "Selected archive entries would collide in the destination: {0}", &[a]),
            Self::ArchiveDecodedMismatch(a) => format(language, "NDR.OP_DIAG_ARCHIVE_DECODED_MISMATCH", "압축을 푼 항목이 원본 정보와 일치하지 않습니다: {0}", "Decoded entry mismatch: {0}", &[a]),
            Self::ArchiveDecodedParent => format(language, "NDR.OP_DIAG_ARCHIVE_DECODED_PARENT", "압축을 푼 항목의 상위 폴더를 확인할 수 없습니다.", "Decoded target parent missing", &[]),
            Self::ArchivePartial(a, b) => format(language, "NDR.OP_DIAG_ARCHIVE_PARTIAL", "{0}; 압축을 푼 항목 {1}개는 유지했습니다.", "{0}; {1} successful archive entries were preserved", &[a, &b.to_string()]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_parameters_are_literal_in_translated_diagnostics() {
        let root = std::env::temp_dir().join(format!("ndr-diag-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("fr.lang"), "[NDR]\nOP_DIAG_RECURSIVE_TRANSFER=Destination {1} sous {0}\n").unwrap();
        let pack = Language::load(&root, "fr");
        let diagnostic = OperationDiagnostic::RecursiveTransfer(Path::new("base-{1}"), Path::new("child-{0}"));
        assert_eq!(diagnostic.render(Some(&pack)), "Destination child-{0} sous base-{1}");
        std::fs::remove_dir_all(root).unwrap();
    }
}
