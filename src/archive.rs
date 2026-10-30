//! Archive operations use bundled 7-Zip, validated entries and isolated extraction.
use crate::fsops::{
    ConflictChoice, check_cancel, ensure_plain_path, is_reparse, rename_new,
    unique_path,
};
use std::{
    collections::HashSet,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::AtomicBool,
    thread,
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub key: String,
    pub is_dir: bool,
    pub size: u64,
}
#[derive(Debug, Clone)]
pub struct ExternalProgram {
    pub program: PathBuf,
    pub create_template: Option<String>,
    pub extract_template: Option<String>,
}
pub fn create_configured(
    sources: &[PathBuf],
    output: &Path,
    options: &crate::commands::ArchiveOptions,
    cancel: &AtomicBool,
) -> Result<(), String> {
    create_with_program(sources, output, options, None, cancel)
}
fn artifact_name(name: &str, expected: &str) -> bool {
    let name = name.to_lowercase();
    let expected = expected.to_lowercase();
    if name == expected {
        return true;
    }
    if let Some(suffix) = name.strip_prefix(&(expected.clone() + ".")) {
        if !suffix.is_empty() && suffix.bytes().all(|b| b.is_ascii_digit()) {
            return true;
        }
    }
    let Some((stem, extension)) = expected.rsplit_once('.') else {
        return false;
    };
    if let Some(part) = name
        .strip_prefix(&(stem.to_owned() + ".part"))
        .and_then(|s| s.strip_suffix(&(".".to_owned() + extension)))
    {
        if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()) {
            return true;
        }
    }
    let prefix = if extension == "rar" {
        Some(".r")
    } else if extension == "arj" {
        Some(".a")
    } else {
        None
    };
    if let Some(prefix) = prefix {
        if let Some(number) = name.strip_prefix(&(stem.to_owned() + prefix)) {
            return number.len() == 2 && number.bytes().all(|b| b.is_ascii_digit());
        }
    }
    false
}
fn commit_artifacts(
    stage: &Path,
    parent: &Path,
    expected: &str,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let mut names = HashSet::new();
    for e in fs::read_dir(stage).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        names.insert(e.file_name().to_string_lossy().to_lowercase());
    }
    let backup = unique_path(parent, "archive-backup");
    fs::create_dir(&backup).map_err(|e| e.to_string())?;
    let mut commit = Commit {
        installed: Vec::new(),
        backups: Vec::new(),
        dirs: Vec::new(),
    };
    let result = (|| {
        for e in fs::read_dir(parent).map_err(|e| e.to_string())? {
            check_cancel(cancel)?;
            let e = e.map_err(|e| e.to_string())?;
            let name = e.file_name().to_string_lossy().into_owned();
            if artifact_name(&name, expected) && !names.contains(&name.to_lowercase()) {
                ensure_plain_path(&e.path())?;
                if !e.file_type().map_err(|e| e.to_string())?.is_file() {
                    return Err("Archive output conflicts with a directory".into());
                }
                let saved = unique_path(&backup, "old-volume");
                rename_new(&e.path(), &saved)?;
                commit.backups.push((e.path(), saved));
            }
        }
        commit.merge_resolved(stage, parent, &backup, cancel, &mut |_, _| {
            ConflictChoice::Overwrite
        })
    })();
    if let Err(error) = result {
        if let Err(rollback) = commit.rollback() {
            return Err(format!(
                "{error}; archive rollback incomplete: {rollback}; backup retained at {}",
                backup.display()
            ));
        }
        let _ = fs::remove_dir_all(&backup);
        return Err(error);
    }
    fs::remove_dir_all(&backup).map_err(|e| {
        format!(
            "Archive committed; previous archive artifacts retained at {}: {e}",
            backup.display()
        )
    })
}
pub fn create_with_program(
    sources: &[PathBuf],
    output: &Path,
    options: &crate::commands::ArchiveOptions,
    program: Option<&ExternalProgram>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    create_with_progress(sources, output, options, program, cancel, None)
}

/// create_with_program에 진행 알림(`progress`)을 더한 것: 동봉 7-Zip의 압축 단계(`a`)에 -bsp1을 주고 그 백분율·파일 이름을 알린다.
pub fn create_with_progress(
    sources: &[PathBuf],
    output: &Path,
    options: &crate::commands::ArchiveOptions,
    program: Option<&ExternalProgram>,
    cancel: &AtomicBool,
    progress: Option<ProgressSink>,
) -> Result<(), String> {
    check_cancel(cancel)?;
    if sources.is_empty() {
        return Err("No archive sources".into());
    }
    if options.volume_bytes == Some(0) {
        return Err("Archive volume size must be positive".into());
    }
    ensure_plain_path(output)?;
    let extension = options.extension.trim_start_matches('.').to_lowercase();
    // C# 내장 TAR.GZ / TAR.BZ2: TAR를 만든 뒤 GZip/BZip2로 감싼다.
    let wrapped = match extension.as_str() {
        "tar.gz" | "tgz" => Some("gzip"),
        "tar.bz2" | "tbz" | "tbz2" | "tar.bzip2" => Some("bzip2"),
        _ => None,
    };
    if program.is_none()
        && wrapped.is_none()
        && !["7z", "zip", "tar", "gz", "bz2", "xz", "wim", "alz", "egg"].contains(&extension.as_str())
    {
        return Err("This archive format requires an explicitly configured external program".into());
    }
    if program.is_none()
        && !options.password.is_empty()
        && !matches!(extension.as_str(), "7z" | "zip")
    {
        return Err("Selected archive format does not support encryption".into());
    }
    let expected = output
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Archive output must have a Unicode filename")?;
    validate_key(expected)?;
    let parent = output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure_plain_path(parent)?;
    let temporary = std::env::temp_dir();
    ensure_plain_path(&temporary)?;
    let snapshot = unique_path(&temporary, "archive-input");
    fs::create_dir(&snapshot).map_err(|e| e.to_string())?;
    let output_stage = unique_path(parent, "archive-output");
    let result = (|| {
        let mut names = HashSet::new();
        let mut source_paths = Vec::new();
        for source in sources {
            let canonical = fs::canonicalize(source).map_err(|e| e.to_string())?;
            if snapshot.starts_with(&canonical) {
                return Err("Archive staging directory lies inside a selected source".into());
            }
            let name = source
                .file_name()
                .ok_or("Cannot archive a filesystem root")?;
            let text = name
                .to_str()
                .ok_or("Archive source filename must be Unicode")?;
            validate_key(text)?;
            if !names.insert(text.to_lowercase()) {
                return Err("Duplicate archive source names".into());
            }
            let target = snapshot.join(name);
            copy_tree(source, &target, cancel)?;
            source_paths.push(target);
        }
        fs::create_dir(&output_stage).map_err(|e| e.to_string())?;
        let output_stage_abs = fs::canonicalize(&output_stage).map_err(|e| e.to_string())?;
        let archive = output_stage_abs.join(expected);
        // 더하기: 있는 압축 파일을 준비 폴더에 복사해 그 사본에 더한다(실패해도 원본은 그대로).
        if options.append && output.is_file() && wrapped.is_none() {
            fs::copy(output, &archive).map_err(|e| e.to_string())?;
            let mut permissions = fs::metadata(&archive).map_err(|e| e.to_string())?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            permissions.set_readonly(false);
            fs::set_permissions(&archive, permissions).map_err(|e| e.to_string())?;
        }
        let snapshot_abs = fs::canonicalize(&snapshot).map_err(|e| e.to_string())?;
        let source_paths: Vec<_> = source_paths
            .iter()
            .map(|p| snapshot_abs.join(p.file_name().unwrap_or_default()))
            .collect();
        let mut cmd = program_command(program)?;
        cmd.current_dir(&snapshot_abs);
        // 진행 알림은 동봉 7-Zip만(-bsp1: 백분율과 파일 이름을 표준 출력으로).
        let sink = progress.filter(|_| program.is_none());
        let bsp: &[&str] = if sink.is_some() { &["-bsp1"] } else { &[] };
        if let Some(program) = program {
            let args = crate::commands::build_archive_create_args(
                options,
                &source_paths,
                &archive,
                &snapshot_abs,
                program.create_template.as_deref(),
            )?;
            cmd.args(args);
        } else if let Some(outer) = wrapped {
            // 1) TAR(더하기면 있는 압축을 풀어 나온 TAR에), 2) 감싸기.
            let tar = snapshot_abs.join(".ndr-inner.tar");
            if options.append && output.is_file() {
                let unpack = snapshot_abs.join(".ndr-inner");
                fs::create_dir(&unpack).map_err(|e| e.to_string())?;
                let mut x = command()?;
                x.args(["e", "-y", "-sccUTF-8", "-snl-"]).arg(format!("-o{}", unpack.display())).arg("--").arg(fs::canonicalize(output).map_err(|e| e.to_string())?);
                execute(x, cancel)?;
                let inner = fs::read_dir(&unpack).map_err(|e| e.to_string())?.filter_map(Result::ok).map(|e| e.path()).find(|p| p.is_file()).ok_or("The existing archive holds no TAR")?;
                fs::rename(&inner, &tar).map_err(|e| e.to_string())?;
                let _ = fs::remove_dir_all(&unpack);
            }
            let mut t = command()?;
            t.current_dir(&snapshot_abs).args(["a", "-ttar", "-sccUTF-8", "-y", if options.recursive { "-r" } else { "-r-" }]).arg("--").arg(&tar);
            for path in &source_paths {
                t.arg(path.file_name().ok_or("Archive source name missing")?);
            }
            execute_capture(t, cancel, true)?;
            cmd.args(["a", &format!("-t{outer}"), "-sccUTF-8", "-y", &format!("-mx={}", options.level.clamp(0, 9))]).args(bsp).arg("--").arg(&archive).arg(&tar);
        } else {
            cmd.arg("a")
                .arg(format!("-t{extension}"))
                .args(["-sccUTF-8", "-y"]);
            if extension != "tar" {
                cmd.arg(format!("-mx={}", options.level.clamp(0, 9)));
            }
            if options.recursive {
                cmd.arg("-r");
            } else {
                cmd.arg("-r-");
            }
            if let Some(bytes) = options.volume_bytes {
                cmd.arg(format!("-v{bytes}b"));
            }
            if !options.password.is_empty() {
                password_arg(&mut cmd, Some(&options.password))?;
                if extension == "7z" {
                    cmd.arg("-mhe=on");
                } else {
                    cmd.arg("-mem=AES256");
                }
            }
            cmd.args(bsp).arg("--").arg(&archive);
            for path in &source_paths {
                cmd.arg(path.file_name().ok_or("Archive source name missing")?);
            }
        }
        execute_capture_progress(cmd, cancel, program.is_none(), sink)?;
        inspect_tree(&output_stage)?;
        let mut generated = Vec::new();
        for e in fs::read_dir(&output_stage).map_err(|e| e.to_string())? {
            let e = e.map_err(|e| e.to_string())?;
            let name = e.file_name().to_string_lossy().into_owned();
            let metadata = e.metadata().map_err(|e| e.to_string())?;
            if !metadata.is_file() || metadata.len() == 0 || !artifact_name(&name, expected) {
                return Err(format!(
                    "Archive program produced an unexpected artifact: {name}"
                ));
            }
            generated.push(e.path());
            if generated.len() > 100000 {
                return Err("Archive program produced too many volumes".into());
            }
        }
        if generated.is_empty() {
            return Err("Archive program produced no archive".into());
        }
        generated.sort();
        let first = generated
            .iter()
            .find(|p| p.file_name().is_some_and(|n| n == expected))
            .unwrap_or(&generated[0]);
        let password = if options.password.is_empty() {
            None
        } else {
            Some(options.password.as_str())
        };
        let _entries = list_cancel(first, password, cancel)?;
        let mut verify = command()?;
        verify.args(["t", "-sccUTF-8"]);
        password_arg(&mut verify, password)?;
        verify.arg("--").arg(first);
        execute(verify, cancel)?;
        check_cancel(cancel)?;
        commit_artifacts(&output_stage, parent, expected, cancel)
    })();
    let _ = fs::remove_dir_all(&snapshot);
    let _ = fs::remove_dir_all(&output_stage);
    result
}
/// Locate configured/bundled resources, or the Rust project's debug resources.
pub fn seven_zip_path() -> Result<PathBuf, String> {
    let mut roots = Vec::new();
    if let Some(root) = std::env::var_os("NDR_RESOURCE_DIR") {
        roots.push(PathBuf::from(root));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_owned());
            roots.push(parent.join("Resources"));
        }
    }
    #[cfg(debug_assertions)]
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Resources"));
    for root in roots {
        for relative in ["Tools/7zip/Full/7z.exe", "Tools/7zip/7za.exe"] {
            let candidate = root.join(relative);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("Bundled 7-Zip not found; set NDR_RESOURCE_DIR to the resource directory".into())
}
fn command() -> Result<Command, String> {
    program_command(None)
}
fn program_command(program: Option<&ExternalProgram>) -> Result<Command, String> {
    let executable = match program {
        Some(p) => fs::canonicalize(&p.program)
            .map_err(|e| format!("Cannot open configured archive program: {e}"))?,
        None => seven_zip_path()?,
    };
    if !executable.is_file() {
        return Err("Archive program is not an ordinary executable file".into());
    }
    if executable
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
    {
        return Err(
            "Archive programs must be executable binaries; select the executable directly".into(),
        );
    }
    let mut c = Command::new(executable);
    c.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000);
    }
    Ok(c)
}
fn execute(command: Command, cancel: &AtomicBool) -> Result<String, String> {
    execute_capture(command, cancel, true)
}
/// 압축 진행을 받는 곳: (백분율 0~100, 지금 파일 이름(7-Zip이 알린 상대 이름)).
pub type ProgressSink = std::sync::Arc<dyn Fn(u8, &str) + Send + Sync>;

/// 7-Zip `-bsp1` 진행 글 한 토막("  23% 4 + 이름" 또는 "  23%")에서 (백분율, 이름).
pub fn parse_seven_zip_progress(segment: &str) -> Option<(u8, String)> {
    let percent_at = segment.find('%')?;
    let digits: String = segment[..percent_at].chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
    let percent: u8 = digits.parse::<u32>().ok()?.min(100) as u8;
    let rest = &segment[percent_at + 1..];
    let name = rest.split_once(" + ").or_else(|| rest.split_once(" U ")).map(|(_, name)| name.trim().to_owned()).unwrap_or_default();
    Some((percent, name))
}

fn execute_capture(
    command: Command,
    cancel: &AtomicBool,
    strict_utf8: bool,
) -> Result<String, String> {
    execute_capture_progress(command, cancel, strict_utf8, None)
}

fn execute_capture_progress(
    mut command: Command,
    cancel: &AtomicBool,
    strict_utf8: bool,
    progress: Option<ProgressSink>,
) -> Result<String, String> {
    check_cancel(cancel)?;
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().ok_or("Missing stdout")?;
    let stderr = child.stderr.take().ok_or("Missing stderr")?;
    let read = |mut pipe: Box<dyn Read + Send>, progress: Option<ProgressSink>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut buffer = [0; 8192];
            let mut exceeded = false;
            // -bsp1 진행 글은 \r·\x08(지우기)·\n으로 덮어쓴다. 토막이 끝날 때마다 읽는다.
            let mut pending = Vec::new();
            loop {
                let n = pipe.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                if let Some(sink) = &progress {
                    for &byte in &buffer[..n] {
                        if matches!(byte, b'\r' | b'\n' | 0x08) {
                            if let Some((percent, name)) = std::str::from_utf8(&pending).ok().and_then(parse_seven_zip_progress) {
                                sink(percent, &name);
                            }
                            pending.clear();
                        } else if pending.len() < 4096 {
                            pending.push(byte);
                        }
                    }
                }
                if bytes.len() + n <= 64 * 1024 * 1024 {
                    bytes.extend_from_slice(&buffer[..n])
                } else {
                    exceeded = true;
                }
            }
            Ok::<_, std::io::Error>((bytes, exceeded))
        })
    };
    let out = read(Box::new(stdout), progress);
    let err = read(Box::new(stderr), None);
    let status = loop {
        if check_cancel(cancel).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            return Err("Cancelled".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = out.join();
                let _ = err.join();
                return Err(e.to_string());
            }
        }
    };
    let (out, large) = out
        .join()
        .map_err(|_| "Archive output reader failed")?
        .map_err(|e| e.to_string())?;
    let (err, large_err) = err
        .join()
        .map_err(|_| "Archive error reader failed")?
        .map_err(|e| e.to_string())?;
    if large || large_err {
        return Err("Archive tool output exceeded 64 MiB".into());
    }
    if !status.success() {
        let message = String::from_utf8_lossy(&err);
        // C# HasExternalArchivePasswordDiagnostic: 표준 출력의 "Enter password (will not be echoed):" 줄도 암호 진단이다.
        let prompt = String::from_utf8_lossy(&out)
            .lines()
            .any(|line| line.trim().eq_ignore_ascii_case(EXTERNAL_PASSWORD_PROMPT))
            .then(|| format!("\n{EXTERNAL_PASSWORD_PROMPT}"))
            .unwrap_or_default();
        return Err(format!(
            "7-Zip failed ({status}): {}{prompt}",
            message.chars().take(2000).collect::<String>()
        ));
    }
    if strict_utf8 {
        decode_tool_output(out)
    } else {
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}
/// 7-Zip 출력은 -sccUTF-8로 UTF-8이어야 한다. 단 동봉 7zip+α(UzuCore)는 머리글 "7zip+α … Copyright (c) …"를
/// -scc 적용 전에 OEM 코드 페이지로 찍으므로(Main.cpp ShowCopyrightAndHelp, α = CP949 A5 E1) 그 첫 줄만 손실 허용으로 읽는다.
fn decode_tool_output(out: Vec<u8>) -> Result<String, String> {
    let start = out.iter().take_while(|b| matches!(b, b'\r' | b'\n')).count();
    let end = out[start..].iter().position(|&b| b == b'\n').map_or(out.len(), |p| start + p + 1);
    let banner = &out[start..end];
    if banner.windows(b"Copyright (c)".len()).any(|w| w == b"Copyright (c)") {
        let rest = std::str::from_utf8(&out[end..]).map_err(|e| format!("Invalid UTF-8 from archive tool: {e}"))?;
        return Ok(format!("{}{}{rest}", String::from_utf8_lossy(&out[..start]), String::from_utf8_lossy(banner)));
    }
    String::from_utf8(out).map_err(|e| format!("Invalid UTF-8 from archive tool: {e}"))
}
fn password_arg(command: &mut Command, password: Option<&str>) -> Result<(), String> {
    if let Some(p) = password {
        if p.contains('\0') || p.contains('\n') || p.contains('\r') {
            return Err("Invalid archive password".into());
        }
        command.arg(format!("-p{p}"));
    } else {
        command.arg("-p");
    }
    Ok(())
}
fn validate_key(key: &str) -> Result<(), String> {
    if key.split(['/', '\\']).count() > 256
        || key.is_empty()
        || key.starts_with(['/', '\\'])
        || key.contains([':', '\0', '\r', '\n'])
    {
        return Err(format!("Unsafe archive entry: {key:?}"));
    }
    for component in key.split(['/', '\\']) {
        if component.is_empty()
            || component == "."
            || component == ".."
            || component.ends_with(['.', ' '])
            || component.chars().any(|c| c < ' ' || "<>\"|?*".contains(c))
        {
            return Err(format!("Unsafe archive entry: {key:?}"));
        }
        let stem = component
            .split('.')
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(format!("Reserved device entry: {key}"));
        }
    }
    Ok(())
}
fn parse_listing(text: &str) -> Result<Vec<ArchiveEntry>, String> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    for block in text.replace("\r\n", "\n").split("\n\n") {
        let mut key = None;
        let mut dir = false;
        let mut size = 0;
        for line in block.lines() {
            if let Some(v) = line.strip_prefix("Path = ") {
                key = Some(v.to_owned())
            } else if line == "Folder = +" {
                dir = true
            } else if let Some(v) = line.strip_prefix("Size = ") {
                size = v.parse().map_err(|_| "Invalid archive entry size")?
            } else if let Some(v) = line.strip_prefix("Attributes = ") {
                dir |= v.starts_with('D');
                if v.contains('L') || v.contains("Reparse") {
                    return Err("Archive reparse entries are rejected".into());
                }
            } else if ["Symbolic Link = ", "Hard Link = ", "Reparse = "]
                .iter()
                .any(|prefix| line.strip_prefix(prefix).is_some_and(|value| !value.trim().is_empty()))
            {
                // 7-Zip은 TAR 항목마다 빈 "Symbolic Link = " 줄을 쓴다. 값이 있는 링크만 막는다.
                return Err("Archive links are rejected".into());
            }
        }
        if let Some(mut key) = key {
            // 동봉 7zip+α의 ALZ·EGG 처리기는 폴더를 "폴더\"처럼 끝 구분자를 붙여 보고한다(다른 형식은 "폴더"). 폴더 항목만 뗀다.
            if dir {
                key.truncate(key.trim_end_matches(['/', '\\']).len());
            }
            validate_key(&key)?;
            if !seen.insert(key.replace('\\', "/").to_lowercase()) {
                return Err("Duplicate archive entry names are rejected".into());
            }
            entries.push(ArchiveEntry {
                key,
                is_dir: dir,
                size,
            })
        }
    }
    Ok(entries)
}
pub fn list(archive: &Path, password: Option<&str>) -> Result<Vec<ArchiveEntry>, String> {
    list_cancel(archive, password, &AtomicBool::new(false))
}
fn list_cancel(
    archive: &Path,
    password: Option<&str>,
    cancel: &AtomicBool,
) -> Result<Vec<ArchiveEntry>, String> {
    list_details(archive, password, cancel).map(|(entries, _)| entries)
}
fn list_details(
    archive: &Path,
    password: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(Vec<ArchiveEntry>, HashSet<String>), String> {
    ensure_plain_path(archive)?;
    let archive = fs::canonicalize(archive).map_err(|e| e.to_string())?;
    let mut cmd = command()?;
    cmd.args(["l", "-slt", "-ba", "-sccUTF-8"]);
    password_arg(&mut cmd, password)?;
    cmd.arg("--").arg(archive);
    let text = execute(cmd, cancel)?;
    let entries = parse_listing(&text)?;
    let mut encrypted = HashSet::new();
    for block in text.replace("\r\n", "\n").split("\n\n") {
        if block.lines().any(|l| l == "Encrypted = +") {
            if let Some(key) = block.lines().find_map(|l| l.strip_prefix("Path = ")) {
                encrypted.insert(key.to_owned());
            }
        }
    }
    Ok((entries, encrypted))
}
fn inspect_tree(path: &Path) -> Result<(), String> {
    let mut pending = vec![path.to_owned()];
    while let Some(path) = pending.pop() {
        ensure_plain_path(&path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if is_reparse(&metadata) {
            return Err("Reparse point rejected".into());
        }
        if metadata.is_dir() {
            for e in fs::read_dir(&path).map_err(|e| e.to_string())? {
                pending.push(e.map_err(|e| e.to_string())?.path());
            }
        } else if !metadata.is_file() {
            return Err("Non-file archive entry rejected".into());
        }
    }
    Ok(())
}
fn copy_tree(source: &Path, target: &Path, cancel: &AtomicBool) -> Result<(), String> {
    use std::io::Write;
    let mut pending = vec![(source.to_owned(), target.to_owned())];
    let mut directories = Vec::new();
    while let Some((source, target)) = pending.pop() {
        check_cancel(cancel)?;
        ensure_plain_path(&source)?;
        let metadata = fs::symlink_metadata(&source).map_err(|e| e.to_string())?;
        if is_reparse(&metadata) {
            return Err("Archive source links are rejected".into());
        }
        if metadata.is_dir() {
            fs::create_dir(&target).map_err(|e| e.to_string())?;
            for e in fs::read_dir(&source).map_err(|e| e.to_string())? {
                let e = e.map_err(|e| e.to_string())?;
                pending.push((e.path(), target.join(e.file_name())));
            }
            directories.push((target, metadata));
        } else if metadata.is_file() {
            let mut input = crate::fsops::open_read(&source)?;
            let mut output = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .map_err(|e| e.to_string())?;
            let mut buffer = vec![0; 1024 * 1024];
            let mut copied = 0;
            loop {
                check_cancel(cancel)?;
                let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
                copied += n as u64;
            }
            let current = input.metadata().map_err(|e| e.to_string())?;
            if copied != metadata.len() || current.modified().ok() != metadata.modified().ok() {
                return Err("Archive source changed while taking the snapshot".into());
            }
            crate::fsops::preserve_times(&output, &metadata)?;
            output
                .set_permissions(metadata.permissions())
                .map_err(|e| e.to_string())?;
        } else {
            return Err("Only ordinary archive sources are supported".into());
        }
    }
    for (target, metadata) in directories.into_iter().rev() {
        check_cancel(cancel)?;
        let mut options = fs::OpenOptions::new();
        options.write(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.custom_flags(0x02000000);
        }
        let directory = options.open(&target).map_err(|e| e.to_string())?;
        crate::fsops::preserve_times(&directory, &metadata)?;
        directory
            .set_permissions(metadata.permissions())
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
struct Commit {
    installed: Vec<PathBuf>,
    backups: Vec<(PathBuf, PathBuf)>,
    dirs: Vec<PathBuf>,
}
impl Commit {
    #[cfg(test)]
    fn merge(
        &mut self,
        stage: &Path,
        dest: &Path,
        backup: &Path,
        cancel: &AtomicBool,
    ) -> Result<(), String> {
        self.merge_resolved(stage, dest, backup, cancel, &mut |_, _| {
            ConflictChoice::Overwrite
        })
    }
    fn merge_resolved(
        &mut self,
        stage: &Path,
        dest: &Path,
        backup: &Path,
        cancel: &AtomicBool,
        resolver: &mut dyn FnMut(&Path, &Path) -> ConflictChoice,
    ) -> Result<(), String> {
        self.merge_entry(stage, dest, backup, cancel, resolver, false)
    }
    fn merge_entry(
        &mut self,
        stage: &Path,
        dest: &Path,
        backup: &Path,
        cancel: &AtomicBool,
        resolver: &mut dyn FnMut(&Path, &Path) -> ConflictChoice,
        prompt_directory: bool,
    ) -> Result<(), String> {
        check_cancel(cancel)?;
        ensure_plain_path(dest)?;
        if stage.is_dir() {
            if dest.exists() {
                if !dest.is_dir() {
                    return Err(format!("Directory/file collision: {}", dest.display()));
                }
                if prompt_directory {
                    match resolver(stage, dest) {
                        ConflictChoice::Skip => return Ok(()),
                        // C# IsArchiveEntryNewer: 항목이 더 새로울 때만 합친다.
                        ConflictChoice::OverwriteNewer if !entry_newer(stage, dest) => return Ok(()),
                        ConflictChoice::Cancel => {
                            return Err("Archive extraction cancelled at a folder conflict".into());
                        }
                        ConflictChoice::Rename => {
                            let candidate = renamed_destination(dest)?;
                            rename_new(stage, &candidate)?;
                            self.installed.push(candidate);
                            return Ok(());
                        }
                        ConflictChoice::ReplaceFolder => {
                            let saved = unique_path(backup, "folder");
                            rename_new(dest, &saved)?;
                            self.backups.push((dest.to_owned(), saved));
                            check_cancel(cancel)?;
                            rename_new(stage, dest)?;
                            self.installed.push(dest.to_owned());
                            return Ok(());
                        }
                        ConflictChoice::Overwrite | ConflictChoice::OverwriteNewer | ConflictChoice::Resume => {}
                    }
                }
            } else {
                fs::create_dir(dest).map_err(|e| e.to_string())?;
                self.dirs.push(dest.to_owned());
            }
            for e in fs::read_dir(stage).map_err(|e| e.to_string())? {
                let e = e.map_err(|e| e.to_string())?;
                self.merge_entry(
                    &e.path(),
                    &dest.join(e.file_name()),
                    backup,
                    cancel,
                    resolver,
                    true,
                )?;
            }
        } else {
            let mut dest = dest.to_owned();
            if dest.exists() {
                if dest.is_dir() {
                    return Err(format!("File/directory collision: {}", dest.display()));
                }
                match resolver(stage, &dest) {
                    ConflictChoice::Skip => return Ok(()),
                    ConflictChoice::OverwriteNewer if !entry_newer(stage, &dest) => return Ok(()),
                    ConflictChoice::Cancel => {
                        return Err(
                            "Archive extraction cancelled at an existing-file conflict".into()
                        );
                    }
                    ConflictChoice::Rename => {
                        let name = dest
                            .file_name()
                            .ok_or("Invalid archive destination")?
                            .to_string_lossy()
                            .into_owned();
                        let parent = dest.parent().ok_or("Invalid archive destination parent")?;
                        let mut index = 1;
                        loop {
                            let candidate = parent.join(format!("{name} ({index})"));
                            if fs::symlink_metadata(&candidate).is_err() {
                                dest = candidate;
                                break;
                            }
                            index += 1;
                        }
                    }
                    ConflictChoice::Overwrite | ConflictChoice::OverwriteNewer | ConflictChoice::Resume | ConflictChoice::ReplaceFolder => {
                        let saved = unique_path(backup, "old");
                        rename_new(&dest, &saved)?;
                        self.backups.push((dest.to_owned(), saved));
                    }
                }
            }
            check_cancel(cancel)?;
            rename_new(stage, &dest)?;
            self.installed.push(dest.to_owned());
        }
        Ok(())
    }
}
/// C# IsArchiveEntryNewer: 풀어 둔 항목(원본 시간 유지)이 대상보다 새로운가. 시간을 모르면 아니다.
fn entry_newer(stage: &Path, dest: &Path) -> bool {
    match (fs::metadata(stage).and_then(|m| m.modified()), fs::metadata(dest).and_then(|m| m.modified())) {
        (Ok(entry), Ok(target)) => entry > target,
        _ => false,
    }
}
impl Commit {
    fn rollback(&self) -> Result<(), String> {
        let mut errors = Vec::new();
        for p in self.installed.iter().rev() {
            let result = if p.is_dir() {
                fs::remove_dir_all(p)
            } else {
                fs::remove_file(p)
            };
            if let Err(e) = result {
                errors.push(e.to_string())
            }
        }
        for (p, saved) in self.backups.iter().rev() {
            if let Err(e) = fs::rename(saved, p) {
                errors.push(format!(
                    "{} retained at {}: {e}",
                    p.display(),
                    saved.display()
                ))
            }
        }
        for p in self.dirs.iter().rev() {
            if let Err(e) = fs::remove_dir(p) {
                errors.push(e.to_string())
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
fn renamed_destination(dest: &Path) -> Result<PathBuf, String> {
    let name = dest
        .file_name()
        .ok_or("Invalid archive target name")?
        .to_string_lossy();
    let parent = dest.parent().ok_or("Invalid archive target parent")?;
    for index in 1..u64::MAX {
        let candidate = parent.join(format!("{name} ({index})"));
        if fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    Err("No unused archive destination name".into())
}
fn commit_aggregate(
    stage: &Path,
    destination: &Path,
    cancel: &AtomicBool,
    resolver: &mut dyn FnMut(&Path, &Path) -> ConflictChoice,
) -> Result<(), String> {
    inspect_tree(stage)?;
    ensure_plain_path(destination)?;
    check_cancel(cancel)?;
    if !destination.exists() {
        return rename_new(stage, destination);
    }
    inspect_tree(destination)?;
    if !destination.is_dir() {
        return Err("Extraction destination is not a directory".into());
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let backup = unique_path(parent, "backup");
    fs::create_dir(&backup).map_err(|e| e.to_string())?;
    let mut commit = Commit {
        installed: Vec::new(),
        backups: Vec::new(),
        dirs: Vec::new(),
    };
    if let Err(error) = commit.merge_resolved(stage, destination, &backup, cancel, resolver) {
        if let Err(rollback) = commit.rollback() {
            return Err(format!(
                "{error}; rollback incomplete: {rollback}; backup retained: {}",
                backup.display()
            ));
        }
        let _ = fs::remove_dir_all(&backup);
        return Err(error);
    }
    fs::remove_dir_all(&backup).map_err(|e| {
        format!(
            "Extraction committed; backup retained at {}: {e}",
            backup.display()
        )
    })
}
pub fn extract(
    archive: &Path,
    destination: &Path,
    password: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    extract_with_conflicts(archive, destination, password, cancel, &mut |_, _| {
        ConflictChoice::Cancel
    })
}
/// Resolve each existing-file collision on the worker thread. Callers can bridge
/// this callback to the same UI reply channel used by filesystem operations.
pub fn extract_with_conflicts(
    archive: &Path,
    destination: &Path,
    password: Option<&str>,
    cancel: &AtomicBool,
    resolver: &mut dyn FnMut(&Path, &Path) -> ConflictChoice,
) -> Result<(), String> {
    check_cancel(cancel)?;
    ensure_plain_path(destination)?;
    let entries = list_cancel(archive, password, cancel)?;
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure_plain_path(parent)?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stage = unique_path(parent, "extract");
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let archive = fs::canonicalize(archive).map_err(|e| e.to_string())?;
    let stage_absolute = fs::canonicalize(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        let mut cmd = command()?;
        cmd.args(["x", "-y", "-sccUTF-8", "-snl-"])
            .arg(format!("-o{}", stage_absolute.display()));
        password_arg(&mut cmd, password)?;
        cmd.arg("--").arg(archive);
        execute(cmd, cancel)?;
        inspect_tree(&stage)?;
        for entry in &entries {
            let target = stage.join(entry.key.replace('\\', "/"));
            ensure_plain_path(&target)?;
            let m = fs::symlink_metadata(&target)
                .map_err(|e| format!("Missing extracted entry {}: {e}", entry.key))?;
            if m.is_dir() != entry.is_dir || (!entry.is_dir && m.len() != entry.size) {
                return Err(format!("Extracted entry mismatch: {}", entry.key));
            }
        }
        check_cancel(cancel)?;
        if !destination.exists() {
            rename_new(&stage, destination)?;
            return Ok(());
        }
        inspect_tree(destination)?;
        if !destination.is_dir() {
            return Err("Extraction destination is not a directory".into());
        }
        let backup = unique_path(parent, "backup");
        fs::create_dir(&backup).map_err(|e| e.to_string())?;
        let mut commit = Commit {
            installed: Vec::new(),
            backups: Vec::new(),
            dirs: Vec::new(),
        };
        if let Err(error) = commit.merge_resolved(&stage, destination, &backup, cancel, resolver) {
            if let Err(rollback) = commit.rollback() {
                return Err(format!(
                    "{error}; rollback incomplete: {rollback}; backup retained: {}",
                    backup.display()
                ));
            }
            let _ = fs::remove_dir_all(&backup);
            return Err(error);
        }
        fs::remove_dir_all(&backup).map_err(|e| {
            format!(
                "Extraction committed; backup cleanup failed at {}: {e}",
                backup.display()
            )
        })?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&stage);
    result
}
/// C# FileSystemService.ArchiveExtensions.
const ARCHIVE_EXTENSIONS: &[&str] = &[
    ".zip", ".zipx", ".7z", ".rar", ".arj", ".ace", ".alz", ".egg", ".iso", ".cab", ".tar", ".gz", ".gzip", ".bz2", ".bzip2", ".z", ".xz", ".lzma", ".lzma86", ".zst", ".zstd",
    ".tar.gz", ".tar.bz2", ".tar.bzip2", ".tar.xz", ".tar.lz", ".tar.zst", ".tgz", ".tpz", ".tbz", ".tbz2", ".txz", ".tlz", ".tzst", ".taz",
    ".apfs", ".apm", ".ar", ".deb", ".udeb", ".lib", ".cpio", ".cramfs", ".dmg", ".ext", ".ext2", ".ext3", ".ext4",
    ".fat", ".gpt", ".hfs", ".hfsx", ".ihex", ".lzh", ".lha", ".mbr", ".ntfs", ".qcow", ".qcow2", ".qcow2c",
    ".rpm", ".squashfs", ".udf", ".uefi", ".scap", ".vdi", ".vhd", ".vhdx", ".avhdx", ".vmdk", ".xar", ".xip",
    ".wim", ".swm", ".esd", ".ppkg",
];

/// C# FileSystemService.GetArchiveFormat(이름): 긴 확장자부터, 숫자 3자리 이상 분할 꼬리 허용, .partN.rar, .rNN, .zNN, .aNN(N), .001.
pub fn archive_format(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let lower = name.to_lowercase();
    let numeric_start = lower.rfind('.');
    let numeric = numeric_start.is_some_and(|i| lower[i + 1..].len() >= 3 && lower[i + 1..].chars().all(|c| c.is_ascii_digit()));
    let mut extensions: Vec<&str> = ARCHIVE_EXTENSIONS.to_vec();
    extensions.sort_by_key(|e| std::cmp::Reverse(e.len()));
    for extension in extensions {
        if lower.ends_with(extension) {
            return Some(extension[1..].to_owned());
        }
        if numeric && lower[..numeric_start.unwrap_or(0)].ends_with(extension) {
            return Some(extension[1..].to_owned());
        }
    }
    if let Some(part) = lower.rfind(".part") {
        if lower.ends_with(".rar") && part + 5 <= lower.len() - 4 && lower[part + 5..lower.len() - 4].chars().all(|c| c.is_ascii_digit()) {
            return Some("rar".into());
        }
    }
    let extension = Path::new(&lower).extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let chars: Vec<char> = extension.chars().collect();
    if chars.len() == 4 && chars[1] == 'r' && chars[2..].iter().all(|c| c.is_ascii_digit()) {
        return Some("rar".into());
    }
    if chars.len() == 4 && chars[1] == 'z' && chars[2..].iter().all(|c| c.is_ascii_digit()) {
        return Some("zip".into());
    }
    if let Some(dot) = lower.rfind(".a") {
        let digits = &lower[dot + 2..];
        if (2..=3).contains(&digits.len()) && digits.chars().all(|c| c.is_ascii_digit()) {
            return Some("arj".into());
        }
    }
    if extension == ".001" {
        let base = Path::new(&name[..name.len() - 4]).extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))?;
        if ARCHIVE_EXTENSIONS.contains(&base.as_str()) {
            return Some(base[1..].to_owned());
        }
    }
    None
}
/// C# BuiltInArchiveExtensions(+ 분할 조각 규칙): NDR이 직접 읽는 형식. 아니면 외부 프로그램 쪽.
/// ALZ·EGG는 동봉 7-Zip(UzuCore 7zip+α)이 읽으므로 여기에 더한다. ACE는 C# 판(SharpCompress 순차 읽기)은 읽지만 동봉 7-Zip은
/// 형식 목록에 없어 못 읽으므로 뺀다(외부 압축 프로그램으로 통째로 푼다, `extract_whole_with_program`).
pub fn can_read_internally(path: &Path) -> bool {
    const BUILT_IN: &[&str] = &["zip", "7z", "rar", "arj", "alz", "egg", "tar", "gz", "bz2", "bzip2", "z", "tar.gz", "tar.bz2", "tar.bzip2", "tar.xz", "tar.lz", "tar.zst", "tgz", "tbz", "tbz2", "txz", "tlz", "tzst"];
    let Some(format) = archive_format(path) else { return false };
    if !BUILT_IN.contains(&format.as_str()) {
        return false;
    }
    let lower = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let ext = Path::new(&lower).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    if ext.len() == 3 && (ext.starts_with('r') || ext.starts_with('z')) && ext[1..].chars().all(|c| c.is_ascii_digit()) {
        return false;
    }
    if let Some(part) = lower.strip_suffix(".rar").and_then(|s| s.rsplit_once(".part")).map(|(_, n)| n) {
        if !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()) && part.trim_start_matches('0') != "1" {
            return false;
        }
    }
    true
}

/// C# CanReadArchiveInternally || GetExternalArchiveListingProgram(동봉 7-Zip): NDR이 그 안으로 들어가 목록을 볼 수 있는 압축 형식인가.
/// 동봉 7-Zip은 ACE만 못 읽는다.
pub fn bundled_can_browse(path: &Path) -> bool {
    archive_format(path).is_some_and(|format| format != "ace")
}

/// C# GetExtractFolderName: PathRemoveExtensionW(파일 이름) — 마지막 확장자만 뗀다(sample.7z.001 → sample.7z).
pub fn extract_folder_name(archive: &Path) -> String {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match name.rfind('.') {
        Some(dot) if !name[dot..].contains(' ') => name[..dot].to_owned(),
        _ => name,
    }
}

/// C# CreateUniqueExtractFolder(FUN_00438cb0): "{parent}\{name}", 있으면 "{…}(2)", "(3)"… CreateDirectoryW로 새로 만든 것만.
pub fn create_unique_extract_folder(parent: &Path, name: &str) -> Result<PathBuf, String> {
    let text = parent.to_string_lossy();
    let separator = if text.ends_with(['\\', '/']) { "" } else { "\\" };
    let base = format!("{text}{separator}{name}");
    for number in 1..=100_000u32 {
        let candidate = if number == 1 { base.clone() } else { format!("{base}({number})") };
        match fs::create_dir(&candidate) {
            Ok(()) => return Ok(PathBuf::from(candidate)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("압축 풀기 폴더를 만들 수 없습니다: {candidate}: {error}")),
        }
    }
    Err(format!("압축 풀기 폴더 이름을 정할 수 없습니다: {base}"))
}

/// C# GetArchiveBaseName(묶음 이름).
pub fn base_name(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    let mut extensions: Vec<&str> = ARCHIVE_EXTENSIONS.to_vec();
    extensions.sort_by_key(|e| std::cmp::Reverse(e.len()));
    if let Some(dot) = lower.rfind('.') {
        let tail = &lower[dot + 1..];
        if tail.len() >= 3 && tail.chars().all(|c| c.is_ascii_digit()) {
            for extension in &extensions {
                if lower[..dot].ends_with(extension) {
                    return name[..dot - extension.len()].to_owned();
                }
            }
        }
    }
    for composite in [".tar.bz2", ".tar.bzip2", ".tar.gz", ".tar.xz", ".tar.lz", ".tar.zst"] {
        if lower.ends_with(composite) {
            return name[..name.len() - composite.len()].to_owned();
        }
    }
    if let Some(part) = lower.rfind(".part") {
        if lower.ends_with(".rar") {
            return name[..part].to_owned();
        }
    }
    let ext = Path::new(&lower).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    if ext.len() == 3 && (ext.starts_with('r') || ext.starts_with('z')) && ext[1..].chars().all(|c| c.is_ascii_digit()) {
        return name[..name.len() - 4].to_owned();
    }
    Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name)
}

/// C# GetArchiveOutputFamilyFiles: 같은 폴더에서 형식과 묶음 이름이 같은 파일들(이름 순).
pub fn output_family(path: &Path) -> Vec<PathBuf> {
    let Some(format) = archive_format(path) else { return vec![] };
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let Some(directory) = full.parent().filter(|d| d.is_dir()) else { return vec![] };
    let base = base_name(&full).to_lowercase();
    let mut files: Vec<PathBuf> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && archive_format(p).is_some_and(|f| f.eq_ignore_ascii_case(&format)) && base_name(p).to_lowercase() == base)
        .collect();
    files.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    files
}

/// C# GetArchiveFirstVolumePath: .partN.rar → .part1.rar, .rNN → .rar, .zNN → .zip, .aNN → .arj, .NNN → .00…1(있을 때만).
pub fn first_volume_path(path: &Path) -> PathBuf {
    let full = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let name = full.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let lower = name.to_lowercase();
    let digits_after = |s: &str, prefix: char| s.len() >= 3 && s.starts_with(prefix) && s[1..].chars().all(|c| c.is_ascii_digit());
    let ext = Path::new(&lower).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let first = if let Some((stem, part)) = lower.strip_suffix(".rar").and_then(|s| s.rsplit_once(".part")).filter(|(_, n)| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
        let _ = part;
        Some(format!("{}.part1.rar", &name[..stem.len()]))
    } else if ext.len() == 3 && digits_after(&ext, 'r') {
        Some(format!("{}.rar", &name[..name.len() - 4]))
    } else if ext.len() == 3 && digits_after(&ext, 'z') {
        Some(format!("{}.zip", &name[..name.len() - 4]))
    } else if (3..=4).contains(&ext.len()) && digits_after(&ext, 'a') {
        Some(format!("{}.arj", &name[..name.len() - ext.len() - 1]))
    } else if ext.len() >= 3 && ext.chars().all(|c| c.is_ascii_digit()) {
        Some(format!("{}.{}1", &name[..name.len() - ext.len() - 1], "0".repeat(ext.len() - 1)))
    } else {
        None
    };
    match first {
        Some(first) => {
            let candidate = full.with_file_name(first);
            if candidate.is_file() { candidate } else { full }
        }
        None => full,
    }
}

/// 목록(머리글 암호가 필요하면 passwords로 묻는다). 고른 암호도 돌려준다.
pub fn list_with_passwords(archive: &Path, initial: Option<&str>, cancel: &AtomicBool, passwords: &mut dyn FnMut(&Path, Option<&str>) -> Option<String>) -> Result<(Vec<ArchiveEntry>, Option<String>), String> {
    let mut password = initial.map(str::to_owned);
    loop {
        match list_cancel(archive, password.as_deref(), cancel) {
            Ok(entries) => return Ok((entries, password)),
            Err(error) if password_failure(&error) => match passwords(archive, None) {
                Some(value) if !value.is_empty() => password = Some(value),
                _ => return Err("Archive password request cancelled".into()),
            },
            Err(error) => return Err(error),
        }
    }
}

/// C# GetAutoExtractLayoutAsync의 판단: 맨 위 항목이 하나이고 그것이 폴더인가.
pub fn has_single_root_folder(entries: &[ArchiveEntry]) -> bool {
    let mut roots: Vec<(String, bool)> = vec![];
    for entry in entries {
        let key = entry.key.replace('\\', "/");
        let key = key.trim_matches('/');
        if key.is_empty() {
            continue;
        }
        let (root, nested) = match key.split_once('/') {
            Some((root, _)) => (root.to_lowercase(), true),
            None => (key.to_lowercase(), entry.is_dir),
        };
        match roots.iter_mut().find(|(name, _)| *name == root) {
            Some(known) => known.1 |= nested,
            None => roots.push((root, nested)),
        }
    }
    roots.len() == 1 && roots[0].1
}

/// C# GetArchiveSummary: (파일 수, 폴더 수(암시된 폴더 포함), 풀린 크기).
pub fn summary(entries: &[ArchiveEntry]) -> (usize, usize, u64) {
    let mut folders = HashSet::new();
    let mut files = 0;
    let mut bytes = 0u64;
    for entry in entries {
        let key = entry.key.replace('\\', "/");
        let key = key.trim_matches('/').to_lowercase();
        let parts: Vec<&str> = key.split('/').collect();
        for depth in 1..parts.len() {
            folders.insert(parts[..depth].join("/"));
        }
        if entry.is_dir {
            folders.insert(key.clone());
        } else {
            files += 1;
            bytes += entry.size;
        }
    }
    (files, folders.len(), bytes)
}
/// C# ExtractEntryToTempAsync: %TEMP%\NDR\<새 이름>에 항목 하나만 풀고(와일드카드 없이) 그 파일 경로를 돌려준다.
pub fn extract_entry_to_temp(archive: &Path, password: Option<&str>, key: &str, cancel: &AtomicBool) -> Result<PathBuf, String> {
    check_cancel(cancel)?;
    ensure_plain_path(archive)?;
    if key.is_empty() || key.contains('\0') || key.split(['/', '\\']).any(|part| part == "..") {
        return Err(format!("Unsafe archive entry: {key}"));
    }
    let root = temp_entry_root();
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let folder = unique_path(&root, "entry");
    fs::create_dir(&folder).map_err(|e| e.to_string())?;
    // 고친 것: 풀기에 실패하면(암호 오류·취소 등) 만든 폴더를 지운다. 예전에는 실패한 시도마다 폴더가 남았다.
    let result = (|| {
        let archive = fs::canonicalize(archive).map_err(|e| e.to_string())?;
        let mut cmd = command()?;
        cmd.args(["x", "-y", "-sccUTF-8", "-snl-", "-spd"]).arg(format!("-o{}", folder.display()));
        password_arg(&mut cmd, password)?;
        cmd.arg("--").arg(archive).arg(key);
        execute(cmd, cancel)?;
        inspect_tree(&folder)?;
        let extracted = folder.join(key.replace('/', "\\"));
        if !extracted.is_file() {
            return Err(format!("The selected archive file could not be extracted: {}", extracted.display()));
        }
        Ok(extracted)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&folder);
    }
    result
}

/// 압축 안 항목을 하나씩 푸는 임시 폴더(%TEMP%\NDR).
pub fn temp_entry_root() -> PathBuf {
    std::env::temp_dir().join("NDR")
}

/// 고친 것(C# 판·예전 Rust 판은 지우지 않아 쌓였다): `root` 아래 ".ndr-entry-*" 폴더 가운데 안의 모든 항목이
/// `max_age`보다 오래된 것을 지운다. 먼저 ".ndr-trash-*"로 이름을 바꿔 보고(열린 파일이 있으면 실패하므로 건너뜀),
/// 바뀐 것만 지운다. 지난번에 지우다 만 ".ndr-trash-*"도 지운다. 지운 폴더 수를 돌려준다.
pub fn clean_stale_temp_entries(root: &Path, max_age: std::time::Duration, now: std::time::SystemTime) -> usize {
    fn newest(path: &Path) -> Option<std::time::SystemTime> {
        let meta = fs::symlink_metadata(path).ok()?;
        let mut latest = meta.modified().ok()?;
        if meta.is_dir() {
            for entry in fs::read_dir(path).ok()? {
                let time = newest(&entry.ok()?.path())?;
                latest = latest.max(time);
            }
        }
        Some(latest)
    }
    let Ok(items) = fs::read_dir(root) else { return 0 };
    let mut removed = 0;
    for item in items.flatten() {
        let name = item.file_name().to_string_lossy().into_owned();
        let path = item.path();
        if !item.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        if name.starts_with(".ndr-trash-") {
            if fs::remove_dir_all(&path).is_ok() {
                removed += 1;
            }
            continue;
        }
        if !name.starts_with(".ndr-entry-") {
            continue;
        }
        // 읽지 못한 항목이 있으면 나이를 모르므로 남긴다.
        let Some(latest) = newest(&path) else { continue };
        if now.duration_since(latest).map_or(true, |age| age < max_age) {
            continue;
        }
        let trash = root.join(format!(".ndr-trash-{}", &name[".ndr-entry-".len()..]));
        if fs::rename(&path, &trash).is_ok() && fs::remove_dir_all(&trash).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// C# ExtractExternalWithPasswordPromptAsync 모양: 암호 오류면 `passwords`로 다시 묻고(취소면 None) 다시 푼다.
/// 돌려주는 값은 (풀린 경로, 실제로 쓴 암호).
pub fn extract_entry_to_temp_with_passwords(
    archive: &Path,
    password: Option<&str>,
    key: &str,
    cancel: &AtomicBool,
    passwords: &mut dyn FnMut(&Path) -> Option<String>,
) -> Result<(PathBuf, Option<String>), String> {
    let mut current = password.map(str::to_owned);
    loop {
        match extract_entry_to_temp(archive, current.as_deref(), key, cancel) {
            Ok(path) => return Ok((path, current)),
            Err(error) if password_failure(&error) => match passwords(archive) {
                Some(value) => current = Some(value),
                None => return Err("Archive password request cancelled".into()),
            },
            Err(error) => return Err(error),
        }
    }
}pub fn create(
    sources: &[PathBuf],
    output: &Path,
    password: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let extension = output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let options = crate::commands::ArchiveOptions {
        extension,
        level: 5,
        password: password.unwrap_or("").into(),
        volume_bytes: None,
        recursive: true,
        append: false,
    };
    create_configured(sources, output, &options, cancel)
}
/// Password-on-demand extraction. Successful entries are aggregated in an isolated
/// tree and committed once, including the completed subset after password cancel.
pub fn extract_interactive(
    archive: &Path,
    destination: &Path,
    initial_password: Option<&str>,
    cancel: &AtomicBool,
    conflicts: &mut dyn FnMut(&Path, &Path) -> ConflictChoice,
    passwords: &mut dyn FnMut(&Path, Option<&str>) -> Option<String>,
) -> Result<(), String> {
    extract_interactive_with_program(
        archive,
        destination,
        initial_password,
        cancel,
        conflicts,
        passwords,
        None,
    )
}
pub fn extract_interactive_with_program(
    archive: &Path,
    destination: &Path,
    initial_password: Option<&str>,
    cancel: &AtomicBool,
    conflicts: &mut dyn FnMut(&Path, &Path) -> ConflictChoice,
    passwords: &mut dyn FnMut(&Path, Option<&str>) -> Option<String>,
    program: Option<&ExternalProgram>,
) -> Result<(), String> {
    extract_entries_with_program(archive, destination, &[], initial_password, cancel, conflicts, passwords, program, &mut |_, _, _| {})
}

/// C# ExtractExternalWithPasswordPromptAsync(외부 프로그램, 압축 파일, 대상): 목록을 읽지 않고 압축 파일 전체를 외부 프로그램으로 푼다
/// (동봉 7-Zip이 못 읽는 형식, 예: .ace). 같은 폴더의 단계 폴더에 풀어 검사(inspect_tree)한 뒤 대상 폴더로 옮긴다.
/// 대상에 같은 이름이 있으면 덮어쓰지 않고 오류로 끝난다.
pub fn extract_whole_with_program(archive: &Path, destination: &Path, password: Option<&str>, program: &ExternalProgram, cancel: &AtomicBool) -> Result<(), String> {
    check_cancel(cancel)?;
    ensure_plain_path(destination)?;
    let archive = fs::canonicalize(archive).map_err(|e| e.to_string())?;
    let parent = destination.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    ensure_plain_path(parent)?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let stage = unique_path(parent, "whole");
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        let stage_absolute = fs::canonicalize(&stage).map_err(|e| e.to_string())?;
        let mut cmd = program_command(Some(program))?;
        cmd.current_dir(&stage_absolute);
        cmd.args(crate::commands::build_archive_extract_args(&archive, &stage_absolute, password.unwrap_or(""), &[], program.extract_template.as_deref(), true)?);
        execute_capture(cmd, cancel, false)?;
        inspect_tree(&stage)?;
        fs::create_dir_all(destination).map_err(|e| e.to_string())?;
        for child in fs::read_dir(&stage).map_err(|e| e.to_string())? {
            let child = child.map_err(|e| e.to_string())?;
            rename_new(&child.path(), &destination.join(child.file_name()))?;
        }
        Ok(())
    })();
    let _ = fs::remove_dir_all(&stage);
    result
}

/// 외부 압축 프로그램이 암호를 물을 때 표준 출력에 찍는 줄(7-Zip 문법).
const EXTERNAL_PASSWORD_PROMPT: &str = "Enter password (will not be echoed):";

/// C# HasExternalArchivePasswordDiagnostic: 표준 출력의 암호 묻기 줄, 또는 "ERROR: Wrong password"·
/// "ERROR: Data Error in encrypted file. Wrong password?"로 시작하거나 "ERROR: Can not open encrypted archive. Wrong password?"인 줄.
fn external_password_failure(message: &str) -> bool {
    message.lines().any(|line| {
        let line = line.split_once("): ").map_or(line, |(_, rest)| rest);
        let (start, whole) = (line.trim_start().to_ascii_lowercase(), line.trim().to_ascii_lowercase());
        whole == EXTERNAL_PASSWORD_PROMPT.to_ascii_lowercase()
            || start.starts_with("error: wrong password")
            || start.starts_with("error: data error in encrypted file. wrong password?")
            || whole == "error: can not open encrypted archive. wrong password?"
    })
}

/// C# RunExternalExtractWithPasswordPromptAsync: 외부 프로그램이 암호 진단을 내면 `passwords`로 묻고(빈 값·취소면 끝) 그 암호로 다시 푼다.
pub fn extract_whole_with_program_with_passwords(
    archive: &Path,
    destination: &Path,
    password: Option<&str>,
    program: &ExternalProgram,
    cancel: &AtomicBool,
    passwords: &mut dyn FnMut(&Path) -> Option<String>,
) -> Result<(), String> {
    let mut current = password.map(str::to_owned);
    loop {
        match extract_whole_with_program(archive, destination, current.as_deref(), program, cancel) {
            Err(error) if external_password_failure(&error) => match passwords(archive).filter(|p| !p.is_empty()) {
                Some(value) => current = Some(value),
                None => return Err("Archive password request cancelled".into()),
            },
            other => return other,
        }
    }
}

/// C# ExtractArchiveWithProgress(requestedKeys): keys가 비면 모두, 아니면 같은 키이거나 그 폴더 안의 항목만(대소문자 무시, 전체 경로 유지).
/// 진행은 (끝낸 항목 수, 고른 항목 수, 키).
#[allow(clippy::too_many_arguments)]
pub fn extract_entries_with_program(
    archive: &Path,
    destination: &Path,
    keys: &[String],
    initial_password: Option<&str>,
    cancel: &AtomicBool,
    conflicts: &mut dyn FnMut(&Path, &Path) -> ConflictChoice,
    passwords: &mut dyn FnMut(&Path, Option<&str>) -> Option<String>,
    program: Option<&ExternalProgram>,
    progress: &mut dyn FnMut(usize, usize, &str),
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    check_cancel(cancel)?;
    ensure_plain_path(destination)?;
    let archive = fs::canonicalize(archive).map_err(|e| e.to_string())?;
    let mut header_password = initial_password.map(str::to_owned);
    let (entries, encrypted) = loop {
        match list_details(&archive, header_password.as_deref(), cancel) {
            Ok(details) => break details,
            Err(error) if password_failure(&error) => match passwords(&archive, None) {
                Some(value) => header_password = Some(value),
                None => {
                    cancel.store(true, Ordering::Relaxed);
                    return Err("Archive password request cancelled".into());
                }
            },
            Err(error) => return Err(error),
        }
    };
    let wanted: Vec<String> = keys.iter().map(|k| k.replace('\\', "/").trim_end_matches('/').to_lowercase()).filter(|k| !k.is_empty()).collect();
    let entries: Vec<ArchiveEntry> = entries
        .into_iter()
        .filter(|e| {
            let key = e.key.replace('\\', "/").trim_end_matches('/').to_lowercase();
            wanted.is_empty() || wanted.iter().any(|w| key == *w || key.starts_with(&format!("{w}/")))
        })
        .collect();
    let total = entries.len();
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    ensure_plain_path(parent)?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let aggregate = unique_path(parent, "aggregate");
    fs::create_dir(&aggregate).map_err(|e| e.to_string())?;
    let mut known = vec![header_password];
    let mut completed = 0usize;
    let mut stopped = None;
    for entry in &entries {
        if let Err(error) = check_cancel(cancel) {
            stopped = Some(error);
            break;
        }
        let relative = entry.key.replace('\\', "/");
        let target = aggregate.join(&relative);
        if entry.is_dir {
            if let Err(e) = fs::create_dir_all(&target) {
                stopped = Some(e.to_string());
                break;
            }
            completed += 1;
            progress(completed, total, &entry.key);
            continue;
        }
        let mut candidate_index = 0;
        let mut supplied = None;
        loop {
            if let Err(error) = check_cancel(cancel) {
                stopped = Some(error);
                break;
            }
            let mut password = if candidate_index < known.len() {
                let value = known[candidate_index].clone();
                candidate_index += 1;
                value
            } else {
                supplied.take()
            };
            // Some external templates omit an empty password switch, which can
            // leave their console waiting for input. Prompt only when trusted
            // archive metadata explicitly marks this entry as encrypted.
            if program.is_some() && password.is_none() && encrypted.contains(&entry.key) {
                if candidate_index < known.len() {
                    continue;
                }
                match passwords(&archive, Some(&entry.key)) {
                    Some(value) => password = Some(value),
                    None => {
                        cancel.store(true, Ordering::Relaxed);
                        stopped = Some("Archive password request cancelled".into());
                        break;
                    }
                }
            }
            let attempt = unique_path(parent, "decode");
            if let Err(e) = fs::create_dir(&attempt) {
                stopped = Some(e.to_string());
                break;
            }
            let decoded = (|| {
                let attempt_absolute = fs::canonicalize(&attempt).map_err(|e| e.to_string())?;
                let mut cmd = program_command(program)?;
                cmd.current_dir(&attempt_absolute);
                if let Some(program) = program {
                    let args = crate::commands::build_archive_extract_args(
                        &archive,
                        &attempt_absolute,
                        password.as_deref().unwrap_or(""),
                        &[entry.key.clone()],
                        program.extract_template.as_deref(),
                        true,
                    )?;
                    cmd.args(args);
                } else {
                    cmd.args(["x", "-y", "-sccUTF-8", "-spd", "-snl-"])
                        .arg(format!("-o{}", attempt_absolute.display()));
                    password_arg(&mut cmd, password.as_deref())?;
                    cmd.arg("--").arg(&archive).arg(&entry.key);
                }
                execute_capture(cmd, cancel, program.is_none())?;
                inspect_tree(&attempt)?;
                let source = attempt.join(&relative);
                let metadata = fs::symlink_metadata(&source)
                    .map_err(|e| format!("Missing decoded entry {}: {e}", entry.key))?;
                if !metadata.is_file() || metadata.len() != entry.size {
                    return Err(format!("Decoded entry mismatch: {}", entry.key));
                }
                let target_parent = target.parent().ok_or("Decoded target parent missing")?;
                fs::create_dir_all(target_parent).map_err(|e| e.to_string())?;
                rename_new(&source, &target)
            })();
            let _ = fs::remove_dir_all(&attempt);
            match decoded {
                Ok(()) => {
                    if !known.contains(&password) {
                        known.push(password)
                    }
                    completed += 1;
                    progress(completed, total, &entry.key);
                    break;
                }
                Err(error) if password_failure(&error) => {
                    if candidate_index < known.len() {
                        continue;
                    }
                    match passwords(&archive, Some(&entry.key)) {
                        Some(value) => supplied = Some(value),
                        None => {
                            cancel.store(true, Ordering::Relaxed);
                            stopped = Some("Archive password request cancelled".into());
                            break;
                        }
                    }
                }
                Err(error) => {
                    stopped = Some(error);
                    break;
                }
            }
        }
        if stopped.is_some() {
            break;
        }
    }
    let result = (|| {
        if completed > 0 {
            let finish_cancel = AtomicBool::new(false);
            let commit_cancel = if cancel.load(Ordering::Relaxed) {
                &finish_cancel
            } else {
                cancel
            };
            commit_aggregate(&aggregate, destination, commit_cancel, conflicts)?;
        }
        if let Some(error) = stopped {
            Err(format!(
                "{error}; {completed} successful archive entries were preserved"
            ))
        } else {
            Ok(())
        }
    })();
    let _ = fs::remove_dir_all(&aggregate);
    result
}

/// 원본 FUN_00404890(압축 엔진 오류 번호 → [RES] 키)에 맞춘 7-Zip 진단 분류. 원본 엔진 번호 대신 7-Zip 글을 본다.
/// 알 수 없으면 None(7-Zip 글 그대로).
pub fn original_error_key(message: &str) -> Option<&'static str> {
    if password_failure(message) {
        return Some("INVALID_PASSWORD");
    }
    let lower = message.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));
    Some(if has(&["bundled 7-zip not found"]) {
        "7ZDLL_LOAD_FAILED"
    } else if has(&["not enough space on the disk", "disk is full", "disk full"]) {
        "DISKISFULL"
    } else if has(&["missing volume"]) {
        "CANNOT_FIND_NEXTVOLUME"
    } else if has(&["cannot open the file as archive", "can not open the file as archive", "is not archive", "unsupported archive"]) {
        "UNSUPPORTED_ARCHIVE_FILE"
    } else if has(&["data error", "crc failed", "headers error", "unexpected end of archive"]) {
        "RAR_INVALIDFILE_OR_CORRUPTED"
    } else if has(&["can not open output file", "cannot open output file", "cannot create", "can not create"]) {
        "CANNOT_CREATEFILE"
    } else if has(&["write error", "cannot write", "can not write"]) {
        "WRITE_FAILED"
    } else if has(&["read error", "cannot read", "can not read"]) {
        "CANNOT_READFILE"
    } else if has(&["cannot open", "can not open", "cannot find the file", "cannot find the path"]) {
        "CANNOT_OPEN_FILE"
    } else {
        return None;
    })
}

/// 7-Zip "ERROR: Missing volume : 이름" 줄의 이름(원본 ARC_CHANGEVOL의 %s). 없으면 None.
pub fn missing_volume(message: &str) -> Option<String> {
    message.lines().find_map(|line| {
        let lower = line.to_lowercase();
        let at = lower.find("missing volume")?;
        let rest = line[at + "missing volume".len()..].trim_start();
        let name = rest.strip_prefix(':').unwrap_or(rest).trim();
        (!name.is_empty()).then(|| name.to_owned())
    })
}

/// Match 7-Zip's diagnostic grammar, never a filename merely containing password words.
fn password_failure(message: &str) -> bool {
    message.lines().any(|line| {
        let line = line.trim();
        let line = line.split_once("): ").map(|(_, s)| s).unwrap_or(line);
        line.starts_with("ERROR: Wrong password : ")
            || line.starts_with("ERROR: Data Error in encrypted file. Wrong password? : ")
            || (line.starts_with("ERROR: ")
                && (line.ends_with(" : Cannot open encrypted archive. Wrong password?")
                    || line.ends_with(" : Can not open encrypted archive. Wrong password?")))
    })
}

#[cfg(test)]
mod interactive_tests {
    use super::*;
    #[test]
    fn seven_zip_diagnostics_map_to_the_original_error_keys() {
        assert_eq!(original_error_key("7-Zip failed (exit code: 2): ERROR: Wrong password : a"), Some("INVALID_PASSWORD"));
        assert_eq!(original_error_key("7-Zip failed (exit code: 2): ERROR: C:\\x.bin\nCan not open the file as archive"), Some("UNSUPPORTED_ARCHIVE_FILE"));
        assert_eq!(original_error_key("ERROR: There is not enough space on the disk."), Some("DISKISFULL"));
        assert_eq!(original_error_key("ERROR: Missing volume : x.7z.002"), Some("CANNOT_FIND_NEXTVOLUME"));
        assert_eq!(original_error_key("ERROR: Data Error : a.txt"), Some("RAR_INVALIDFILE_OR_CORRUPTED"));
        assert_eq!(original_error_key("ERROR: Can not open output file : Access is denied. : C:\\o\\a.txt"), Some("CANNOT_CREATEFILE"));
        assert_eq!(original_error_key("Bundled 7-Zip not found; set NDR_RESOURCE_DIR to the resource directory"), Some("7ZDLL_LOAD_FAILED"));
        assert_eq!(original_error_key("Cancelled"), None);
        assert_eq!(missing_volume("7-Zip failed (exit code: 2): Open ERROR\nERROR: Missing volume : C:\\a\\x.7z.002\n"), Some("C:\\a\\x.7z.002".into()));
        assert_eq!(missing_volume("ERROR: Data Error : a.txt"), None);
    }

    #[test]
    fn fork_banner_in_the_oem_code_page_is_tolerated_but_entries_stay_strict() {
        let banner = b"\r\n7zip+\xa5\xe1 26.05 (x64) : Copyright (c) 1999-2026 Igor Pavlov : 2026-10-21\r\n\r\nPath = \xed\x95\x9c.txt\r\n".to_vec();
        let text = decode_tool_output(banner).unwrap();
        assert!(text.ends_with("\r\nPath = 한.txt\r\n") && text.contains("Copyright (c)"));
        assert!(decode_tool_output(b"\r\n7zip+\xa5\xe1 : Copyright (c)\r\nPath = \xa5\xe1\r\n".to_vec()).is_err());
        assert!(decode_tool_output(b"Path = \xa5\xe1\r\n".to_vec()).is_err());
        assert_eq!(decode_tool_output(b"\r\n7-Zip 26.04 : Copyright (c) Igor Pavlov\r\nok".to_vec()).unwrap(), "\r\n7-Zip 26.04 : Copyright (c) Igor Pavlov\r\nok");
        let entries = parse_listing("Path = 폴더\\\nFolder = +\nAttributes = DN\n\nPath = 폴더\\한글.txt\nFolder = -\nSize = 3\n").unwrap();
        assert_eq!(entries.iter().map(|e| (e.key.as_str(), e.is_dir)).collect::<Vec<_>>(), [("폴더", true), ("폴더\\한글.txt", false)]);
        assert!(parse_listing("Path = a.txt\\\nFolder = -\n").is_err(), "a file key ending in a separator stays unsafe");
    }

    #[test]
    fn password_diagnostics_ignore_unrelated_path_words() {
        assert!(password_failure(
            "7-Zip failed (exit code: 2): ERROR: Wrong password : a"
        ));
        assert!(password_failure(
            "ERROR: C:\\archive.7z : Cannot open encrypted archive. Wrong password?"
        ));
        for message in [
            "Cannot open C:\\Wrong password\\data.zip",
            "ERROR: Cannot create output file : Wrong password.txt",
            "Unsafe archive entry: Wrong password?",
            "ERROR: Data Error : a",
        ] {
            assert!(!password_failure(message), "{message}");
        }
    }
    #[test]
    fn encrypted_headers_retry_and_folder_replace_and_rename_are_aggregate() {
        let p = super::tests::fixture();
        fs::create_dir(p.join("folder")).unwrap();
        fs::write(p.join("folder/new"), b"new").unwrap();
        let cancel = AtomicBool::new(false);
        create(
            &[p.join("folder")],
            &p.join("header.7z"),
            Some("secret"),
            &cancel,
        )
        .unwrap();
        fs::create_dir_all(p.join("out/folder")).unwrap();
        fs::write(p.join("out/folder/old"), b"old").unwrap();
        let mut asked = 0;
        extract_interactive(
            &p.join("header.7z"),
            &p.join("out"),
            Some("wrong"),
            &cancel,
            &mut |_, _| ConflictChoice::Rename,
            &mut |_, entry| {
                assert!(entry.is_none());
                asked += 1;
                Some("secret".into())
            },
        )
        .unwrap();
        assert_eq!(asked, 1);
        assert_eq!(fs::read(p.join("out/folder/old")).unwrap(), b"old");
        assert_eq!(fs::read(p.join("out/folder (1)/new")).unwrap(), b"new");
        extract_interactive(
            &p.join("header.7z"),
            &p.join("out"),
            Some("secret"),
            &cancel,
            &mut |_, _| ConflictChoice::ReplaceFolder,
            &mut |_, _| panic!("known password prompted"),
        )
        .unwrap();
        assert!(!p.join("out/folder/old").exists());
        assert_eq!(fs::read(p.join("out/folder/new")).unwrap(), b"new");
        let cancelled = AtomicBool::new(false);
        assert!(
            extract_interactive(
                &p.join("header.7z"),
                &p.join("not-created"),
                None,
                &cancelled,
                &mut |_, _| ConflictChoice::Overwrite,
                &mut |_, _| None
            )
            .is_err()
        );
        assert!(!p.join("not-created").exists());
        fs::remove_dir_all(p).unwrap();
    }
    fn mixed_zip(p: &Path) {
        fs::create_dir(p.join("folder")).unwrap();
        fs::write(p.join("folder/one"), b"first").unwrap();
        fs::write(p.join("folder/two"), b"second").unwrap();
        for (name, password) in [("folder/one", "first"), ("folder/two", "second")] {
            let mut c = command().unwrap();
            c.current_dir(p)
                .args(["a", "-tzip", "-mem=AES256", "-sccUTF-8"])
                .arg(format!("-p{password}"))
                .arg("--")
                .arg(p.join("mixed.zip"))
                .arg(name);
            execute(c, &AtomicBool::new(false)).unwrap();
        }
    }
    #[test]
    fn mixed_passwords_retry_and_folder_skip_preserve_originals() {
        let p = super::tests::fixture();
        mixed_zip(&p);
        let cancel = AtomicBool::new(false);
        let mut asked = 0;
        extract_interactive(
            &p.join("mixed.zip"),
            &p.join("out"),
            Some("wrong"),
            &cancel,
            &mut |_, _| ConflictChoice::Overwrite,
            &mut |_, entry| {
                asked += 1;
                Some(
                    if entry.is_some_and(|s| s.ends_with("one")) {
                        "first"
                    } else {
                        "second"
                    }
                    .into(),
                )
            },
        )
        .unwrap();
        assert!(asked >= 2);
        assert_eq!(fs::read(p.join("out/folder/one")).unwrap(), b"first");
        assert_eq!(fs::read(p.join("out/folder/two")).unwrap(), b"second");
        fs::write(p.join("out/folder/one"), b"protected").unwrap();
        extract_interactive(
            &p.join("mixed.zip"),
            &p.join("out"),
            None,
            &cancel,
            &mut |s, _| {
                assert!(s.is_dir());
                ConflictChoice::Skip
            },
            &mut |_, entry| {
                Some(
                    if entry.is_some_and(|s| s.ends_with("one")) {
                        "first"
                    } else {
                        "second"
                    }
                    .into(),
                )
            },
        )
        .unwrap();
        assert_eq!(fs::read(p.join("out/folder/one")).unwrap(), b"protected");
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn password_cancel_commits_only_successes_without_placeholders() {
        let p = super::tests::fixture();
        mixed_zip(&p);
        fs::create_dir_all(p.join("out/folder")).unwrap();
        fs::write(p.join("out/folder/two"), b"old second").unwrap();
        let cancel = AtomicBool::new(false);
        let result = extract_interactive(
            &p.join("mixed.zip"),
            &p.join("out"),
            Some("first"),
            &cancel,
            &mut |_, _| ConflictChoice::Overwrite,
            &mut |_, _| None,
        );
        assert!(result.is_err());
        assert_eq!(fs::read(p.join("out/folder/one")).unwrap(), b"first");
        assert_eq!(fs::read(p.join("out/folder/two")).unwrap(), b"old second");
        assert!(cancel.load(std::sync::atomic::Ordering::Relaxed));
        fs::remove_dir_all(p).unwrap();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::SystemTime};
    pub(super) fn fixture() -> PathBuf {
        // 병렬 시험이 같은 시각 값을 받아 한 폴더를 함께 쓰지 않게 차례 번호를 붙인다.
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "ndr-archive-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        p
    }
    /// 7-Zip -bsp1 진행 글: 백분율과 파일 이름. 동봉 7-Zip 압축은 끝나기 전에 진행을 알린다.
    #[test]
    fn seven_zip_progress_is_parsed_and_reported() {
        assert_eq!(parse_seven_zip_progress("  23% 4 + folder\\a.bin"), Some((23, "folder\\a.bin".to_owned())));
        assert_eq!(parse_seven_zip_progress(" 18% + noise.bin"), Some((18, "noise.bin".to_owned())), "bundled 7-Zip -bsp1 line");
        assert_eq!(parse_seven_zip_progress("100%"), Some((100, String::new())));
        assert_eq!(parse_seven_zip_progress("Scanning"), None);
        let p = fixture();
        let mut state = 0x9e37_79b9u32;
        let noise: Vec<u8> = (0..3 * 1024 * 1024).map(|_| { state ^= state << 13; state ^= state >> 17; state ^= state << 5; (state >> 24) as u8 }).collect();
        fs::write(p.join("noise.bin"), &noise).unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(u8, String)>::new()));
        let record = seen.clone();
        let sink: ProgressSink = std::sync::Arc::new(move |percent, name| record.lock().unwrap().push((percent, name.to_owned())));
        let options = crate::commands::ArchiveOptions { extension: "7z".into(), level: 5, password: String::new(), volume_bytes: None, recursive: true, append: false };
        create_with_progress(&[p.join("noise.bin")], &p.join("out.7z"), &options, None, &AtomicBool::new(false), Some(sink)).unwrap();
        let seen = seen.lock().unwrap();
        assert!(!seen.is_empty(), "progress was reported");
        assert!(seen.iter().all(|(percent, _)| *percent <= 100), "{seen:?}");
        assert!(p.join("out.7z").is_file());
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn rejects_windows_escape_keys() {
        for key in [
            "../escape",
            "a/../../escape",
            "C:\\escape",
            "/absolute",
            "\\\\server\\share",
            "ok:stream",
            "a/../b",
            "CON.txt",
            "a./b",
            "a /b",
            "a\\..\\b",
        ] {
            assert!(validate_key(key).is_err(), "accepted {key}");
        }
        assert!(validate_key("nested/한국어.txt").is_ok());
    }
    #[test]
    fn real_archive_roundtrip_and_auth_failure_preserves_destination() {
        let p = fixture();
        fs::write(p.join("한글.txt"), b"original data").unwrap();
        let cancel = AtomicBool::new(false);
        create(
            &[p.join("한글.txt")],
            &p.join("test.7z"),
            Some("correct"),
            &cancel,
        )
        .unwrap();
        let entries = list(&p.join("test.7z"), Some("correct")).unwrap();
        assert!(entries.iter().any(|e| e.key == "한글.txt" && e.size == 13));
        fs::create_dir(p.join("out")).unwrap();
        fs::write(p.join("out/한글.txt"), b"old data").unwrap();
        assert!(extract(&p.join("test.7z"), &p.join("out"), Some("wrong"), &cancel).is_err());
        assert_eq!(fs::read(p.join("out/한글.txt")).unwrap(), b"old data");
        assert!(extract(&p.join("test.7z"), &p.join("out"), Some("correct"), &cancel).is_err());
        assert_eq!(fs::read(p.join("out/한글.txt")).unwrap(), b"old data");
        extract_with_conflicts(
            &p.join("test.7z"),
            &p.join("out"),
            Some("correct"),
            &cancel,
            &mut |_, _| ConflictChoice::Overwrite,
        )
        .unwrap();
        assert_eq!(fs::read(p.join("out/한글.txt")).unwrap(), b"original data");
        assert!(
            create(
                &[p.join("한글.txt")],
                &p.join("test.7z"),
                None,
                &AtomicBool::new(true)
            )
            .is_err()
        );
        fs::remove_dir_all(p).unwrap();
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    fn zip(path: &Path, key: &str, data: &[u8], attributes: u32) {
        let mut crc = 0xffff_ffffu32;
        for b in data {
            crc ^= *b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                }
            }
        }
        let crc = !crc;
        let mut bytes = Vec::new();
        fn w(b: &mut Vec<u8>, v: u16) {
            b.extend(v.to_le_bytes())
        }
        fn d(b: &mut Vec<u8>, v: u32) {
            b.extend(v.to_le_bytes())
        }
        d(&mut bytes, 0x04034b50);
        for v in [20, 0, 0, 0, 0] {
            w(&mut bytes, v)
        }
        d(&mut bytes, crc);
        d(&mut bytes, data.len() as u32);
        d(&mut bytes, data.len() as u32);
        w(&mut bytes, key.len() as u16);
        w(&mut bytes, 0);
        bytes.extend(key.as_bytes());
        bytes.extend(data);
        let offset = bytes.len() as u32;
        d(&mut bytes, 0x02014b50);
        for v in [0x0314, 20, 0, 0, 0, 0] {
            w(&mut bytes, v)
        }
        d(&mut bytes, crc);
        d(&mut bytes, data.len() as u32);
        d(&mut bytes, data.len() as u32);
        w(&mut bytes, key.len() as u16);
        for _ in 0..4 {
            w(&mut bytes, 0)
        }
        d(&mut bytes, attributes);
        d(&mut bytes, 0);
        bytes.extend(key.as_bytes());
        let size = bytes.len() as u32 - offset;
        d(&mut bytes, 0x06054b50);
        w(&mut bytes, 0);
        w(&mut bytes, 0);
        w(&mut bytes, 1);
        w(&mut bytes, 1);
        d(&mut bytes, size);
        d(&mut bytes, offset);
        w(&mut bytes, 0);
        fs::write(path, bytes).unwrap();
    }
    #[test]
    fn malicious_zip_paths_and_symlinks_rejected_before_extraction() {
        let p = super::tests::fixture();
        let cancel = AtomicBool::new(false);
        zip(&p.join("traversal.zip"), "../escaped", b"bad", 0x81a40000);
        assert!(extract(&p.join("traversal.zip"), &p.join("out"), None, &cancel).is_err());
        assert!(!p.join("escaped").exists());
        zip(&p.join("link.zip"), "link", b"../outside", 0xa1ff0000);
        assert!(extract(&p.join("link.zip"), &p.join("out"), None, &cancel).is_err());
        assert!(!p.join("out/link").exists());
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn create_with_output_inside_selected_source_does_not_recurse() {
        let p = super::tests::fixture();
        fs::create_dir(p.join("source")).unwrap();
        fs::write(p.join("source/Upper.TXT"), b"data").unwrap();
        let cancel = AtomicBool::new(false);
        create(
            &[p.join("source")],
            &p.join("source/out.zip"),
            None,
            &cancel,
        )
        .unwrap();
        let found = list(&p.join("source/out.zip"), None).unwrap();
        assert!(
            found
                .iter()
                .any(|e| e.key.replace('\\', "/") == "source/Upper.TXT")
        );
        assert!(!found.iter().any(|e| e.key.contains(".ndr-")));
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn failed_commit_restores_every_existing_file() {
        let p = super::tests::fixture();
        let cancel = AtomicBool::new(false);
        fs::create_dir(p.join("stage")).unwrap();
        fs::create_dir(p.join("dest")).unwrap();
        fs::create_dir(p.join("backup")).unwrap();
        fs::write(p.join("stage/a"), b"new").unwrap();
        fs::write(p.join("dest/a"), b"old").unwrap();
        let mut commit = Commit {
            installed: Vec::new(),
            backups: Vec::new(),
            dirs: Vec::new(),
        };
        commit
            .merge(
                &p.join("stage/a"),
                &p.join("dest/a"),
                &p.join("backup"),
                &cancel,
            )
            .unwrap();
        fs::create_dir(p.join("stage/collision")).unwrap();
        fs::write(p.join("dest/collision"), b"keep").unwrap();
        assert!(
            commit
                .merge(
                    &p.join("stage/collision"),
                    &p.join("dest/collision"),
                    &p.join("backup"),
                    &cancel
                )
                .is_err()
        );
        commit.rollback().unwrap();
        assert_eq!(fs::read(p.join("dest/a")).unwrap(), b"old");
        assert_eq!(fs::read(p.join("dest/collision")).unwrap(), b"keep");
        fs::remove_dir_all(p).unwrap();
    }
}
#[cfg(test)]
mod configured_tests {
    use super::*;
    use crate::commands::ArchiveOptions;
    #[test]
    fn configured_creation_honors_volume_size_and_custom_templates_are_staged() {
        let p = super::tests::fixture();
        fs::write(p.join("data.bin"), vec![42; 65536]).unwrap();
        let cancel = AtomicBool::new(false);
        let options = ArchiveOptions {
            extension: "zip".into(),
            level: 0,
            password: String::new(),
            volume_bytes: Some(4096),
            recursive: true,
            append: false,
        };
        create_configured(
            &[p.join("data.bin")],
            &p.join("split.zip"),
            &options,
            &cancel,
        )
        .unwrap();
        assert!(p.join("split.zip.001").exists());
        assert!(p.join("split.zip.002").exists());
        for e in fs::read_dir(&p).unwrap() {
            let e = e.unwrap();
            if e.file_name().to_string_lossy().starts_with("split.zip.") {
                assert!(e.metadata().unwrap().len() <= 4096)
            }
        }
        let options = ArchiveOptions {
            volume_bytes: None,
            password: "secret".into(),
            ..options
        };
        let program = ExternalProgram {
            program: seven_zip_path().unwrap(),
            create_template: Some(
                "a -tzip -mx={level} -mem=AES256 -p{password} -sccUTF-8 {archive} {sources}".into(),
            ),
            extract_template: Some(
                "x {archive} -o{destination} -y -sccUTF-8 -p{password} {keys}".into(),
            ),
        };
        create_with_program(
            &[p.join("data.bin")],
            &p.join("custom.zip"),
            &options,
            Some(&program),
            &cancel,
        )
        .unwrap();
        let mut asked = 0;
        extract_interactive_with_program(
            &p.join("custom.zip"),
            &p.join("out"),
            None,
            &cancel,
            &mut |_, _| ConflictChoice::Overwrite,
            &mut |_, key| {
                assert!(key.is_some());
                asked += 1;
                Some("secret".into())
            },
            Some(&program),
        )
        .unwrap();
        assert_eq!(asked, 1);
        assert_eq!(fs::read(p.join("out/data.bin")).unwrap(), vec![42; 65536]);
        fs::write(p.join("protected.zip"), b"old archive").unwrap();
        let bad = ExternalProgram {
            create_template: Some("a -tzip -invalid-switch {archive} {sources}".into()),
            ..program
        };
        assert!(
            create_with_program(
                &[p.join("data.bin")],
                &p.join("protected.zip"),
                &options,
                Some(&bad),
                &cancel
            )
            .is_err()
        );
        assert_eq!(fs::read(p.join("protected.zip")).unwrap(), b"old archive");
        fs::remove_dir_all(p).unwrap();
    }
}

#[cfg(test)]
mod entry_tests {
    use super::*;

    #[test]
    fn archive_format_and_entry_extraction_follow_filesystemservice() {
        for (name, format) in [("a.ZIP", Some("zip")), ("a.tar.gz", Some("tar.gz")), ("a.7z.001", Some("7z")), ("a.part2.rar", Some("rar")), ("a.r01", Some("rar")), ("a.z02", Some("zip")), ("a.a01", Some("arj")), ("a.zip.123", Some("zip")), ("a.txt", None), ("a.001", None)] {
            assert_eq!(archive_format(Path::new(name)).as_deref(), format, "{name}");
        }
        let root = std::env::temp_dir().join(format!("ndr-entry-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(root.join("src/inner")).unwrap();
        fs::write(root.join("src/inner/b.txt"), b"inside").unwrap();
        fs::write(root.join("src/a[1].txt"), b"wild").unwrap();
        let output = root.join("pack.zip");
        let cancel = AtomicBool::new(false);
        create(&[root.join("src/inner"), root.join("src/a[1].txt")], &output, None, &cancel).unwrap();
        let keys: Vec<String> = list(&output, None).unwrap().into_iter().map(|e| e.key).collect();
        let inner = keys.iter().find(|k| k.ends_with("b.txt")).unwrap().clone();
        let extracted = extract_entry_to_temp(&output, None, &inner, &cancel).unwrap();
        assert_eq!(fs::read(&extracted).unwrap(), b"inside");
        let folder = extracted.ancestors().find(|p| p.parent() == Some(&std::env::temp_dir().join("NDR"))).unwrap().to_path_buf();
        assert!(folder.file_name().unwrap().to_string_lossy().starts_with(".ndr-entry-"), "a new folder under %TEMP%\\NDR");
        fs::remove_dir_all(&folder).unwrap();
        let wild = extract_entry_to_temp(&output, None, "a[1].txt", &cancel).unwrap();
        assert_eq!(fs::read(&wild).unwrap(), b"wild", "-spd: brackets are not wildcards");
        fs::remove_dir_all(wild.parent().unwrap()).unwrap();
        assert!(extract_entry_to_temp(&output, None, "../x", &cancel).is_err());
        // 고친 것: 없는 항목처럼 풀기에 실패하면 만든 폴더를 남기지 않는다.
        let before: Vec<_> = fs::read_dir(temp_entry_root()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(extract_entry_to_temp(&output, None, "missing.txt", &cancel).is_err());
        let after: Vec<_> = fs::read_dir(temp_entry_root()).unwrap().flatten().map(|e| e.file_name()).filter(|n| !before.contains(n)).collect();
        assert!(after.iter().all(|n| !n.to_string_lossy().starts_with(&format!(".ndr-entry-{}-", std::process::id()))), "{after:?}");
        fs::remove_dir_all(root).unwrap();
    }

    /// 고친 것: 오래된 ".ndr-entry-*"만 지우고, 최근 것·다른 이름·열려 있어 이름을 못 바꾸는 것은 남긴다.
    #[test]
    fn stale_temp_entry_folders_are_removed() {
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::{Duration, SystemTime};
        let root = std::env::temp_dir().join(format!("ndr-clean-{}-{}", std::process::id(), SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        for name in [".ndr-entry-1-0", ".ndr-entry-2-0", ".ndr-entry-3-0", "other"] {
            fs::create_dir_all(root.join(name).join("sub")).unwrap();
            fs::write(root.join(name).join("sub").join("a.txt"), b"x").unwrap();
        }
        fs::create_dir_all(root.join(".ndr-trash-9-0")).unwrap();
        let week = Duration::from_secs(7 * 24 * 3600);
        // 지금 기준으로는 모두 최근 것: 남은 휴지 폴더만 지운다.
        assert_eq!(clean_stale_temp_entries(&root, week, SystemTime::now()), 1);
        assert!(root.join(".ndr-entry-1-0").is_dir() && !root.join(".ndr-trash-9-0").exists());
        // 8일 뒤: 열린 파일(삭제 공유 없음)이 있는 폴더와 다른 이름의 폴더는 남는다.
        let locked = fs::OpenOptions::new().read(true).share_mode(1).open(root.join(".ndr-entry-3-0").join("sub").join("a.txt")).unwrap();
        let later = SystemTime::now() + Duration::from_secs(8 * 24 * 3600);
        assert_eq!(clean_stale_temp_entries(&root, week, later), 2);
        assert!(!root.join(".ndr-entry-1-0").exists() && !root.join(".ndr-entry-2-0").exists());
        assert!(root.join(".ndr-entry-3-0").join("sub").join("a.txt").is_file(), "in use: kept whole");
        assert!(root.join("other").is_dir());
        drop(locked);
        assert_eq!(clean_stale_temp_entries(&root, week, later), 1);
        fs::remove_dir_all(root).unwrap();
    }

    /// 동봉 7-Zip(UzuCore 7zip+α 26.05): ALZ·EGG 만들기·목록·풀기(한글 이름 포함). 더하기는 7-Zip이 거절하고 원본은 그대로.
    #[test]
    fn bundled_seven_zip_creates_lists_and_extracts_alz_and_egg() {
        let root = std::env::temp_dir().join(format!("ndr-alzegg-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(root.join("src/폴더")).unwrap();
        fs::write(root.join("src/a.txt"), b"alpha").unwrap();
        fs::write(root.join("src/폴더/한글.txt"), "한글 내용".as_bytes()).unwrap();
        let cancel = AtomicBool::new(false);
        let options = |extension: &str, append: bool| crate::commands::ArchiveOptions { extension: extension.into(), level: 5, password: String::new(), volume_bytes: None, recursive: true, append };
        for format in ["alz", "egg"] {
            let archive = root.join(format!("out.{format}"));
            create_configured(&[root.join("src/a.txt"), root.join("src/폴더")], &archive, &options(format, false), &cancel).unwrap();
            assert!(can_read_internally(&archive));
            let mut keys: Vec<String> = list(&archive, None).unwrap().into_iter().filter(|e| !e.is_dir).map(|e| e.key.replace('\\', "/")).collect();
            keys.sort();
            assert_eq!(keys, ["a.txt", "폴더/한글.txt"], "{format}");
            let out = root.join(format!("out-{format}"));
            extract(&archive, &out, None, &cancel).unwrap();
            assert_eq!(fs::read_to_string(out.join("폴더/한글.txt")).unwrap(), "한글 내용", "{format}");
            let before = fs::read(&archive).unwrap();
            assert!(create_configured(&[root.join("src/a.txt")], &archive, &options(format, true), &cancel).is_err(), "{format} append is refused");
            assert_eq!(fs::read(&archive).unwrap(), before, "{format} original untouched");
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn archive_add_extract_keys_and_names_follow_the_csharp_flows() {
        let root = std::env::temp_dir().join(format!("ndr-arcflow-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        fs::create_dir_all(root.join("src/dir/deep")).unwrap();
        fs::write(root.join("src/a.txt"), b"a").unwrap();
        fs::write(root.join("src/b.txt"), b"bb").unwrap();
        fs::write(root.join("src/dir/deep/c.txt"), b"ccc").unwrap();
        let cancel = AtomicBool::new(false);
        let options = |extension: &str, append: bool| crate::commands::ArchiveOptions { extension: extension.into(), level: 5, password: String::new(), volume_bytes: None, recursive: true, append };
        let keys = |archive: &Path| {
            let mut keys: Vec<String> = list(archive, None).unwrap().into_iter().filter(|e| !e.is_dir).map(|e| e.key.replace('\\', "/")).collect();
            keys.sort();
            keys
        };
        // ZIP: 새로 만들고, 더하기는 있던 항목을 남긴다.
        let zip = root.join("out.zip");
        create_configured(&[root.join("src/a.txt")], &zip, &options("zip", false), &cancel).unwrap();
        create_configured(&[root.join("src/b.txt"), root.join("src/dir")], &zip, &options("zip", true), &cancel).unwrap();
        assert_eq!(keys(&zip), ["a.txt", "b.txt", "dir/deep/c.txt"]);
        create_configured(&[root.join("src/b.txt")], &zip, &options("zip", false), &cancel).unwrap();
        assert_eq!(keys(&zip), ["b.txt"], "without append the archive is replaced");
        // TAR.GZ: 만들기와 더하기.
        let tgz = root.join("out.tar.gz");
        create_configured(&[root.join("src/a.txt")], &tgz, &options("tar.gz", false), &cancel).unwrap();
        create_configured(&[root.join("src/dir")], &tgz, &options("tar.gz", true), &cancel).unwrap();
        let unpacked = root.join("tgz");
        extract(&tgz, &unpacked, None, &cancel).unwrap();
        let inner = fs::read_dir(&unpacked).unwrap().filter_map(Result::ok).map(|e| e.path()).next().unwrap();
        assert_eq!(keys(&inner), ["a.txt", "dir/deep/c.txt"]);
        // 고른 키만 풀기(폴더면 그 안 전부, 전체 경로 유지), 진행.
        let full = root.join("full.zip");
        create_configured(&[root.join("src/a.txt"), root.join("src/dir")], &full, &options("zip", false), &cancel).unwrap();
        let destination = root.join("picked");
        let mut steps = vec![];
        extract_entries_with_program(&full, &destination, &["DIR".into()], None, &cancel, &mut |_, _| ConflictChoice::Overwrite, &mut |_, _| None, None, &mut |done, total, _| steps.push((done, total))).unwrap();
        assert!(destination.join("dir/deep/c.txt").is_file() && !destination.join("a.txt").exists());
        assert_eq!(steps.last(), Some(&(3, 3)), "dir, deep, c.txt");
        let entries = list(&full, None).unwrap();
        assert!(!has_single_root_folder(&entries));
        let single = root.join("single.zip");
        create_configured(&[root.join("src/dir")], &single, &options("zip", false), &cancel).unwrap();
        assert!(has_single_root_folder(&list(&single, None).unwrap()));
        assert_eq!(summary(&entries), (2, 2, 4), "a.txt + c.txt, dir + dir/deep");
        // 이름 규칙.
        assert_eq!(extract_folder_name(Path::new("C:\\x\\sample.7z.001")), "sample.7z");
        assert_eq!(extract_folder_name(Path::new("C:\\x\\sample.zip")), "sample");
        let first = create_unique_extract_folder(&root, "sample").unwrap();
        let second = create_unique_extract_folder(&root, "sample").unwrap();
        assert_eq!((first, second), (root.join("sample"), PathBuf::from(format!("{}\\sample(2)", root.display()))));
        assert_eq!(base_name(Path::new("a.part2.rar")), "a");
        assert_eq!(base_name(Path::new("a.7z.001")), "a");
        assert_eq!(base_name(Path::new("a.tar.gz")), "a");
        fs::write(root.join("vol.7z.001"), b"x").unwrap();
        fs::write(root.join("vol.7z.002"), b"x").unwrap();
        assert_eq!(first_volume_path(&root.join("vol.7z.002")), root.join("vol.7z.001"));
        assert_eq!(output_family(&root.join("vol.7z.001")), [root.join("vol.7z.001"), root.join("vol.7z.002")]);
        assert!(can_read_internally(Path::new("a.zip")) && !can_read_internally(Path::new("a.iso")) && !can_read_internally(Path::new("a.part2.rar")));
        assert!(can_read_internally(Path::new("a.alz")) && can_read_internally(Path::new("a.egg")));
        // 동봉 7-Zip은 ACE를 못 읽는다(형식 목록에 없음): 내부 읽기·안으로 들어가기 모두 아님.
        assert!(!can_read_internally(Path::new("a.ace")) && !bundled_can_browse(Path::new("a.ace")) && bundled_can_browse(Path::new("a.iso")));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn whole_archive_extraction_with_an_external_program_moves_everything_into_the_folder() {
        let root = super::tests::fixture();
        fs::create_dir_all(root.join("src/inner")).unwrap();
        fs::write(root.join("src/inner/b.txt"), b"inside").unwrap();
        fs::write(root.join("src/a.txt"), b"top").unwrap();
        let cancel = AtomicBool::new(false);
        let archive = root.join("pack.zip");
        create(&[root.join("src/inner"), root.join("src/a.txt")], &archive, None, &cancel).unwrap();
        // 목록 없이 통째로(C# ExtractExternalWithPasswordPromptAsync): 외부 프로그램 자리에 동봉 7-Zip과 풀기 형식.
        let program = ExternalProgram { program: seven_zip_path().unwrap(), create_template: None, extract_template: Some("x {archive} -o{destination} -y {keys}".into()) };
        let destination = root.join("pack");
        extract_whole_with_program(&archive, &destination, None, &program, &cancel).unwrap();
        assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"top");
        assert_eq!(fs::read(destination.join("inner/b.txt")).unwrap(), b"inside");
        // 단계 폴더는 남지 않는다.
        assert!(!fs::read_dir(&root).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".ndr-whole-")));
        // 같은 이름이 이미 있으면 덮어쓰지 않고 오류.
        assert!(extract_whole_with_program(&archive, &destination, None, &program, &cancel).is_err());
        assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"top");
        // C# RunExternalExtractWithPasswordPromptAsync: 암호 진단(7-Zip 문법)이면 묻고 그 암호로 다시 푼다. 틀리면 또 묻고, 취소면 끝.
        let secret = root.join("secret.zip");
        create(&[root.join("src/a.txt")], &secret, Some("s3cret"), &cancel).unwrap();
        let program = ExternalProgram { extract_template: Some("x {archive} -o{destination} -y -p{password} {keys}".into()), ..program };
        let mut asked = 0;
        let mut answers = |_: &Path| {
            asked += 1;
            Some(if asked == 1 { "wrong" } else { "s3cret" }.to_owned())
        };
        let unlocked = root.join("secret");
        extract_whole_with_program_with_passwords(&secret, &unlocked, None, &program, &cancel, &mut answers).unwrap();
        assert_eq!((asked, fs::read(unlocked.join("a.txt")).unwrap()), (2, b"top".to_vec()));
        let mut cancelled = |_: &Path| None;
        assert_eq!(extract_whole_with_program_with_passwords(&secret, &root.join("secret2"), None, &program, &cancel, &mut cancelled), Err("Archive password request cancelled".into()));
        assert!(!root.join("secret2").exists());
        assert!(external_password_failure("7-Zip failed (exit code: 2): ERROR: Wrong password : a.txt") && external_password_failure("x\nEnter password (will not be echoed):"));
        assert!(!external_password_failure("ERROR: a password.txt : cannot open"));
        fs::remove_dir_all(root).unwrap();
    }
}
