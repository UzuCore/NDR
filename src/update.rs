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

/// The build number is only embedded in GitHub Actions distributions.
pub fn build_label(build: u64) -> Option<String> {
    (build > 0).then(|| format!("build-{build}"))
}

/// One-shot compatibility for an update launched by a previous installer
/// that did not pass --updated. Only a matching, recent successful build
/// session is eligible. Never modify user settings or create a cache on a
/// normal first launch without an existing completed session.
pub fn consume_recent_update(updates:&Path, build:u64, executable:&Path)->bool {
    use std::{fs,io::Read,time::{SystemTime,Duration}};
    if build==0 || crate::fsops::ensure_plain_path(updates).is_err(){return false}
    let Ok(installed)=fs::metadata(executable).and_then(|m|m.modified())else{return false};
    let Ok(items)=fs::read_dir(updates)else{return false};
    let prefix=format!("ndr-update-build-{build}-");
    let mut newest:Option<(SystemTime,PathBuf)>=None;
    let now=SystemTime::now();
    for item in items.flatten().take(512){
        let name=item.file_name();let Some(name)=name.to_str()else{continue};
        let Some(suffix)=name.strip_prefix(&prefix)else{continue};
        let Some((pid,stamp))=suffix.split_once('-')else{continue};
        if pid.parse::<u32>().ok().filter(|n|*n>0).is_none() ||
            stamp.parse::<u128>().ok().filter(|n|*n>0).is_none(){continue}
        let path=item.path();
        let log=path.join("update.log");
        if crate::fsops::ensure_plain_path(&log).is_err(){continue}
        let Ok(m)=fs::metadata(&log)else{continue};
        if !m.is_file()||m.len()>256{continue}
        let Ok(at)=m.modified()else{continue};
        if at<installed||!now.duration_since(at).is_ok_and(|d|d<=Duration::from_secs(600)){continue}
        let mut text=String::new();
        if fs::File::open(&log).and_then(|f|f.take(256).read_to_string(&mut text)).is_err(){continue}
        if !text.trim_start_matches('\u{feff}').trim().strip_prefix("Update installed: ")
            .and_then(|s|s.strip_suffix(" files")).is_some_and(|n|n.parse::<u32>().is_ok_and(|n|n>0)){continue}
        if newest.as_ref().is_none_or(|(old,_)|at>*old){newest=Some((at,path));}
    }
    let Some((_,session))=newest else{return false};
    let receipt=session.join(".ndr-about-shown");
    if crate::fsops::ensure_plain_path(&receipt).is_err(){return false}
    // create_new prevents duplicate popups if two instances race, and
    // never truncates a user file or follows a substituted hard link.
    fs::OpenOptions::new().write(true).create_new(true).open(receipt).is_ok()
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
    download_github_update_with_progress(release, get, |_| {})
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
    let cache = crate::cache::updates();
    crate::cache::prepare(&cache)?;
    let folder = cache.join(format!("ndr-update-{}-{}-{}", release.tag,
        std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?.as_nanos()));
    std::fs::create_dir(&folder).map_err(|e| e.to_string())?;
    let archive = folder.join("NDR-windows-x64.zip");
    if let Err(error) = std::fs::write(&archive, &response.body) {
        // This folder was created by this request and is not an installer
        // or recovery session yet. Do not accumulate partial downloads.
        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_dir(&folder);
        return Err(error.to_string());
    }
    if !cfg!(test) { crate::cache::schedule_maintenance(); }
    Ok(archive)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdatePhase { Downloading, Verifying, Preparing, Installing, Failed }

/// Translate NDR-owned update diagnostics at the UI boundary. Keep external
/// OS/network diagnostics verbatim for troubleshooting.
pub fn localized_failure(language: &crate::compat::Language, raw: &str) -> String {
    let key = match raw {
        "GitHub did not return a published stable release" | "Missing release tag" |
        "Invalid build tag" | "Release assets missing" | "Complete NDR ZIP missing" =>
            Some(("NDR.UPD_DETAIL_RELEASE_INVALID", "The release information is invalid or incomplete.")),
        "Unexpected release download URL" | "Unsafe release ZIP size" |
        "Release ZIP has no trustworthy SHA-256 digest" | "Update download was not secure or HTTP 200" =>
            Some(("NDR.UPD_DETAIL_UNTRUSTED", "The update package could not be verified as trustworthy.")),
        "Update ZIP size differs from GitHub release" =>
            Some(("NDR.UPD_DETAIL_SIZE", "The downloaded ZIP size does not match the release.")),
        "Update SHA-256 verification failed" =>
            Some(("NDR.UPD_DETAIL_HASH", "SHA-256 verification of the update failed.")),
        "Installation directory not found" | "Run the updater from a complete installed NDR folder" =>
            Some(("NDR.UPD_DETAIL_INSTALL_DIR", "Run the updater from a complete NDR installation.")),
        "Update staging folder missing" =>
            Some(("NDR.UPD_DETAIL_STAGING", "Could not prepare the update files.")),
        "The NexusFile update feed is malformed and has no valid version record." |
        "The NexusFile update feed contains unsupported extra records." =>
            Some(("NDR.UPD_DETAIL_FEED_INVALID", "The update feed is malformed.")),
        "Download failed: Timeout" =>
            Some(("NDRUpdate.RequestTimedOut", "The update server did not respond within 15 seconds.")),
        "Download failed: TooLarge" =>
            Some(("NDR.UPD_DETAIL_DOWNLOAD_LIMIT", "The update package exceeds the supported size limit.")),
        _ => None,
    };
    if let Some((key, fallback)) = key {
        return language.text(key, fallback);
    }
    if let Some(detail) = raw.strip_prefix("Could not launch updater: ") {
        return format!("{}\n{}", language.text("NDR.UPD_DETAIL_LAUNCH", "Could not start the installer."), detail);
    }
    if let Some(detail) = raw.strip_prefix("Download failed: ") {
        return format!("{}\n{}", language.text("NDRUpdate.RequestFailed", "Could not contact the update server. Check your connection and update feed."), detail);
    }
    raw.to_owned()
}

fn write_installer_script(path: &Path) -> std::io::Result<()> {
    // Windows PowerShell 5.1 treats BOM-less -File scripts as ANSI.
    std::fs::write(path, format!("\u{feff}{}", include_str!("apply-update.ps1")))
}

/// Resolve window text in Rust before leaving the selected app catalog. The
/// updater receives data only (UTF-8 JSON), never executable locale snippets.
fn installer_messages(language: &crate::compat::Language) -> std::collections::BTreeMap<&'static str, String> {
    [
        ("title", language.localized("NDR.UPD_INSTALL_TITLE", "NDR 업데이트", "NDR Update")),
        ("heading", language.localized("NDR.UPD_INSTALL_HEADING", "NDR 업데이트 설치", "Installing NDR update")),
        ("checking", language.localized("NDR.UPD_INSTALL_CHECKING", "설치 패키지를 확인하고 있습니다...", "Checking the installation package...")),
        ("extracting", language.localized("NDR.UPD_INSTALL_EXTRACTING", "설치 파일을 준비하고 있습니다...", "Preparing the installation files...")),
        ("closing", language.localized("NDR.UPD_INSTALL_CLOSING", "NDR 종료를 확인하고 있습니다...", "Checking whether NDR has closed...")),
        ("waiting", language.localized("NDR.UPD_INSTALL_WAITING", "NDR 종료를 기다리고 있습니다...", "Waiting for NDR to close...")),
        ("installing", language.localized("NDR.UPD_INSTALL_WRITING", "NDR 파일을 설치하고 있습니다...", "Installing the NDR files...")),
        ("finished", language.localized("NDR.UPD_INSTALL_FINISHED", "설치 완료. NDR을 다시 실행합니다...", "Installation complete. Restarting NDR...")),
        ("failed", language.localized("NDR.UPD_INSTALL_FAILURE", "업데이트 실패. 자세한 내용은 update.log를 확인하세요.", "The update failed. See update.log for details.")),
        ("failureTitle", language.localized("NDR.UPD_INSTALL_FAILURE_TITLE", "NDR 업데이트 오류", "NDR update error")),
    ].into_iter().collect()
}

/// The script is external to NDR so that Windows can replace NDR.exe after exit.
pub fn launch_update(archive: &Path, language: &crate::compat::Language) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let installation = executable.parent().ok_or("Installation directory not found")?;
    if !installation.join("Skins").is_dir() || !installation.join("Languages").is_dir() {
        return Err("Run the updater from a complete installed NDR folder".into());
    }
    let folder = archive.parent().ok_or("Update staging folder missing")?;
    crate::fsops::ensure_plain_path(archive)?;
    let script = folder.join("apply-update.ps1");
    crate::fsops::ensure_plain_path(&script)?;
    write_installer_script(&script).map_err(|e| e.to_string())?;
    let messages = folder.join("installer-locale.json");
    crate::fsops::ensure_plain_path(&messages)?;
    let locale = serde_json::to_vec(&installer_messages(language)).map_err(|e| e.to_string())?;
    std::fs::write(&messages, locale).map_err(|e| e.to_string())?;
    std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Sta", "-ExecutionPolicy", "Bypass", "-WindowStyle", "Hidden", "-File"])
        .arg(&script).arg("-Archive").arg(archive)
        .arg("-Destination").arg(installation)
        .arg("-ParentPid").arg(std::process::id().to_string())
        .arg("-LogPath").arg(folder.join("update.log"))
        .arg("-TextPath").arg(&messages)
        .arg("-ShowProgress")
        .arg("-ManagedCache")
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
    fn emitted_installer_has_bom_and_intact_korean() {
        let path = std::env::temp_dir().join(format!("ndr-update-utf8-test-{}.ps1", std::process::id()));
        write_installer_script(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(bytes.starts_with(&[0xef, 0xbb, 0xbf]));
        let content = std::str::from_utf8(&bytes[3..]).unwrap();
        assert_eq!(content, include_str!("apply-update.ps1"));
        assert!(content.contains("Get-Content -LiteralPath $path -Raw -Encoding UTF8"));
        assert!(content.contains("Import-InstallerText $TextPath"));
        assert!(content.contains("$script:installerText"));
    }

    #[test]
    fn installer_messages_follow_the_selected_catalog_as_unicode_data() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Resources/Languages"));
        for &(locale, _) in crate::compat::BUILTIN_LANGUAGES {
            let selected = crate::compat::Language::load(root, locale);
            let messages = installer_messages(&selected);
            let serialized = serde_json::to_vec(&messages).unwrap();
            if let Some(dir) = std::env::var_os("NDR_INSTALLER_QA_DIR") {
                let dir = PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(dir.join(format!("{locale}.json")), &serialized).unwrap();
            }
            let decoded: std::collections::BTreeMap<String, String> = serde_json::from_slice(&serialized).unwrap();
            let english = installer_messages(&crate::compat::Language::load(root, "en"));
            for (field, key) in [
                ("title", "TITLE"), ("heading", "HEADING"), ("checking", "CHECKING"),
                ("extracting", "EXTRACTING"), ("closing", "CLOSING"), ("waiting", "WAITING"),
                ("installing", "WRITING"), ("finished", "FINISHED"), ("failed", "FAILURE"),
                ("failureTitle", "FAILURE_TITLE"),
            ] {
                let key = format!("NDR.UPD_INSTALL_{key}");
                let primary = selected.primary_raw(&key).unwrap_or_else(|| panic!("{locale}: {key}"));
                assert_eq!(decoded[field], primary, "{locale}: {key}");
                assert!(!primary.trim().is_empty() && !primary.contains('\u{fffd}'), "{locale}: {key}");
                if locale != "en" {
                    assert_ne!(decoded[field], english[field], "{locale}: English fallback for {key}");
                }
            }
        }
    }

    #[test]
    fn updater_owned_failures_are_localized_in_every_language() {
        let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Resources/Languages"));
        let cases = [
            ("Missing release tag", "RELEASE_INVALID"),
            ("Unexpected release download URL", "UNTRUSTED"),
            ("Update ZIP size differs from GitHub release", "SIZE"),
            ("Update SHA-256 verification failed", "HASH"),
            ("Run the updater from a complete installed NDR folder", "INSTALL_DIR"),
            ("Update staging folder missing", "STAGING"),
            ("The NexusFile update feed is malformed and has no valid version record.", "FEED_INVALID"),
            ("Download failed: TooLarge", "DOWNLOAD_LIMIT"),
        ];
        for &(locale, _) in crate::compat::BUILTIN_LANGUAGES {
            let language = crate::compat::Language::load(root, locale);
            for (raw, suffix) in cases {
                let key = format!("NDR.UPD_DETAIL_{suffix}");
                let primary = language.primary_raw(&key).unwrap();
                assert_eq!(localized_failure(&language, raw), primary, "{locale}: {key}");
            }
            let launch = localized_failure(&language, "Could not launch updater: OS error 5");
            assert_eq!(launch, format!("{}\nOS error 5", language.primary_raw("NDR.UPD_DETAIL_LAUNCH").unwrap()));
            assert_eq!(localized_failure(&language, "OS error 112"), "OS error 112");
        }
    }

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

#[test]
fn completed_update_receipt_matches_build_success_and_is_consumed_once() {
    use std::fs;
    let root=std::env::temp_dir().join(format!("ndr-completed-update-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let updates=root.join("updates");fs::create_dir_all(&updates).unwrap();
    let exe=root.join("NDR.exe");fs::write(&exe,b"fixture").unwrap();
    let session=updates.join("ndr-update-build-123-42-1700000000");fs::create_dir(&session).unwrap();
    let log=session.join("update.log");fs::write(&log,b"Update failed: invalid package").unwrap();
    assert!(!consume_recent_update(&updates,123,&exe));
    fs::write(&log,b"\xef\xbb\xbfUpdate installed: 42 files\r\n").unwrap();
    assert!(!consume_recent_update(&updates,122,&exe));
    assert!(consume_recent_update(&updates,123,&exe));
    assert!(!consume_recent_update(&updates,123,&exe));
    assert!(session.join(".ndr-about-shown").is_file());
    fs::remove_dir_all(root).unwrap();
}
