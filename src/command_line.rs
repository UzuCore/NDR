//! C# NexusFileCommandService (command parsing, cd folder lookup), NexusFileCommandHistoryService (command history),
//! NexusFileExecutableHistoryService (executable history "name=path"). Calls the shlwapi/kernel32 functions directly.
use std::path::Path;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

/// C# NexusFileCommandKind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandKind {
    Local,
    ChangeDirectory,
    Ftp,
    Http,
}

/// C# NexusFileCommand(FileName, Arguments, Kind).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedCommand {
    pub file_name: String,
    pub arguments: String,
    pub kind: CommandKind,
}

/// lstrcmpiW(a, b).
pub fn lstrcmpi(a: &str, b: &str) -> i32 {
    // SAFETY: both buffers are NUL-terminated.
    unsafe { windows_sys::Win32::Globalization::lstrcmpiW(wide(a).as_ptr(), wide(b).as_ptr()) }
}

/// Parse: PathGetArgsW gives the arguments, PathRemoveArgsW the program name. "ftp://" or "http://" (case-sensitive) means FTP/HTTP, the name "cd" changes folder.
pub fn parse(command: &str) -> ParsedCommand {
    let mut buffer = wide(command);
    // SAFETY: PathGetArgsW returns a pointer into the NUL-terminated buffer; PathRemoveArgsW edits it in place.
    let arguments = unsafe {
        let start = windows_sys::Win32::UI::Shell::PathGetArgsW(buffer.as_ptr());
        let offset = start.offset_from(buffer.as_ptr()).max(0) as usize;
        from_wide(&buffer[offset.min(buffer.len())..])
    };
    // SAFETY: as above; the buffer keeps its terminator.
    unsafe { windows_sys::Win32::UI::Shell::PathRemoveArgsW(buffer.as_mut_ptr()) };
    let file_name = from_wide(&buffer);
    let kind = if file_name.starts_with("ftp://") {
        CommandKind::Ftp
    } else if file_name.starts_with("http://") {
        CommandKind::Http
    } else if lstrcmpi(&file_name, "cd") == 0 {
        CommandKind::ChangeDirectory
    } else {
        CommandKind::Local
    };
    ParsedCommand { file_name, arguments, kind }
}

/// TryResolveDirectory: the whole argument (quotes not stripped) as a full path relative to the working folder, only when it is an existing folder.
pub fn resolve_directory(argument: &str, working_directory: &Path) -> Option<std::path::PathBuf> {
    if argument.is_empty() || argument.contains('\0') {
        return None;
    }
    let resolved = std::path::absolute(working_directory.join(argument)).ok()?;
    resolved.is_dir().then_some(resolved)
}

/// CompareStringW(LOCALE_USER_DEFAULT, NORM_IGNORECASE, ...) == CSTR_EQUAL.
fn same_text(a: &str, b: &str) -> bool {
    let (x, y): (Vec<u16>, Vec<u16>) = (a.encode_utf16().collect(), b.encode_utf16().collect());
    // SAFETY: explicit lengths; no terminators needed.
    unsafe { windows_sys::Win32::Globalization::CompareStringW(0x400, 1, x.as_ptr(), x.len() as i32, y.as_ptr(), y.len() as i32) == 2 }
}

/// RememberLocalCommand: unless equal to the top line (lstrcmpiW), removes existing lines of the same length and text (case-insensitive) and puts it on top.
/// For local commands, when over 20 lines only the oldest line is removed.
pub fn remember_local_command(history: &mut Vec<String>, command: &str, apply_local_limit: bool) {
    if !command.is_empty() && (history.is_empty() || lstrcmpi(&history[0], command) != 0) {
        let length = command.encode_utf16().count();
        if let Some(existing) = history.iter().position(|item| item.encode_utf16().count() == length && same_text(item, command)) {
            history.remove(existing);
        }
        history.insert(0, command.to_owned());
    }
    if apply_local_limit && history.len() > 20 {
        history.pop();
    }
}

/// NexusFileExecutableHistoryService.Exists: PathFileExistsW (full path relative to the working folder).
pub fn executable_exists(path: &str, working_directory: Option<&Path>) -> bool {
    if path.is_empty() || path.contains('\0') {
        return false;
    }
    let full = match working_directory {
        Some(directory) => match std::path::absolute(directory.join(path)) {
            Ok(full) => full.to_string_lossy().into_owned(),
            Err(_) => return false,
        },
        None => path.to_owned(),
    };
    // SAFETY: NUL-terminated path.
    unsafe { windows_sys::Win32::UI::Shell::PathFileExistsW(wide(&full).as_ptr()) != 0 }
}

/// FindCandidates: only for local commands, the "name=path" lines with the same name (lstrcmpiW) and an existing path (in order).
pub fn find_candidates(rows: &[String], command: &str, working_directory: Option<&Path>) -> Vec<String> {
    let parsed = parse(command);
    if parsed.kind != CommandKind::Local {
        return vec![];
    }
    rows.iter()
        .filter_map(|row| {
            let (name, path) = row.split_once('=')?;
            (lstrcmpi(name, &parsed.file_name) == 0 && executable_exists(path, working_directory)).then(|| path.to_owned())
        })
        .collect()
}

/// PruneMissing: removes lines without '=' or with a missing path.
pub fn prune_missing(rows: &mut Vec<String>, working_directory: Option<&Path>) {
    rows.retain(|row| row.split_once('=').is_some_and(|(_, path)| executable_exists(path, working_directory)));
}

/// RememberOpenedExecutable: .exe/.lnk only. An existing same path (lstrcmpiW) moves to the top, otherwise "name without extension=path" is appended.
/// Original FUN_00479db0: compares each row's value (ValueFromIndex, lstrcmpiW); found = Delete + Insert(0), else Add("name=path") with the
/// name from PathFindFileNameW + PathRemoveExtensionW.
pub fn remember_opened_executable(rows: &mut Vec<String>, path: &str) {
    let buffer = wide(path);
    // SAFETY: both functions return pointers into the NUL-terminated buffer.
    let (extension, file_name) = unsafe {
        let ext = windows_sys::Win32::UI::Shell::PathFindExtensionW(buffer.as_ptr());
        let name = windows_sys::Win32::UI::Shell::PathFindFileNameW(buffer.as_ptr());
        let at = |p: *const u16| from_wide(&buffer[(p.offset_from(buffer.as_ptr()).max(0) as usize).min(buffer.len())..]);
        (at(ext), at(name))
    };
    if lstrcmpi(&extension, ".exe") != 0 && lstrcmpi(&extension, ".lnk") != 0 {
        return;
    }
    if let Some(index) = rows.iter().position(|row| row.split_once('=').is_some_and(|(_, known)| lstrcmpi(known, path) == 0)) {
        let existing = rows.remove(index);
        rows.insert(0, existing);
        return;
    }
    let mut name = wide(&file_name);
    // SAFETY: PathRemoveExtensionW edits the NUL-terminated buffer in place.
    unsafe { windows_sys::Win32::UI::Shell::PathRemoveExtensionW(name.as_mut_ptr()) };
    rows.push(format!("{}={path}", from_wide(&name)));
}

/// C# ExternalOpenTemplateToken + QuoteArgument: {files} (all), {file}, {name}, {dir} (case-insensitive) become quoted values.
pub fn expand_open_template(template: &str, paths: &[String]) -> String {
    let first = paths.first().map(String::as_str).unwrap_or("");
    let first_path = Path::new(first);
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let token = ["{files}", "{file}", "{name}", "{dir}"].into_iter().find(|t| tail.len() >= t.len() && tail[..t.len()].eq_ignore_ascii_case(t));
        match token {
            Some(t) => {
                let value = match t {
                    "{files}" => paths.iter().map(|p| quote_argument(p)).collect::<Vec<_>>().join(" "),
                    "{file}" => quote_argument(first),
                    "{name}" => quote_argument(&first_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
                    _ => quote_argument(&first_path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default()),
                };
                out.push_str(&value);
                rest = &tail[t.len()..];
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// QuoteArgument (CommandLineToArgvW rules).
pub fn quote_argument(value: &str) -> String {
    let mut result = String::from("\"");
    let mut slashes = 0;
    for ch in value.chars() {
        if ch == '\\' {
            slashes += 1;
            continue;
        }
        if ch == '"' {
            result.push_str(&"\\".repeat(slashes * 2 + 1));
            result.push('"');
        } else {
            result.push_str(&"\\".repeat(slashes));
            result.push(ch);
        }
        slashes = 0;
    }
    result.push_str(&"\\".repeat(slashes * 2));
    result.push('"');
    result
}

/// NexusFileMacroService.ExpandSetting: environment variables, then $(PROGRAMDIR) and $(PROGRAMDRIVE) (case-insensitive). Unknown $(name) stays.
pub fn expand_setting(value: &str) -> String {
    let expanded = crate::native::expand_environment(value);
    let program = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
    let directory = program.to_string_lossy().trim_end_matches('\\').to_owned();
    let drive = program.components().next().map(|c| {
        let mut root = c.as_os_str().to_string_lossy().into_owned();
        if !root.ends_with('\\') {
            root.push('\\');
        }
        root
    }).unwrap_or_default();
    let mut out = String::new();
    let mut rest = expanded.as_str();
    while let Some(start) = rest.find("$(") {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 2..];
        let name_length = tail.char_indices().take_while(|(i, c)| if *i == 0 { c.is_ascii_alphabetic() } else { c.is_ascii_alphanumeric() || *c == '_' }).count();
        if name_length > 0 && tail[name_length..].starts_with(')') {
            let name = &tail[..name_length];
            if name.eq_ignore_ascii_case("PROGRAMDIR") {
                out.push_str(&directory);
            } else if name.eq_ignore_ascii_case("PROGRAMDRIVE") {
                out.push_str(&drive);
            } else {
                out.push_str(&rest[start..start + 2 + name_length + 1]);
            }
            rest = &tail[name_length + 1..];
        } else {
            out.push_str("$(");
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_follow_nexusfilecommandservice() {
        assert_eq!(parse("notepad.exe a.txt b"), ParsedCommand { file_name: "notepad.exe".into(), arguments: "a.txt b".into(), kind: CommandKind::Local });
        assert_eq!(parse("\"C:\\Program Files\\x.exe\" -v").file_name, "\"C:\\Program Files\\x.exe\"", "PathRemoveArgsW keeps the quotes");
        assert_eq!(parse("CD ..").kind, CommandKind::ChangeDirectory);
        assert_eq!(parse("ftp://host/dir").kind, CommandKind::Ftp);
        assert_eq!(parse("FTP://host").kind, CommandKind::Local, "the scheme test is case-sensitive");
        assert_eq!(parse("http://example.invalid").kind, CommandKind::Http);
        let mut history = vec!["dir".to_owned(), "Notepad".to_owned()];
        remember_local_command(&mut history, "notepad", true);
        assert_eq!(history, ["notepad", "dir"], "the same command (ignoring case) moves to the top");
        remember_local_command(&mut history, "NOTEPAD", true);
        assert_eq!(history, ["notepad", "dir"], "equal to the top: nothing changes");
        let mut long: Vec<String> = (0..21).map(|i| format!("c{i}")).collect();
        remember_local_command(&mut long, "new", true);
        assert_eq!((long.len(), long[0].as_str(), long.last().unwrap().as_str()), (21, "new", "c19"), "only one oldest row goes");
        let root = std::env::temp_dir().join(format!("ndr-cmd-{}", std::process::id()));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("tool.exe"), b"").unwrap();
        assert_eq!(resolve_directory("sub", &root), Some(root.join("sub")));
        assert_eq!(resolve_directory("missing", &root), None);
        let mut rows = vec![];
        let exe = root.join("tool.exe").to_string_lossy().into_owned();
        remember_opened_executable(&mut rows, &exe);
        remember_opened_executable(&mut rows, &root.join("a.txt").to_string_lossy());
        assert_eq!(rows, [format!("tool={exe}")], "only .exe/.lnk");
        rows.push("other=C:\\missing\\other.exe".into());
        remember_opened_executable(&mut rows, &exe.to_uppercase());
        assert_eq!(rows[0], format!("tool={exe}"), "a known path is promoted, not added");
        assert_eq!(find_candidates(&rows, "TOOL -x", None), [exe.clone()]);
        assert!(find_candidates(&rows, "cd tool", None).is_empty());
        prune_missing(&mut rows, None);
        assert_eq!(rows.len(), 1);
        assert_eq!(expand_open_template("{FILE} {files} {name} {dir} {x}", &["C:\\a b\\c.txt".into(), "d.txt".into()]), "\"C:\\a b\\c.txt\" \"C:\\a b\\c.txt\" \"d.txt\" \"c.txt\" \"C:\\a b\" {x}");
        assert_eq!(quote_argument("a\\\"b\\"), "\"a\\\\\\\"b\\\\\"");
        assert!(!expand_setting("$(PROGRAMDIR)\\x").contains("$("));
        assert_eq!(expand_setting("$(OTHER)"), "$(OTHER)");
        std::fs::remove_dir_all(root).unwrap();
    }
}
