//! Decision part of C# CheckForUpdatesAsync: HTTPS JSON feed ({"version": "x.y[.z[.w]]", "url": "https://..."}), 256 KiB limit,
//! recognizing the old NexusFile feed (LegacyUpdateFeedDetector: "NXUPDATEINFO1", 12 digits, version), System.Version comparison.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const GITHUB_LATEST_URL: &str = "https://api.github.com/repos/UzuCore/NDR/releases/latest";
pub const MAX_PACKAGE_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GithubRelease {
    pub build: u64,
    pub tag: String,
    pub url: String,
    pub size: usize,
    pub sha256: String,
}

pub fn current_build() -> u64 {
    option_env!("NDR_BUILD_NUMBER").and_then(|n| n.parse().ok()).unwrap_or(0)
}

/// Refuse untrusted asset URLs, malformed digests, prereleases and downgrades.
pub fn parse_github_release(body: &[u8], installed: u64) -> Result<Option<GithubRelease>, String> {
    let json: serde_json::Value = serde_json::from_slice(body).map_err(|e| e.to_string())?;
    if json["draft"].as_bool() != Some(false) || json["prerelease"].as_bool() != Some(false) {
        return Err("GitHub did not return a published stable release".into());
    }
    let tag = json["tag_name"].as_str().ok_or("Missing release tag")?;
    let build = tag.strip_prefix("build-").and_then(|value| value.parse::<u64>().ok()).filter(|n| *n > 0)
        .ok_or("Invalid build tag")?;
    if build <= installed { return Ok(None); }
    let asset = json["assets"].as_array().ok_or("Release assets missing")?.iter()
        .find(|a| a["name"] == "NDR-windows-x64.zip").ok_or("Complete NDR ZIP missing")?;
    let expected_url = format!("https://github.com/UzuCore/NDR/releases/download/{tag}/NDR-windows-x64.zip");
    if asset["browser_download_url"].as_str() != Some(expected_url.as_str()) { return Err("Unexpected release download URL".into()); }
    let size = asset["size"].as_u64().and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n > 0 && *n <= MAX_PACKAGE_BYTES).ok_or("Unsafe release ZIP size")?;
    let sha256 = asset["digest"].as_str().and_then(|value| value.strip_prefix("sha256:"))
        .filter(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .ok_or("Release ZIP has no trustworthy SHA-256 digest")?.to_ascii_lowercase();
    Ok(Some(GithubRelease { build, tag: tag.to_owned(), url: expected_url, size, sha256 }))
}

/// Runs off the GUI thread, checks TLS, response size and GitHub's published digest.
pub fn download_github_update(
    release: &GithubRelease,
    get: fn(&str, usize) -> Result<HttpResponse, HttpError>,
) -> Result<PathBuf, String> {
    download_github_update_with_progress(release, |url, max| get(url, max), |_| {})
}

/// HTTP progress callback reports bytes downloaded; phase callback describes
/// SHA-256 verification and staging (not a fictitious installation percent).
pub fn download_github_update_with_progress(
    release: &GithubRelease,
    get: impl FnOnce(&str, usize) -> Result<HttpResponse, HttpError>,
    mut phase: impl FnMut(UpdatePhase),
) -> Result<PathBuf, String> {
    let response = get(&release.url, MAX_PACKAGE_BYTES).map_err(|e| format!("Download failed: {e:?}"))?;
    phase(UpdatePhase::Verifying);
    if response.status != 200 || !response.secure { return Err("Update download was not secure or HTTP 200".into()); }
    if response.body.len() != release.size { return Err("Update ZIP size differs from GitHub release".into()); }
    let digest = format!("{:x}", Sha256::digest(&response.body));
    if digest != release.sha256 { return Err("Update SHA-256 verification failed".into()); }
    phase(UpdatePhase::Preparing);
    let folder = std::env::temp_dir().join(format!("ndr-update-{}-{}-{}", release.tag,
        std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?.as_nanos()));
    std::fs::create_dir(&folder).map_err(|e| e.to_string())?;
    let archive = folder.join("NDR-windows-x64.zip");
    std::fs::write(&archive, &response.body).map_err(|e| e.to_string())?;
    Ok(archive)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatePhase { Downloading, Verifying, Preparing, Installing, Failed }

/// The script is external to NDR so that Windows can replace NDR.exe after exit.
pub fn launch_update(archive: &Path) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let installation = executable.parent().ok_or("Installation directory not found")?;
    if !installation.join("Skins").is_dir() || !installation.join("Languages").is_dir() {
        return Err("Run the updater from a complete installed NDR folder".into());
    }
    let folder = archive.parent().ok_or("Update staging folder missing")?;
    let script = folder.join("apply-update.ps1");
    std::fs::write(&script, include_str!("apply-update.ps1")).map_err(|e| e.to_string())?;
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Sta", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
        .arg(&script).arg("-Archive").arg(archive)
        .arg("-Destination").arg(installation)
        .arg("-ParentPid").arg(std::process::id().to_string())
        .arg("-LogPath").arg(folder.join("update.log"))
        .arg("-ShowProgress")
        .spawn().map_err(|e| format!("Could not launch updater: {e}"))?;
    Ok(())
}

/// HTTPS GET result (status code, whether the final address was HTTPS, body).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u32,
    pub secure: bool,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HttpError {
    /// No response within 15 seconds.
    Timeout,
    /// The body exceeds the limit.
    TooLarge,
    /// Connection failed (message).
    Failed(String),
}

pub const MAX_MANIFEST_BYTES: usize = 256 * 1024;

/// System.Version.TryParse: 2 to 4 non-negative integers separated by dots.
pub fn parse_version(text: &str) -> Option<Vec<u32>> {
    let parts: Vec<&str> = text.trim().split('.').collect();
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    parts.iter().map(|p| p.trim().parse::<u32>().ok().filter(|v| *v <= i32::MAX as u32)).collect()
}

/// System.Version comparison (missing parts count as -1: 2.0 < 2.0.0).
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

/// LegacyUpdateFeedDetector.TryReadNexusFileVersion: Ok(Some(version)) for a NexusFile feed, otherwise Ok(None), Err when broken.
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

/// Decision after reading the feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Manifest {
    /// A NexusFile feed (its version).
    Legacy(String),
    /// A new version (version text, download address).
    Newer(String, String),
    UpToDate,
    /// Language file key (InvalidVersion, InvalidDownloadUrl).
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
    fn github_release_requires_exact_package_digest_and_newer_build() {
        let body = br#"{"tag_name":"build-12","draft":false,"prerelease":false,"assets":[{"name":"NDR-windows-x64.zip","size":123,"digest":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","browser_download_url":"https://github.com/UzuCore/NDR/releases/download/build-12/NDR-windows-x64.zip"}]}"#;
        let release = parse_github_release(body, 11).unwrap().unwrap();
        assert_eq!((release.build, release.size), (12, 123));
        assert_eq!(parse_github_release(body, 12), Ok(None));
        assert_eq!(parse_github_release(body, 13), Ok(None));
        let bad_digest = String::from_utf8(body.to_vec()).unwrap().replace("sha256:aaaa", "sha256:xxxx");
        assert!(parse_github_release(bad_digest.as_bytes(), 11).is_err());
        let bad_host = String::from_utf8(body.to_vec()).unwrap().replace("https://github.com/UzuCore/NDR/releases/download/", "https://example.invalid/");
        assert!(parse_github_release(bad_host.as_bytes(), 11).is_err());
        let draft = String::from_utf8(body.to_vec()).unwrap().replace("\"draft\":false", "\"draft\":true");
        assert!(parse_github_release(draft.as_bytes(), 11).is_err());
    }

    #[test]
    fn update_download_rejects_bad_body_hash() {
        fn fake(_: &str, _: usize) -> Result<HttpResponse, HttpError> {
            Ok(HttpResponse { status: 200, secure: true, body: vec![1, 2, 3] })
        }
        let release = GithubRelease { build: 12, tag: "build-12".into(),
            url: "https://github.com/UzuCore/NDR/releases/download/build-12/NDR-windows-x64.zip".into(),
            size: 3, sha256: "0".repeat(64) };
        assert!(download_github_update(&release, fake).is_err());
    }

    #[test]
    fn verified_update_advances_to_staging_without_an_extra_approval() {
        fn fake(_: &str, _: usize) -> Result<HttpResponse, HttpError> {
            Ok(HttpResponse { status: 200, secure: true, body: b"test update zip".to_vec() })
        }
        let payload = b"test update zip";
        let release = GithubRelease { build: 101, tag: "build-101".into(),
            url: "https://github.com/UzuCore/NDR/releases/download/build-101/NDR-windows-x64.zip".into(),
            size: payload.len(), sha256: format!("{:x}", Sha256::digest(payload)) };
        let mut phases = Vec::new();
        let archive = download_github_update_with_progress(&release, fake, |phase| phases.push(phase)).unwrap();
        assert_eq!(phases, [UpdatePhase::Verifying, UpdatePhase::Preparing]);
        assert_eq!(std::fs::read(&archive).unwrap(), payload);
        std::fs::remove_dir_all(archive.parent().unwrap()).unwrap();
        let mut bad_release = release;
        bad_release.sha256 = "0".repeat(64);
        let mut phases = Vec::new();
        assert!(download_github_update_with_progress(&bad_release, fake, |phase| phases.push(phase)).is_err());
        assert_eq!(phases, [UpdatePhase::Verifying]);
    }

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
