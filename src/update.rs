//! C# CheckForUpdatesAsync의 판단 부분: HTTPS JSON 피드({"version": "x.y[.z[.w]]", "url": "https://…"}), 256 KiB 한도,
//! NexusFile 옛 피드(LegacyUpdateFeedDetector: "NXUPDATEINFO1", 12자리 숫자, 버전) 알아보기, System.Version 비교.

/// HTTPS GET 결과(상태 코드, 마지막 주소가 HTTPS였는지, 본문).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u32,
    pub secure: bool,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// 15초 안에 응답 없음.
    Timeout,
    /// 본문이 한도를 넘음.
    TooLarge,
    /// 연결 실패(메시지).
    Failed(String),
}

pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

/// System.Version.TryParse: 점으로 나눈 2~4개의 0 이상 정수.
pub fn parse_version(text: &str) -> Option<Vec<u32>> {
    let parts: Vec<&str> = text.trim().split('.').collect();
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    parts.iter().map(|p| p.trim().parse::<u32>().ok().filter(|v| *v <= i32::MAX as u32)).collect()
}

/// System.Version 비교(없는 자리는 -1로 본다: 2.0 < 2.0.0).
pub fn compare_versions(a: &[u32], b: &[u32]) -> std::cmp::Ordering {
    for i in 0..4 {
        let x = a.get(i).map_or(-1i64, |v| *v as i64);
        let y = b.get(i).map_or(-1i64, |v| *v as i64);
        if x != y {
            return x.cmp(&y);
        }
    }
    std::cmp::Ordering::Equal
}

pub fn version_text(version: &[u32]) -> String {
    version.iter().map(u32::to_string).collect::<Vec<_>>().join(".")
}

/// LegacyUpdateFeedDetector.TryReadNexusFileVersion: NexusFile 피드면 Ok(Some(버전)), 아니면 Ok(None), 깨졌으면 Err.
pub fn legacy_nexusfile_version(content: &[u8]) -> Result<Option<String>, String> {
    let Ok(text) = std::str::from_utf8(content) else { return Ok(None) };
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.first() != Some(&"NXUPDATEINFO1") {
        return Ok(None);
    }
    let version = lines.get(2).and_then(|l| parse_version(l));
    if lines.len() < 3 || lines[1].len() != 12 || !lines[1].bytes().all(|b| b.is_ascii_digit()) || version.is_none() {
        return Err("The NexusFile update feed is malformed and has no valid version record.".into());
    }
    if lines.iter().skip(3).any(|l| !l.trim().is_empty()) {
        return Err("The NexusFile update feed contains unsupported extra records.".into());
    }
    Ok(version.map(|v| version_text(&v)))
}

/// 피드 읽은 뒤의 판단.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Manifest {
    /// NexusFile 피드(그 버전).
    Legacy(String),
    /// 새 버전(버전 글, 다운로드 주소).
    Newer(String, String),
    UpToDate,
    /// 언어 파일 키(InvalidVersion, InvalidDownloadUrl).
    Invalid(&'static str),
}

pub fn evaluate(body: &[u8], current: &[u32]) -> Result<Manifest, String> {
    if let Some(version) = legacy_nexusfile_version(body)? {
        return Ok(Manifest::Legacy(version));
    }
    let json: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    let Some(latest) = json.as_object().and_then(|o| o.get("version")).and_then(|v| v.as_str()).and_then(parse_version) else {
        return Ok(Manifest::Invalid("InvalidVersion"));
    };
    if compare_versions(&latest, current) != std::cmp::Ordering::Greater {
        return Ok(Manifest::UpToDate);
    }
    match json.get("url").and_then(|v| v.as_str()).filter(|u| u.len() > 8 && u[..8].eq_ignore_ascii_case("https://")) {
        Some(url) => Ok(Manifest::Newer(version_text(&latest), url.to_owned())),
        None => Ok(Manifest::Invalid("InvalidDownloadUrl")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_and_versions_follow_csharp() {
        assert_eq!(parse_version("2.1"), Some(vec![2, 1]));
        assert_eq!(parse_version("1"), None);
        assert_eq!(parse_version("1.2.3.4.5"), None);
        assert_eq!(compare_versions(&[2, 0], &[2, 0, 0]), std::cmp::Ordering::Less);
        let current = [2, 0, 0];
        assert_eq!(evaluate(br#"{"version":"2.1","url":"https://example.invalid/ndr"}"#, &current), Ok(Manifest::Newer("2.1".into(), "https://example.invalid/ndr".into())));
        assert_eq!(evaluate(br#"{"version":"2.1","url":"http://example.invalid/ndr"}"#, &current), Ok(Manifest::Invalid("InvalidDownloadUrl")));
        assert_eq!(evaluate(br#"{"version":"2.0.0"}"#, &current), Ok(Manifest::UpToDate));
        assert_eq!(evaluate(br#"{"version":3}"#, &current), Ok(Manifest::Invalid("InvalidVersion")));
        assert_eq!(evaluate(b"\xef\xbb\xbfNXUPDATEINFO1\r\n202610270000\r\n5.4.1.5623\r\n", &current), Ok(Manifest::Legacy("5.4.1.5623".into())));
        assert!(evaluate(b"NXUPDATEINFO1\n12\n5.4\n", &current).is_err());
        assert!(evaluate(b"not json", &current).is_err());
    }
}
