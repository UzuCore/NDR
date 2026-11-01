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
    /// Some raw streams do not report the uncompressed size in `7z l`.
    pub size_known: bool,
    /// "Modified = YYYY-MM-DD HH:MM:SS[.fffffff]" of a 7-Zip listing (local time). None when missing.
    pub modified: Option<std::time::SystemTime>,
}

/// 7-Zip -slt modification time (local time) -> SystemTime.
fn parse_modified(text: &str) -> Option<std::time::SystemTime> {
    use chrono::TimeZone;
    let naive = chrono::NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M:%S%.f").ok()?;
    chrono::Local.from_local_datetime(&naive).earliest().map(std::time::SystemTime::from)
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
/// Original archive writer FUN_00407628 picks by the archive extension: .zip → FUN_00408f14 (the bundled Ark compressor: level 9 / 5 / 0
/// for best / normal / store, password, names relative to the first source's folder, folders always recursed by FUN_00408d5c, an
/// existing archive rewritten through "<archive>.tmp"), .7z → FUN_004094f8 (7z.exe "u -y" for every update mode, -r, -v<n>k, -p,
/// -mx9/5/0 and a UTF-8 list file "NXArc###.$$$" in the short temp path, removed afterwards), .arj / .rar / .alz → FUN_00407e18 /
/// FUN_004085ec / FUN_00407714 (external ARJ, WinRAR and ALZ command lines). Changed (user request): everything goes through the
/// bundled 7-Zip, and ARJ, RAR and ALZ cannot be created. Fixed: the original ZIP writer ignored "Include subfolders" (always recursed).
pub fn create_with_program(
    sources: &[PathBuf],
    output: &Path,
    options: &crate::commands::ArchiveOptions,
    program: Option<&ExternalProgram>,
    cancel: &AtomicBool,
) -> Result<(), String> {
    create_with_progress(sources, output, options, program, cancel, None)
}

/// create_with_program plus progress reporting (`progress`): passes -bsp1 to the archive step (`a`) of the bundled 7-Zip and reports its percentage and file name.
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
    // C# built-in TAR.GZ / TAR.BZ2: create a TAR, then wrap it with GZip/BZip2.
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
        // Add: copy the existing archive to a staging folder and add to that copy (the original stays intact on failure).
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
        // Progress reporting only for the bundled 7-Zip (-bsp1: percentage and file name to standard output).
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
            // 1) TAR (for an add, into the TAR extracted from the existing archive), 2) wrap.
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
/// Receiver of archive progress: (percentage 0-100, current file name (the relative name 7-Zip reported)).
pub type ProgressSink = std::sync::Arc<dyn Fn(u8, &str) + Send + Sync>;

/// (percentage, name) from one chunk of 7-Zip `-bsp1` progress text ("  23% 4 + name" or "  23%").
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
            // -bsp1 progress text overwrites itself with \r, \x08 (erase) and \n. Read at each chunk end.
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
        // C# HasExternalArchivePasswordDiagnostic: an "Enter password (will not be echoed):" line on standard output is also a password diagnostic.
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
/// 7-Zip output must be UTF-8 via -sccUTF-8. But the bundled 7zip+ (UzuCore) prints its header "7zip+α ... Copyright (c) ..."
/// in the OEM code page before -scc applies (Main.cpp ShowCopyrightAndHelp, α = CP949 A5 E1), so only that first line is read lossily.
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
        let mut size_known = true;
        let mut modified = None;
        for line in block.lines() {
            if let Some(v) = line.strip_prefix("Path = ") {
                key = Some(v.to_owned())
            } else if let Some(v) = line.strip_prefix("Modified = ") {
                modified = parse_modified(v)
            } else if line == "Folder = +" {
                dir = true
            } else if let Some(v) = line.strip_prefix("Size = ") {
                if v.trim().is_empty() {
                    size_known = false;
                } else {
                    size = v.parse().map_err(|_| "Invalid archive entry size")?;
                }
            } else if let Some(v) = line.strip_prefix("Attributes = ") {
                dir |= v.starts_with('D');
                if v.contains('L') || v.contains("Reparse") {
                    return Err("Archive reparse entries are rejected".into());
                }
            } else if ["Symbolic Link = ", "Hard Link = ", "Reparse = "]
                .iter()
                .any(|prefix| line.strip_prefix(prefix).is_some_and(|value| !value.trim().is_empty()))
            {
                // 7-Zip writes an empty "Symbolic Link = " line for every TAR entry. Only links with a value are blocked.
                return Err("Archive links are rejected".into());
            }
        }
        if let Some(mut key) = key {
            // The ALZ and EGG handlers of the bundled 7zip+ report folders with a trailing separator like "folder\" (other formats "folder"). Strip it from folder entries only.
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
                size_known,
                modified,
            })
        }
    }
    Ok(entries)
}
/// Some single-stream 7zip+ handlers emit `Size = ...` without `Path =`
/// under `l -slt -ba`, although extraction produces one real file.
/// Derive that one output file's name using the same wrapper suffix as 7z x.
fn stream_member_name(path: &Path) -> Option<String> {
    let filename = path.file_name()?.to_str()?;
    let lower = filename.to_ascii_lowercase();
    let suffixes = [
        (".tar.bzip2", ".tar"), (".tar.bz2", ".tar"),
        (".tar.zstd", ".tar"), (".tar.zst", ".tar"),
        (".tar.gzip", ".tar"), (".tar.gz", ".tar"),
        (".tar.xz", ".tar"), (".tar.z", ".tar"),
        (".tgz", ".tar"), (".tpz", ".tar"),
        (".tbz2", ".tar"), (".tbz", ".tar"),
        (".txz", ".tar"), (".tzst", ".tar"),
        (".taz", ".tar"),
        (".bzip2", ""), (".bz2", ""),
        (".gzip", ""), (".gz", ""),
        (".zstd", ""), (".zst", ""),
        (".lzma86", ""), (".lzma", ""),
        (".xz", ""), (".z", ""),
    ];
    let (suffix, append) = suffixes.into_iter().find(|(suffix, _)| lower.ends_with(suffix))?;
    let stem = &filename[..filename.len().checked_sub(suffix.len())?];
    if stem.is_empty() { return None }
    let member = format!("{stem}{append}");
    validate_key(&member).ok()?;
    Some(member)
}
/// For RPM with a bzip2-wrapped cpio member, `-ba` omits the only Path.
/// A regular 7zip+ listing exposes the nested filename. The decoder removes
/// the trailing `.bz2` when extracting; the outer .rpm name is unrelated.
fn rpm_member_name(archive: &Path, password: Option<&str>, cancel: &AtomicBool) -> Result<String, String> {
    let mut cmd = command()?;
    cmd.args(["l", "-slt", "-sccUTF-8"]);
    password_arg(&mut cmd, password)?;
    cmd.arg("--").arg(archive);
    let listing = execute(cmd, cancel)?;
    let archive_name = archive.to_string_lossy();
    let inner = listing.lines().filter_map(|line| line.strip_prefix("Path = "))
        .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case(&archive_name))
        .last().ok_or("RPM payload has no named member")?;
    let name = inner.strip_suffix(".bz2").or_else(|| inner.strip_suffix(".BZ2")).unwrap_or(inner);
    validate_key(name)?;
    Ok(name.to_owned())
}
/// Lists an archive for an archive pane. Original TNXArchiveEnumerator: SetArchive FUN_0045e8b0 re-opens only when the path changes
/// (lstrcmpiW); an empty path closes it, a missing file or a failed open (FUN_0040442c / FUN_00405858, the Ark reader) clears the path;
/// FUN_0045f01c closes it, the constructor FUN_0045e6b8 (enumerator kind 3) / destructor FUN_0045e7a0 create and free it, and
/// FUN_0045ea36 looks an inner path up (FUN_00406724 / FUN_004066e4).
/// NDR reads the whole listing with the bundled 7-Zip instead of keeping a reader open.
pub fn list(archive: &Path, password: Option<&str>) -> Result<Vec<ArchiveEntry>, String> {
    list_cancel(archive, password, &AtomicBool::new(false))
}
fn list_cancel(
    archive: &Path,
    password: Option<&str>,
    cancel: &AtomicBool,
) -> Result<Vec<ArchiveEntry>, String> {
    list_details(archive, password, cancel).map(|(entries, _, _)| entries)
}
fn list_details(
    archive: &Path,
    password: Option<&str>,
    cancel: &AtomicBool,
) -> Result<(Vec<ArchiveEntry>, HashSet<String>, bool), String> {
    if !bundled_can_browse(archive) {
        return Err(format!("Archive format not supported by bundled 7-Zip: {}", archive.display()));
    }
    ensure_plain_path(archive)?;
    let archive = fs::canonicalize(archive).map_err(|e| e.to_string())?;
    let mut cmd = command()?;
    cmd.args(["l", "-slt", "-ba", "-sccUTF-8"]);
    password_arg(&mut cmd, password)?;
    cmd.arg("--").arg(&archive);
    // Some real ISO9660/UDF hybrid discs have a broken UDF header. 7zip+
    // autodetects UDF and returns exit 2 even when ISO9660 is perfectly
    // readable. Retry *only .iso files*, and only after autodetect fails.
    // Extraction must use the same handler as the successful listing.
    let (text, iso9660_fallback) = match execute(cmd, cancel) {
        Ok(text) => (text, false),
        Err(first) if archive_format(&archive).as_deref() == Some("iso") && !password_failure(&first) => {
            check_cancel(cancel)?;
            let mut retry = command()?;
            retry.args(["l", "-tiso", "-slt", "-ba", "-sccUTF-8"]);
            password_arg(&mut retry, password)?;
            retry.arg("--").arg(&archive);
            (execute(retry, cancel).map_err(|error| format!("{first}; ISO9660 fallback failed: {error}"))?, true)
        }
        Err(error) => return Err(error),
    };
    reject_unavailable_archive_methods(&archive, &text)?;
    let mut entries = parse_listing(&text)?;
    // 7zip+ omits Path for many single-member wrapper streams (bzip2, xz,
    // Zstandard, compress .Z), despite being able to extract them normally.
    if entries.is_empty() {
        if archive_format(&archive).as_deref() == Some("rpm") {
            let inner = rpm_member_name(&archive, password, cancel)?;
            entries.push(ArchiveEntry { key: inner, is_dir: false, size: 0, size_known: false, modified: None });
        } else if let Some(inner) = stream_member_name(&archive) {
            entries.push(ArchiveEntry { key: inner, is_dir: false, size: 0, size_known: false, modified: None });
        }
    }
    // Concatenated GZip streams can report only the first member's ISIZE to
    // `l -slt -ba` (real tgz fixture: 168 bytes reported, 7168 extracted).
    // That Size is not reliable enough for the output-size assertion.
    if archive_format(&archive).is_some_and(|f| matches!(f.as_str(), "gz" | "gzip" | "tar.gz" | "tgz" | "tpz")) {
        for entry in &mut entries {
            if !entry.is_dir { entry.size_known = false; }
        }
    }
    let mut encrypted = HashSet::new();
    for block in text.replace("\r\n", "\n").split("\n\n") {
        if block.lines().any(|l| l == "Encrypted = +") {
            if let Some(key) = block.lines().find_map(|l| l.strip_prefix("Path = ")) {
                encrypted.insert(key.to_owned());
            }
        }
    }
    Ok((entries, encrypted, iso9660_fallback))
}
/// The bundled 7zip+ can *list* a 7z with the proprietary extension codec ID
/// 04F71101 (Zstandard) but cannot decode its payload. Two independent public
/// test suites reproduce `7z t`: Unsupported Method. Reject before navigation
/// or extraction, while keeping ZIPX-ZSTD and tar.zst supported.
fn reject_unavailable_archive_methods(path: &Path, listing: &str) -> Result<(), String> {
    if archive_format(path).as_deref() == Some("7z")
        && listing.lines().any(|line| line.trim().eq_ignore_ascii_case("Method = 04F71101"))
    {
        return Err("This 7z archive uses Zstandard method 04F71101, which the bundled archive engine cannot extract.".into());
    }
    Ok(())
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
                        // C# IsArchiveEntryNewer: merge only when the entry is newer.
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
                        // Fixed: for files the number goes before the extension ("report (1).txt"). Previously it became "report.txt (1)" and broke the extension association.
                        let (stem, extension) = match name.rfind('.') {
                            Some(dot) if dot > 0 => (&name[..dot], &name[dot..]),
                            _ => (name.as_str(), ""),
                        };
                        let mut index = 1;
                        loop {
                            let candidate = parent.join(format!("{stem} ({index}){extension}"));
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
/// C# IsArchiveEntryNewer: whether an extracted entry (original time kept) is newer than the destination. Not when the time is unknown.
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
    let (entries, _, iso9660_fallback) = list_details(archive, password, cancel)?;
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
        if iso9660_fallback { cmd.arg("-tiso"); }
        password_arg(&mut cmd, password)?;
        cmd.arg("--").arg(archive);
        execute(cmd, cancel)?;
        inspect_tree(&stage)?;
        for entry in &entries {
            let target = stage.join(entry.key.replace('\\', "/"));
            ensure_plain_path(&target)?;
            let m = fs::symlink_metadata(&target)
                .map_err(|e| format!("Missing extracted entry {}: {e}", entry.key))?;
            if m.is_dir() != entry.is_dir || (!entry.is_dir && entry.size_known && m.len() != entry.size) {
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

/// C# FileSystemService.GetArchiveFormat(name): longest extensions first, split tails of 3+ digits allowed, .partN.rar, .rNN, .zNN, .aNN(N), .001.
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
/// C# BuiltInArchiveExtensions (+ split piece rules): formats NDR reads itself. Otherwise the external program side.
/// ALZ and EGG are added here because the bundled 7-Zip (UzuCore 7zip+) reads them. ACE is read by the C# version (SharpCompress sequential reading) but not by the bundled 7-Zip,
/// whose format list lacks it, so it is left out (extracted whole with an external archiver, `extract_whole_with_program`).
pub fn can_read_internally(path: &Path) -> bool {
    if !bundled_can_browse(path) { return false; }
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

/// Formats advertised by the original file-extension list are not necessarily
/// handlers in the shipped 7zip+ 26.05. `7z i` has no ACE, Lzip or `.uefi`
/// handler (UEFIc is `.scap`, UEFIf is `.uefif`, not `.uefi`). Refuse these
/// *before* opening a pane or an extraction destination dialog.
pub fn bundled_can_browse(path: &Path) -> bool {
    archive_format(path).is_some_and(|format| !matches!(format.as_str(), "ace" | "tar.lz" | "tlz" | "uefi"))
}

/// C# GetExtractFolderName: PathRemoveExtensionW(file name), removing only the last extension (sample.7z.001 -> sample.7z).
pub fn extract_folder_name(archive: &Path) -> String {
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match name.rfind('.') {
        Some(dot) if !name[dot..].contains(' ') => name[..dot].to_owned(),
        _ => name,
    }
}

/// C# CreateUniqueExtractFolder (FUN_00438cb0): "{parent}\{name}", then "{...}(2)", "(3)"... when it exists; only folders newly created with CreateDirectoryW.
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

/// C# GetArchiveBaseName (set name).
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

/// C# GetArchiveOutputFamilyFiles: files in the same folder with the same format and set name (by name).
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

/// User request (not in the original): add, delete and rename items inside an archive in place. Only single-volume zip, 7z, tar. The shipped 7zip+ can *create and list* WIM but `7z d` fails on it (exit code 2): don't advertise write support.
/// (not rar, alz, egg, iso, split archives, or single-file wrappers like gz).
pub fn can_modify(path: &Path) -> bool {
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !matches!(extension.as_str(), "zip" | "7z" | "tar") || !path.is_file() {
        return false;
    }
    // Split zips (.z01 ...) are not modified.
    !(extension == "zip" && path.with_extension("z01").is_file())
}

/// Copies the archive to a staging folder in the same folder, modifies the copy with 7-Zip, then swaps it with the original when done (on failure or cancel the original stays).
fn modify_archive(archive: &Path, cancel: &AtomicBool, run: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
    check_cancel(cancel)?;
    if !can_modify(archive) {
        return Err("This archive format cannot be changed in place".into());
    }
    let archive = fs::canonicalize(archive).map_err(|e| e.to_string())?;
    ensure_plain_path(&archive)?;
    let parent = archive.parent().ok_or("Archive has no folder")?.to_path_buf();
    let name = archive.file_name().ok_or("Archive has no name")?.to_owned();
    let stage = unique_path(&parent, "archive-edit");
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        let copy = stage.join(&name);
        fs::copy(&archive, &copy).map_err(|e| e.to_string())?;
        let mut permissions = fs::metadata(&copy).map_err(|e| e.to_string())?.permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&copy, permissions).map_err(|e| e.to_string())?;
        run(&copy)?;
        check_cancel(cancel)?;
        fs::rename(&copy, &archive).map_err(|e| e.to_string())
    })();
    let _ = fs::remove_dir_all(&stage);
    result
}

fn edit_command(verb: &str, archive: &Path, password: Option<&str>) -> Result<Command, String> {
    let mut command = command()?;
    command.args([verb, "-y", "-sccUTF-8"]);
    if let Some(p) = password.filter(|p| !p.is_empty()) {
        if p.contains('\0') || p.contains('\n') || p.contains('\r') {
            return Err("Invalid archive password".into());
        }
        command.arg(format!("-p{p}"));
    }
    command.arg("--").arg(archive);
    Ok(command)
}

fn entry_arg(key: &str) -> Result<String, String> {
    let key = key.trim_end_matches('/');
    validate_key(key)?;
    Ok(key.replace('/', "\\"))
}

/// Deletes archive items (folders with their contents) (7-Zip d).
pub fn delete_entries(archive: &Path, keys: &[String], password: Option<&str>, cancel: &AtomicBool) -> Result<(), String> {
    if keys.is_empty() {
        return Ok(());
    }
    let arguments = keys.iter().map(|k| entry_arg(k)).collect::<Result<Vec<_>, _>>()?;
    modify_archive(archive, cancel, |copy| {
        let mut command = edit_command("d", copy, password)?;
        command.args(&arguments);
        execute(command, cancel).map(|_| ())
    })
}

/// Renames an archive item (7-Zip rn; for a folder the paths of the items inside change too).
pub fn rename_entry(archive: &Path, from: &str, to: &str, password: Option<&str>, cancel: &AtomicBool) -> Result<(), String> {
    let (from, to) = (entry_arg(from)?, entry_arg(to)?);
    modify_archive(archive, cancel, |copy| {
        let mut command = edit_command("rn", copy, password)?;
        command.arg(&from).arg(&to);
        execute(command, cancel).map(|_| ())
    })
}

/// Choice when a name already exists while adding into an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddConflict {
    /// Adds as is (files are replaced, folders merge).
    Merge,
    /// Deletes the existing item (a folder with its contents) and adds.
    Replace,
    Skip,
    /// Adds under this name (asks again if that name exists too).
    Rename(String),
    Cancel,
}

/// Adds files and folders under the archive `prefix` ("folder/below/" or "" for the top) (7-Zip a). Existing names are replaced.
pub fn add_entries(archive: &Path, prefix: &str, sources: &[PathBuf], password: Option<&str>, cancel: &AtomicBool) -> Result<(), String> {
    add_entries_with(archive, prefix, sources, password, cancel, &mut |_, _| AddConflict::Merge).map(|_| ())
}

/// An explicit directory entry is not required: an archive with "dir/file.txt"
/// already has a "dir" for purposes of collision detection.
fn existing_archive_entry(entries: &[ArchiveEntry], key: &str) -> Option<ArchiveEntry> {
    let wanted = key.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let normal = |k: &str| k.replace('\\', "/").trim_end_matches('/').to_lowercase();
    entries.iter().find(|e| normal(&e.key) == wanted).cloned().or_else(|| {
        let prefix = format!("{wanted}/");
        entries.iter().any(|e| normal(&e.key).starts_with(&prefix)).then(|| ArchiveEntry {
            key: key.to_owned(), is_dir: true, size: 0, size_known: true, modified: None,
        })
    })
}

/// A merged folder must resolve *nested* name collisions too. Otherwise 7-Zip
/// silently replaces existing files, and a move could delete the skipped source.
/// Skipped children are reported by their original paths so callers can keep
/// the entire source folder if the transfer was only partially successful.
#[allow(clippy::too_many_arguments)]
fn stage_merged_archive_folder(
    source: &Path,
    target: &Path,
    key: &str,
    existing: &[ArchiveEntry],
    cancel: &AtomicBool,
    conflicts: &mut dyn FnMut(&Path, &ArchiveEntry) -> AddConflict,
    skipped: &mut Vec<PathBuf>,
    removals: &mut Vec<String>,
) -> Result<(), String> {
    check_cancel(cancel)?;
    ensure_plain_path(source)?;
    let metadata = fs::symlink_metadata(source).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err(format!("Unsafe archive source folder: {}", source.display()));
    }
    fs::create_dir(target).map_err(|e| e.to_string())?;
    for child in fs::read_dir(source).map_err(|e| e.to_string())? {
        check_cancel(cancel)?;
        let child = child.map_err(|e| e.to_string())?;
        let child_path = child.path();
        let metadata = fs::symlink_metadata(&child_path).map_err(|e| e.to_string())?;
        if is_reparse(&metadata) {
            return Err(format!("Archive source links are rejected: {}", child_path.display()));
        }
        let directory = metadata.is_dir();
        let mut name = child.file_name().to_str().ok_or("Archive source filename must be Unicode")?.to_owned();
        validate_key(&name)?;
        let mut merge_folder = false;
        let mut skip = false;
        loop {
            let child_key = format!("{key}/{name}");
            let Some(entry) = existing_archive_entry(existing, &child_key) else { break };
            match conflicts(&child_path, &entry) {
                AddConflict::Merge if entry.is_dir == directory => {
                    merge_folder = directory;
                    break;
                }
                AddConflict::Merge | AddConflict::Replace => {
                    removals.push(child_key);
                    break;
                }
                AddConflict::Skip => {
                    skip = true;
                    break;
                }
                AddConflict::Rename(new_name) => {
                    validate_key(&new_name)?;
                    if new_name.contains(['/', '\\']) {
                        return Err(format!("Unsafe archive entry: {new_name:?}"));
                    }
                    name = new_name;
                }
                AddConflict::Cancel => return Err("Archive add cancelled".into()),
            }
        }
        if skip {
            skipped.push(child_path);
            continue;
        }
        let staged = target.join(&name);
        if staged.exists() {
            return Err(format!("Duplicate archive source names: {}", staged.display()));
        }
        if merge_folder {
            stage_merged_archive_folder(&child_path, &staged, &format!("{key}/{name}"), existing, cancel, conflicts, skipped, removals)?;
        } else {
            copy_tree(&child_path, &staged, cancel)?;
        }
    }
    Ok(())
}

/// add_entries plus name conflict handling (`conflicts(source, existing item)`).
/// Builds that path shape in a staging folder and copies into it (source files are not touched), then passes only the top names.
pub fn add_entries_with(
    archive: &Path,
    prefix: &str,
    sources: &[PathBuf],
    password: Option<&str>,
    cancel: &AtomicBool,
    conflicts: &mut dyn FnMut(&Path, &ArchiveEntry) -> AddConflict,
) -> Result<Vec<PathBuf>, String> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    let prefix = prefix.trim_matches('/');
    if !prefix.is_empty() {
        validate_key(prefix)?;
    }
    // Existing items (a folder counts even without a separate folder entry when items below it exist).
    let existing = list_cancel(archive, password, cancel)?;
    let find = |key: &str| existing_archive_entry(&existing, key);
    let snapshot = unique_path(&std::env::temp_dir(), "archive-add");
    fs::create_dir(&snapshot).map_err(|e| e.to_string())?;
    let result = (|| {
        let base = if prefix.is_empty() { snapshot.clone() } else { snapshot.join(prefix.replace('/', "\\")) };
        fs::create_dir_all(&base).map_err(|e| e.to_string())?;
        let mut names = HashSet::new();
        let mut removals = Vec::new();
        let mut skipped = Vec::new();
        for source in sources {
            let name = source.file_name().ok_or("Cannot add a filesystem root")?;
            let mut text = name.to_str().ok_or("Archive source filename must be Unicode")?.to_owned();
            validate_key(&text)?;
            let source_dir = source.is_dir();
            let mut skip = false;
            let mut merge_folder = false;
            loop {
                let key = if prefix.is_empty() { text.clone() } else { format!("{prefix}/{text}") };
                let Some(entry) = find(&key) else { break };
                match conflicts(source, &entry) {
                    AddConflict::Merge if entry.is_dir == source_dir => {
                        merge_folder = source_dir;
                        break;
                    }
                    AddConflict::Merge | AddConflict::Replace => {
                        removals.push(key);
                        break;
                    }
                    AddConflict::Skip => {
                        skip = true;
                        break;
                    }
                    AddConflict::Rename(new_name) => {
                        validate_key(&new_name)?;
                        if new_name.contains(['/', '\\']) {
                            return Err(format!("Unsafe archive entry: {new_name:?}"));
                        }
                        text = new_name;
                    }
                    AddConflict::Cancel => return Err("Archive add cancelled".into()),
                }
            }
            if skip {
                skipped.push(source.clone());
                continue;
            }
            if !names.insert(text.to_lowercase()) {
                return Err("Duplicate archive source names".into());
            }
            if merge_folder {
                let key = if prefix.is_empty() { text.clone() } else { format!("{prefix}/{text}") };
                stage_merged_archive_folder(source, &base.join(&text), &key, &existing, cancel, conflicts, &mut skipped, &mut removals)?;
            } else {
                copy_tree(source, &base.join(&text), cancel)?;
            }
        }
        if names.is_empty() {
            return Ok(skipped);
        }
        let tops: Vec<String> = if prefix.is_empty() {
            names_in(&base)?
        } else {
            vec![prefix.split('/').next().unwrap_or(prefix).to_owned()]
        };
        let snapshot_abs = fs::canonicalize(&snapshot).map_err(|e| e.to_string())?;
        let removals = removals.iter().map(|k| entry_arg(k)).collect::<Result<Vec<_>, _>>()?;
        modify_archive(archive, cancel, |copy| {
            if !removals.is_empty() {
                let mut command = edit_command("d", copy, password)?;
                command.args(&removals);
                execute(command, cancel)?;
            }
            let mut command = edit_command("a", copy, password)?;
            command.current_dir(&snapshot_abs).args(&tops);
            execute(command, cancel).map(|_| ())
        })?;
        Ok(skipped)
    })();
    let _ = fs::remove_dir_all(&snapshot);
    result
}

/// Names directly under a folder.
fn names_in(folder: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for item in fs::read_dir(folder).map_err(|e| e.to_string())? {
        names.push(item.map_err(|e| e.to_string())?.file_name().to_string_lossy().into_owned());
    }
    Ok(names)
}

/// Creates an empty folder `name` under the archive `prefix`.
pub fn add_folder(archive: &Path, prefix: &str, name: &str, password: Option<&str>, cancel: &AtomicBool) -> Result<(), String> {
    validate_key(name)?;
    let holder = unique_path(&std::env::temp_dir(), "archive-folder");
    fs::create_dir_all(holder.join(name)).map_err(|e| e.to_string())?;
    let result = add_entries(archive, prefix, &[holder.join(name)], password, cancel);
    let _ = fs::remove_dir_all(&holder);
    result
}

/// C# GetArchiveFirstVolumePath: .partN.rar -> .part1.rar, .rNN -> .rar, .zNN -> .zip, .aNN -> .arj, .NNN -> .00...1 (only when it exists).
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

/// Listing (asks through passwords when the header needs a password). Also returns the chosen password.
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

/// Decision of C# GetAutoExtractLayoutAsync: whether there is a single top item and it is a folder.
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

/// C# GetArchiveSummary: (file count, folder count (including implied folders), unpacked size).
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
            if entry.size_known { bytes += entry.size; }
        }
    }
    (files, folders.len(), bytes)
}
/// C# ExtractEntryToTempAsync: extracts a single item (no wildcards) into %TEMP%\NDR\<new name> and returns that file path.
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
    // Fixed: when extraction fails (wrong password, cancel etc.) the created folder is removed. Previously a folder was left behind for every failed attempt.
    let result = (|| {
        let archive = fs::canonicalize(archive).map_err(|e| e.to_string())?;
        // Opening an item must use the same ISO9660 fallback as the archive
        // pane. For non-ISO archives avoid the extra listing pass.
        let iso9660_fallback = archive_format(&archive).as_deref() == Some("iso")
            && list_details(&archive, password, cancel)?.2;
        let mut cmd = command()?;
        cmd.args(["x", "-y", "-sccUTF-8", "-snl-", "-spd"]).arg(format!("-o{}", folder.display()));
        if iso9660_fallback { cmd.arg("-tiso"); }
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

/// Temp folder where archive items are extracted one by one (%TEMP%\NDR).
/// The original FUN_0040d9b8 uses %TEMP%\NexusFileTmp (created when missing); NDR keeps its own name so a running NexusFile
/// and its cleanup never share the folder.
pub fn temp_entry_root() -> PathBuf {
    std::env::temp_dir().join("NDR")
}

/// Fixed (the C# version and earlier Rust versions never removed them, so they piled up): removes ".ndr-entry-*" folders under `root` whose contents are all
/// older than `max_age`. Each is first renamed to ".ndr-trash-*" (skipped when it fails because of an open file),
/// and only renamed ones are deleted. Leftover ".ndr-trash-*" from a previous run are deleted too. Returns the number of folders removed.
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
        // With an unreadable item the age is unknown, so it is kept.
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

/// Shaped like C# ExtractExternalWithPasswordPromptAsync: on a password error asks again through `passwords` (None on cancel) and extracts again.
/// Returns (extracted path, password actually used).
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

/// C# ExtractExternalWithPasswordPromptAsync(external program, archive, destination): extracts the whole archive with an external program without reading the listing
/// (formats the bundled 7-Zip cannot read, e.g. .ace). Extracts into a staging folder in the same folder, checks it (inspect_tree), then moves it to the destination.
/// When the destination already has the name, it does not overwrite and ends with an error.
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

/// Line printed on standard output when an external archiver asks for a password (7-Zip syntax).
const EXTERNAL_PASSWORD_PROMPT: &str = "Enter password (will not be echoed):";

/// C# HasExternalArchivePasswordDiagnostic: the password prompt line on standard output, or a line starting with "ERROR: Wrong password" /
/// "ERROR: Data Error in encrypted file. Wrong password?", or exactly "ERROR: Can not open encrypted archive. Wrong password?".
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

/// C# RunExternalExtractWithPasswordPromptAsync: when the external program emits a password diagnostic, asks through `passwords` (empty or cancel ends) and extracts again with it.
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

/// C# ExtractArchiveWithProgress(requestedKeys): everything when keys is empty, otherwise only items with the same key or inside that folder (case-insensitive, full paths kept).
/// Progress is (items done, items chosen, key).
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
    if program.is_none() && !bundled_can_browse(archive) {
        return Err(format!("Archive format not supported by bundled 7-Zip: {}", archive.display()));
    }
    ensure_plain_path(destination)?;
    let archive = fs::canonicalize(archive).map_err(|e| e.to_string())?;
    let mut header_password = initial_password.map(str::to_owned);
    let (entries, encrypted, iso9660_fallback) = loop {
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
    let parents: Vec<String> = keys
        .iter()
        .map(|k| {
            let k = k.replace('\\', "/");
            let k = k.trim_end_matches('/');
            k.rfind('/').map(|i| k[..=i].to_owned()).unwrap_or_default()
        })
        .collect();
    let entries: Vec<ArchiveEntry> = entries
        .into_iter()
        .filter(|e| {
            let key = e.key.replace('\\', "/").trim_end_matches('/').to_lowercase();
            wanted.is_empty() || wanted.iter().any(|w| key == *w || key.starts_with(&format!("{w}/")))
        })
        .collect();
    let total = entries.len();
    progress(0, total, "");
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
        // Fixed: extracting items chosen in an archive subfolder recreated that subfolder path ("dir/sub/c.txt" -> destination\dir\sub\c.txt).
        // Now the parent path of chosen items is removed so they land directly under the destination (destination\c.txt). Full extraction (no keys) is unchanged.
        let placed = parents
            .iter()
            .filter(|p| !p.is_empty())
            .find_map(|p| relative.strip_prefix(p.as_str()))
            .unwrap_or(&relative)
            .to_owned();
        let target = aggregate.join(&placed);
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
                    if iso9660_fallback { cmd.arg("-tiso"); }
                    password_arg(&mut cmd, password.as_deref())?;
                    cmd.arg("--").arg(&archive).arg(&entry.key);
                }
                execute_capture(cmd, cancel, program.is_none())?;
                inspect_tree(&attempt)?;
                let source = attempt.join(&relative);
                let metadata = fs::symlink_metadata(&source)
                    .map_err(|e| format!("Missing decoded entry {}: {e}", entry.key))?;
                if !metadata.is_file() || (entry.size_known && metadata.len() != entry.size) {
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

/// 7-Zip diagnostic classification matching original FUN_00404890 (archive engine error number -> [RES] key). It reads 7-Zip text instead of the original engine number.
/// None when unknown (the 7-Zip text as is).
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

/// The name in a 7-Zip "ERROR: Missing volume : name" line (%s of the original ARC_CHANGEVOL). None when absent.
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
    #[test]
    fn merging_an_archive_folder_honors_nested_skips_without_losing_existing_files() {
        let p = super::tests::fixture();
        fs::create_dir_all(p.join("old/dir/inner")).unwrap();
        fs::create_dir_all(p.join("incoming/dir/inner")).unwrap();
        fs::write(p.join("old/dir/inner/locked.txt"), b"original").unwrap();
        fs::write(p.join("old/dir/inner/untouched.txt"), b"untouched").unwrap();
        fs::write(p.join("incoming/dir/inner/locked.txt"), b"replacement").unwrap();
        fs::write(p.join("incoming/dir/inner/new.txt"), b"added").unwrap();
        let cancel = AtomicBool::new(false);
        for extension in ["zip", "7z"] {
            let archive = p.join(format!("nested.{extension}"));
            create(&[p.join("old/dir")], &archive, None, &cancel).unwrap();
            let mut asked = Vec::new();
            let skipped = add_entries_with(
                &archive, "", &[p.join("incoming/dir")], None, &cancel,
                &mut |source, entry| {
                    asked.push((source.to_path_buf(), entry.key.replace('\\', "/")));
                    if entry.key.ends_with("locked.txt") { AddConflict::Skip } else { AddConflict::Merge }
                },
            ).unwrap();
            assert_eq!(skipped, [p.join("incoming/dir/inner/locked.txt")]);
            assert!(asked.iter().any(|(_, key)| key == "dir/inner/locked.txt"), "nested collision was not offered: {asked:?}");
            let out = p.join(format!("check-{extension}"));
            extract(&archive, &out, None, &cancel).unwrap();
            assert_eq!(fs::read(out.join("dir/inner/locked.txt")).unwrap(), b"original", "{extension}");
            assert_eq!(fs::read(out.join("dir/inner/untouched.txt")).unwrap(), b"untouched", "{extension}");
            assert_eq!(fs::read(out.join("dir/inner/new.txt")).unwrap(), b"added", "{extension}");
            assert_eq!(fs::read(p.join("incoming/dir/inner/locked.txt")).unwrap(), b"replacement");
        }
        fs::remove_dir_all(p).unwrap();
    }

    #[test]
    fn archive_entries_can_be_added_renamed_and_deleted_in_place() {
        let p = super::tests::fixture();
        fs::create_dir_all(p.join("src/dir/sub")).unwrap();
        fs::write(p.join("src/a.txt"), b"a").unwrap();
        fs::write(p.join("src/dir/sub/c.txt"), b"c").unwrap();
        let cancel = AtomicBool::new(false);
        let zip = p.join("t.zip");
        create(&[p.join("src/a.txt"), p.join("src/dir")], &zip, None, &cancel).unwrap();
        assert!(can_modify(&zip) && !can_modify(&p.join("x.rar")));
        let keys = |zip: &Path| {
            let mut keys: Vec<String> = list(zip, None).unwrap().into_iter().map(|e| e.key.replace('\\', "/")).collect();
            keys.sort();
            keys
        };
        // Add: at the top and under an inner folder.
        fs::write(p.join("new.txt"), b"new").unwrap();
        add_entries(&zip, "", &[p.join("new.txt")], None, &cancel).unwrap();
        add_entries(&zip, "dir/sub/", &[p.join("new.txt")], None, &cancel).unwrap();
        add_folder(&zip, "dir/", "empty", None, &cancel).unwrap();
        let after_add = keys(&zip);
        for key in ["new.txt", "dir/sub/new.txt", "dir/empty", "a.txt", "dir/sub/c.txt"] {
            assert!(after_add.iter().any(|k| k == key), "{key} in {after_add:?}");
        }
        assert!(p.join("new.txt").is_file(), "the source stays");
        // Same name: the item the callback receives, skip, new name, replace.
        fs::write(p.join("new.txt"), b"second").unwrap();
        let mut seen = Vec::new();
        add_entries_with(&zip, "", &[p.join("new.txt")], None, &cancel, &mut |source, entry| {
            seen.push((source.to_path_buf(), entry.key.replace('\\', "/"), entry.is_dir, entry.size, entry.modified.is_some()));
            AddConflict::Skip
        })
        .unwrap();
        assert_eq!(seen, vec![(p.join("new.txt"), "new.txt".to_owned(), false, 3, true)]);
        let mut asked = 0;
        add_entries_with(&zip, "", &[p.join("new.txt")], None, &cancel, &mut |_, entry| {
            asked += 1;
            // If the first new name exists too, it asks again.
            if asked == 1 { AddConflict::Rename("a.txt".into()) } else { assert_eq!(entry.key, "a.txt"); AddConflict::Rename("new (2).txt".into()) }
        })
        .unwrap();
        assert_eq!(asked, 2);
        let read = |key: &str| {
            let out = p.join(format!("read-{}", key.replace('/', "-")));
            let _ = fs::remove_dir_all(&out);
            extract_entries_with_program(&zip, &out, &[key.into()], None, &cancel, &mut |_, _| ConflictChoice::Overwrite, &mut |_, _| None, None, &mut |_, _, _| {}).unwrap();
            fs::read(out.join(key.rsplit('/').next().unwrap())).unwrap()
        };
        assert_eq!(read("new.txt"), b"new", "skipped");
        assert_eq!(read("new (2).txt"), b"second");
        // Replace folder: items inside the existing folder disappear, only the new folder's contents remain.
        fs::create_dir_all(p.join("other/dir")).unwrap();
        fs::write(p.join("other/dir/only.txt"), b"only").unwrap();
        add_entries_with(&zip, "", &[p.join("other/dir")], None, &cancel, &mut |_, entry| {
            assert!(entry.is_dir);
            AddConflict::Replace
        })
        .unwrap();
        let replaced = keys(&zip);
        assert!(replaced.iter().any(|k| k == "dir/only.txt") && !replaced.iter().any(|k| k.starts_with("dir/sub")), "{replaced:?}");
        assert!(add_entries_with(&zip, "", &[p.join("new.txt")], None, &cancel, &mut |_, _| AddConflict::Cancel).is_err());
        // Back to the earlier state (the tests below assume dir/sub exists).
        add_entries_with(&zip, "", &[p.join("src/dir")], None, &cancel, &mut |_, _| AddConflict::Replace).unwrap();
        add_entries(&zip, "dir/sub/", &[p.join("new.txt")], None, &cancel).unwrap();
        add_folder(&zip, "dir/", "empty", None, &cancel).unwrap();
        // Rename: for a folder, the items inside too.
        rename_entry(&zip, "dir/sub", "dir/moved", None, &cancel).unwrap();
        let after_rename = keys(&zip);
        assert!(after_rename.iter().any(|k| k == "dir/moved/c.txt") && !after_rename.iter().any(|k| k.starts_with("dir/sub")), "{after_rename:?}");
        // Delete: a folder with its contents; other items with similar names stay.
        delete_entries(&zip, &["dir/moved".into(), "a.txt".into()], None, &cancel).unwrap();
        let after_delete = keys(&zip);
        assert!(!after_delete.iter().any(|k| k.starts_with("dir/moved") || k == "a.txt"), "{after_delete:?}");
        assert!(after_delete.iter().any(|k| k == "new.txt") && after_delete.iter().any(|k| k == "dir/empty"), "{after_delete:?}");
        // Unmodifiable formats and bad names fail without touching the original.
        assert!(delete_entries(&zip, &["../x".into()], None, &cancel).is_err());
        assert_eq!(keys(&zip), after_delete);
        fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn renamed_extracted_files_keep_their_extension_last() {
        let p = super::tests::fixture();
        fs::write(p.join("report.txt"), b"new").unwrap();
        let cancel = AtomicBool::new(false);
        create(&[p.join("report.txt")], &p.join("r.7z"), None, &cancel).unwrap();
        fs::create_dir(p.join("out")).unwrap();
        fs::write(p.join("out/report.txt"), b"old").unwrap();
        extract_interactive(&p.join("r.7z"), &p.join("out"), None, &cancel, &mut |_, _| ConflictChoice::Rename, &mut |_, _| None).unwrap();
        assert_eq!(fs::read(p.join("out/report.txt")).unwrap(), b"old");
        assert_eq!(fs::read(p.join("out/report (1).txt")).unwrap(), b"new");
        assert!(!p.join("out/report.txt (1)").exists());
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
        // Add a sequence number so parallel tests that get the same time value do not share a folder.
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
    /// 7-Zip -bsp1 progress text: percentage and file name. Bundled 7-Zip archiving reports progress before it finishes.
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
        // Fixed: when extraction fails, as for a missing item, the created folder is not left behind.
        let before: Vec<_> = fs::read_dir(temp_entry_root()).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(extract_entry_to_temp(&output, None, "missing.txt", &cancel).is_err());
        let after: Vec<_> = fs::read_dir(temp_entry_root()).unwrap().flatten().map(|e| e.file_name()).filter(|n| !before.contains(n)).collect();
        assert!(after.iter().all(|n| !n.to_string_lossy().starts_with(&format!(".ndr-entry-{}-", std::process::id()))), "{after:?}");
        fs::remove_dir_all(root).unwrap();
    }

    /// Fixed: only old ".ndr-entry-*" are removed; recent ones, other names and ones that cannot be renamed because they are open stay.
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
        // By the current time everything is recent: only the leftover trash folders are removed.
        assert_eq!(clean_stale_temp_entries(&root, week, SystemTime::now()), 1);
        assert!(root.join(".ndr-entry-1-0").is_dir() && !root.join(".ndr-trash-9-0").exists());
        // 8 days later: the folder with an open file (no delete sharing) and the folder with another name stay.
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

    /// Bundled 7-Zip (UzuCore 7zip+α 26.05): ALZ/EGG create, list and extract (including Korean names). 7-Zip refuses adding and the original stays.
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
        // ZIP: create anew; adding keeps existing items.
        let zip = root.join("out.zip");
        create_configured(&[root.join("src/a.txt")], &zip, &options("zip", false), &cancel).unwrap();
        create_configured(&[root.join("src/b.txt"), root.join("src/dir")], &zip, &options("zip", true), &cancel).unwrap();
        assert_eq!(keys(&zip), ["a.txt", "b.txt", "dir/deep/c.txt"]);
        create_configured(&[root.join("src/b.txt")], &zip, &options("zip", false), &cancel).unwrap();
        assert_eq!(keys(&zip), ["b.txt"], "without append the archive is replaced");
        // TAR.GZ: create and add.
        let tgz = root.join("out.tar.gz");
        create_configured(&[root.join("src/a.txt")], &tgz, &options("tar.gz", false), &cancel).unwrap();
        create_configured(&[root.join("src/dir")], &tgz, &options("tar.gz", true), &cancel).unwrap();
        let unpacked = root.join("tgz");
        extract(&tgz, &unpacked, None, &cancel).unwrap();
        let inner = fs::read_dir(&unpacked).unwrap().filter_map(Result::ok).map(|e| e.path()).next().unwrap();
        assert_eq!(keys(&inner), ["a.txt", "dir/deep/c.txt"]);
        // Extract only chosen keys (a folder with everything inside, full paths kept), with progress.
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
        // Naming rules.
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
        // The bundled 7-Zip cannot read ACE (not in its format list): neither internal reading nor entering.
        assert!(!can_read_internally(Path::new("a.ace")) && !bundled_can_browse(Path::new("a.ace")) && bundled_can_browse(Path::new("a.iso")));
        fs::remove_dir_all(root).unwrap();
    }

    /// Exercise every format exposed by NDR's create dialog plus the two
    /// additional bundled ALZ/EGG writers and WIM (browse-only). These are
    /// real archives created, listed, and decoded by the shipped 7zip+.
    #[test]
    fn supported_archive_formats_round_trip_and_wim_is_not_editable() {
        let root = super::tests::fixture();
        let source = root.join("fixture.txt");
        fs::write(&source, b"ndr-format-verified").unwrap();
        let cancel = AtomicBool::new(false);
        for extension in ["zip", "7z", "tar", "tar.gz", "tar.bz2", "alz", "egg", "wim"] {
            let archive = root.join(format!("fixture.{extension}"));
            let options = crate::commands::ArchiveOptions {
                extension: extension.into(), level: 5, password: String::new(), volume_bytes: None,
                recursive: true, append: false,
            };
            create_configured(&[source.clone()], &archive, &options, &cancel).unwrap_or_else(|e| panic!("{extension} creation: {e}"));
            assert!(bundled_can_browse(&archive), "{extension} browse should be advertised");
            assert!(!list(&archive, None).unwrap_or_else(|e| panic!("{extension} list: {e}")).is_empty(), "{extension} listing");
            let dest = root.join(format!("extracted-{extension}"));
            extract(&archive, &dest, None, &cancel).unwrap_or_else(|e| panic!("{extension} extract: {e}"));
            assert!(dest.is_dir() && fs::read_dir(&dest).unwrap().next().is_some(), "{extension} extraction should materialize at least one member");
            assert_eq!(can_modify(&archive), matches!(extension, "zip" | "7z" | "tar"), "{extension} edit support");
        }
        let renamed_zipx = root.join("fixture.zipx");
        fs::copy(root.join("fixture.zip"), &renamed_zipx).unwrap();
        assert_eq!(list(&renamed_zipx, None).unwrap()[0].key, "fixture.txt");
        assert!(!can_modify(&renamed_zipx));
        // BZip2 has neither Path nor Size in `7z l -slt -ba`. Give the stream
        // its derived name, then actually decode it (including .tbz2 alias).
        let alias = root.join("aliased.tbz2");
        fs::copy(root.join("fixture.tar.bz2"), &alias).unwrap();
        let streamed = list(&alias, None).unwrap();
        assert_eq!(streamed[0].key, "aliased.tar");
        assert!(!streamed[0].size_known && streamed[0].size == 0);
        let extracted = root.join("alias-extracted");
        extract(&alias, &extracted, None, &cancel).unwrap();
        assert!(extracted.join("aliased.tar").is_file());
        fs::remove_dir_all(root).unwrap();
    }

    /// All original advertised file extensions get a decision, but archive
    /// names absent from `7z i` must not start a job or write a destination.
    #[test]
    fn every_declared_archive_extension_has_a_capability_and_unsupported_formats_stop_early() {
        let unsupported = ["ace", "tar.lz", "tlz", "uefi"];
        for extension in ARCHIVE_EXTENSIONS {
            let sample = PathBuf::from(format!("sample{extension}"));
            assert_eq!(archive_format(&sample).as_deref(), Some(&extension[1..]), "recognized extension: {extension}");
            assert_eq!(bundled_can_browse(&sample), !unsupported.contains(&&extension[1..]), "7zip+ support: {extension}");
        }
        let root = super::tests::fixture();
        for extension in unsupported {
            let source = root.join(format!("invalid.{extension}"));
            fs::write(&source, b"not a supported archive").unwrap();
            assert!(!can_read_internally(&source));
            let dest = root.join(format!("no-output-{extension}"));
            assert!(list(&source, None).unwrap_err().contains("not supported"));
            assert!(extract(&source, &dest, None, &AtomicBool::new(false)).is_err());
            assert!(!dest.exists(), "{extension}: failed archive must not create output directory");
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_web_corpus_rejects_7z_zstd_codec_before_opening_but_keeps_zipx_zstd() {
        let listing = "Path = a.txt\nSize = 3\nMethod = 04F71101\n";
        assert!(reject_unavailable_archive_methods(Path::new("a.7z"), listing)
            .unwrap_err().contains("cannot extract"));
        assert!(reject_unavailable_archive_methods(Path::new("a.zipx"), listing).is_ok());
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/archive-web-corpus/samples");
        for name in ["test_read_format_7zip_zstd.7z", "test_read_format_7zip_solid_zstd.7z", "sharpcompress/7Zip.ZSTD.7z"] {
            let archive = corpus.join(name);
            if archive.is_file() {
                let error = list(&archive, None).unwrap_err();
                assert!(error.contains("04F71101"), "{name}: {error}");
            }
        }
        for name in ["test_read_format_zip_zstd.zipx", "sharpcompress/WinZip27_ZSTD.zipx"] {
            let archive = corpus.join(name);
            if archive.is_file() {
                assert!(!list(&archive, None).unwrap_or_else(|e| panic!("{name}: {e}")).is_empty());
            }
        }
    }

    #[test]
    fn single_stream_archives_expose_real_member_names_for_selective_copy() {
        for (name, expected) in [
            ("data.tar.zst", "data.tar"), ("data.tar.xz", "data.tar"),
            ("data.tar.bz2", "data.tar"), ("data.txz", "data.tar"),
            ("data.tbz", "data.tar"), ("data.tgz", "data.tar"),
            ("image.iso.Z", "image.iso"), ("plain.xz", "plain"),
            ("plain.zstd", "plain"),
        ] {
            assert_eq!(stream_member_name(Path::new(name)).as_deref(), Some(expected), "{name}");
        }
        assert!(stream_member_name(Path::new("invalid.7z")).is_none());
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/archive-web-corpus/samples");
        for (name, expected) in [
            ("test_compat_xz_1.txz", "test_compat_xz_1.tar"),
            ("test_compat_zstd_1.tar.zst", "test_compat_zstd_1.tar"),
            ("test_read_format_iso.iso.Z", "test_read_format_iso.iso"),
            ("test_read_format_cpio_svr4_bzip2_rpm.rpm", "rpmsample-1.0.0-1.noarch.cpio"),
        ] {
            let sample = corpus.join(name);
            if sample.is_file() {
                assert!(list(&sample, None).unwrap_or_else(|e| panic!("{name}: {e}"))
                    .iter().any(|e| e.key == expected), "{name}: missing {expected}");
            }
        }
        let concatenated = corpus.join("test_compat_gzip_1.tgz");
        if concatenated.is_file() {
            let items = list(&concatenated, None).unwrap();
            assert!(items.iter().any(|e| e.key == "test-split.taraa" && !e.size_known),
                "7zip+ can underreport concatenated GZip uncompressed sizes");
        }
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
        // Whole extraction without a listing (C# ExtractExternalWithPasswordPromptAsync): the bundled 7-Zip and an extract format in place of the external program.
        let program = ExternalProgram { program: seven_zip_path().unwrap(), create_template: None, extract_template: Some("x {archive} -o{destination} -y {keys}".into()) };
        let destination = root.join("pack");
        extract_whole_with_program(&archive, &destination, None, &program, &cancel).unwrap();
        assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"top");
        assert_eq!(fs::read(destination.join("inner/b.txt")).unwrap(), b"inside");
        // No staging folder is left behind.
        assert!(!fs::read_dir(&root).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".ndr-whole-")));
        // When the name already exists, it does not overwrite and fails.
        assert!(extract_whole_with_program(&archive, &destination, None, &program, &cancel).is_err());
        assert_eq!(fs::read(destination.join("a.txt")).unwrap(), b"top");
        // C# RunExternalExtractWithPasswordPromptAsync: on a password diagnostic (7-Zip syntax) asks and extracts again with that password. Wrong asks again, cancel ends.
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
