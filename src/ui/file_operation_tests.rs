//! C# FileConflictWindow, FileOperationProgressWindow, FileOperationErrorDialog, ReportFileOperationResult, 삭제 결과,
//! TryExternalCopyAsync를 Rust UI에서 확인한다. 한국어만(영어 글은 같은 경로). 격리된 임시 폴더만 쓰고, 외부 프로그램은
//! 가짜 실행 함수(program_runner)로 바꾼다. 휴지통은 쓰지 않는다.
use super::original_dialogs::{DeleteWarningChoice, DialogKind, DialogResult};
use super::*;
use ndr::msgbox::IDYES;
use std::fs;
use std::sync::Mutex;

static EXTERNAL: Mutex<Vec<(String, Vec<String>, String)>> = Mutex::new(Vec::new());

/// 가짜 외부 복사 프로그램: (프로그램, 인수, 목록 파일 내용)을 남기고 0으로 끝난다.
fn fake_runner(program: &Path, args: &[String], _: &Path) -> Result<i32, String> {
    let list = args
        .iter()
        .find_map(|a| {
            let path = a.strip_prefix('*').unwrap_or(a);
            (path.ends_with(".txt") || path.ends_with(".lst")).then(|| fs::read(path).ok()).flatten()
        })
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    EXTERNAL.lock().unwrap().push((program.to_string_lossy().into_owned(), args.to_vec(), list));
    Ok(0)
}

fn fixture(label: &str) -> (PathBuf, egui::Context, App) {
    let root = std::env::temp_dir().join(format!("ndr-fileop-{}-{}-{label}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let (left, right) = (root.join("left"), root.join("right"));
    fs::create_dir_all(left.join("folder")).unwrap();
    fs::create_dir_all(right.join("folder")).unwrap();
    fs::write(left.join("folder").join("x.txt"), b"x").unwrap();
    fs::write(left.join("a.txt"), b"resumable payload").unwrap();
    fs::write(right.join("a.txt"), b"resumable").unwrap();
    fs::write(left.join("b.txt"), b"bee").unwrap();
    fs::write(left.join("locked.txt"), b"locked").unwrap();
    let ctx = egui::Context::default();
    let options = Options { profile: Some(root.join("profile")), left: Some(left), right: Some(right), language: Some("ko".into()), ..Default::default() };
    let mut app = App::from_context(&ctx, options);
    app.clipboard_writer = |_| Ok(());
    app.shell_runner = |_, _, _| Ok(());
    app.program_runner = fake_runner;
    wait(&mut app, &ctx, |app| app.panes.iter().all(|p| !p.loading));
    (root, ctx, app)
}
fn wait(app: &mut App, ctx: &egui::Context, done: impl Fn(&App) -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !done(app) {
        assert!(Instant::now() < end, "timeout: status={:?} job={:?} dialogs={} boxes={:?}", app.status, app.job.as_ref().map(|j| (&j.title, j.finished, &j.errors, &j.fatal)), app.original_dialogs.len(), app.msgboxes.iter().map(|m| m.text.clone()).collect::<Vec<_>>());
        app.poll(ctx);
        thread::sleep(Duration::from_millis(5));
    }
}
fn finished(app: &App) -> bool {
    app.job.as_ref().is_none_or(|j| j.finished) && app.panes.iter().all(|p| !p.loading)
}
fn select(app: &mut App, names: &[&str]) {
    assert!(app.panes.iter().all(|p| !p.loading), "select after the listing");
    let pane = &mut app.panes[0];
    pane.selected = names.iter().map(|n| pane.entries.iter().position(|e| e.name == *n).unwrap()).collect();
}
fn conflict(app: &App) -> Option<&super::file_operation::ConflictForm> {
    app.original_dialogs.last().and_then(|d| match &d.kind {
        DialogKind::FileConflict(form) => Some(form.as_ref()),
        _ => None,
    })
}
fn copy_to_other(app: &mut App, ctx: &egui::Context, names: &[&str]) {
    select(app, names);
    let active = app.active;
    app.copy_to_other(active, false, ctx);
    app.answer_message_box(IDYES, ctx);
}
fn draw(app: &mut App, ctx: &egui::Context) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        app.file_progress_ui(ctx);
        app.original_dialog_ui(ctx);
        app.msgbox_ui(ctx);
    });
}
fn draw_with(app: &mut App, ctx: &egui::Context, events: Vec<egui::Event>) {
    let raw = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1400.0, 850.0))), events, ..Default::default() };
    let _ = ctx.run(raw, |ctx| {
        app.file_progress_ui(ctx);
        app.original_dialog_ui(ctx);
        app.msgbox_ui(ctx);
    });
}
fn press(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }, egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers }]
}

/// VCL CM_DIALOGCHAR: TfFileExists의 "덮어쓰기(&W)"는 W만 쳐도, "건너뛰기(&S)"는 Alt+S로도 눌린다.
#[test]
fn file_exists_buttons_answer_their_access_keys() {
    let (root, ctx, mut app) = fixture("accesskeys");
    let right = root.join("right");
    fs::write(right.join("b.txt"), b"old").unwrap();
    copy_to_other(&mut app, &ctx, &["b.txt"]);
    wait(&mut app, &ctx, |app| conflict(app).is_some());
    draw(&mut app, &ctx);
    draw_with(&mut app, &ctx, press(egui::Key::W, egui::Modifiers::NONE));
    wait(&mut app, &ctx, finished);
    assert_eq!(fs::read(right.join("b.txt")).unwrap(), b"bee", "W = Overwrite");
    fs::write(right.join("b.txt"), b"old").unwrap();
    copy_to_other(&mut app, &ctx, &["b.txt"]);
    wait(&mut app, &ctx, |app| conflict(app).is_some());
    draw(&mut app, &ctx);
    draw_with(&mut app, &ctx, press(egui::Key::S, egui::Modifiers::ALT));
    wait(&mut app, &ctx, finished);
    assert_eq!(fs::read(right.join("b.txt")).unwrap(), b"old", "Alt+S = Skip");
    fs::remove_dir_all(root).unwrap();
}

/// 원본 창은 제목 줄을 끌어 옮긴다: 창 안 대화 상자·메시지 상자도 제목 줄을 끌면 그만큼 움직인다.
#[test]
fn in_app_windows_move_by_their_title_bar() {
    let (root, ctx, mut app) = fixture("dragtitle");
    app.system_box("Title", "Text", ndr::msgbox::MB_OK, |_, _, _| {});
    draw(&mut app, &ctx);
    draw(&mut app, &ctx);
    let before = app.msgboxes[0].layout.as_ref().unwrap().origin;
    // 제목 줄은 몸통(origin) 바로 위에 있다.
    let grab = before + egui::vec2(40.0, -12.0);
    let moved = grab + egui::vec2(-150.0, 90.0);
    let pointer = |pos: egui::Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    draw_with(&mut app, &ctx, vec![egui::Event::PointerMoved(grab), pointer(grab, true)]);
    draw_with(&mut app, &ctx, vec![egui::Event::PointerMoved(grab + egui::vec2(-50.0, 30.0))]);
    draw_with(&mut app, &ctx, vec![egui::Event::PointerMoved(moved)]);
    draw_with(&mut app, &ctx, vec![pointer(moved, false)]);
    draw(&mut app, &ctx);
    let after = app.msgboxes[0].layout.as_ref().unwrap().origin;
    assert!((after - before - egui::vec2(-150.0, 90.0)).length() < 2.0, "moved by the drag: {before:?} -> {after:?}");
    assert_eq!(app.msgboxes.len(), 1, "dragging does not close the box");
    fs::remove_dir_all(root).unwrap();
}

/// 원본 0x412308 lTo = 지금 대상 파일의 폴더: 파일 작업이 알려 준 실제 대상(충돌로 바뀐 이름)이 먼저, 없으면 상대 경로로 셈한다.
#[test]
fn progress_target_line_uses_the_reported_destination_like_the_original() {
    use super::file_operation::progress_target_line;
    let target = (PathBuf::from("C:\\src\\sub\\a.txt"), PathBuf::from("D:\\dst\\renamed\\a (2).txt"));
    assert_eq!(progress_target_line(Some("C:\\src"), Some("D:\\dst"), "C:\\src\\sub\\a.txt", Some(&target)).as_deref(), Some("D:\\dst\\renamed"));
    assert_eq!(progress_target_line(Some("C:\\src"), Some("D:\\dst"), "C:\\src\\sub\\b.txt", Some(&target)).as_deref(), Some("D:\\dst\\sub"), "another file: relative rule");
    assert_eq!(progress_target_line(Some("C:\\src"), Some("D:\\dst"), "C:\\src\\b.txt", None).as_deref(), Some("D:\\dst"));
    assert_eq!(progress_target_line(None, Some("D:\\dst"), "x", None).as_deref(), Some("D:\\dst"));
    // fsops: 대상에 같은 이름이 있어 "이름 (2)"로 바꾸면 그 대상을 알린다.
    let root = std::env::temp_dir().join(format!("ndr-progress-target-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("dst")).unwrap();
    fs::write(root.join("src").join("a.txt"), b"new").unwrap();
    fs::write(root.join("dst").join("a.txt"), b"old").unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let operation = ndr::fsops::Operation::Copy { sources: vec![root.join("src").join("a.txt")], destination: root.join("dst"), move_files: false };
    let worker = std::thread::spawn(move || ndr::fsops::run(operation, tx, cancel));
    let mut reported = None;
    for event in rx {
        match event {
            ndr::fsops::Event::Conflict { reply, .. } => reply.send(ndr::fsops::ConflictChoice::Rename).unwrap(),
            ndr::fsops::Event::Target { source, destination } => reported = Some((source, destination)),
            _ => {}
        }
    }
    worker.join().unwrap();
    let (source, destination) = reported.expect("a Target event");
    assert_eq!(source, root.join("src").join("a.txt"));
    assert!(destination.parent() == Some(&root.join("dst")) && destination != root.join("dst").join("a.txt") && destination.is_file(), "{destination:?}");
    fs::remove_dir_all(root).unwrap();
}

/// 원본 epPathEllipsis(TfProgress lFrom/lTo, TfFileExists lOrgDir/lNewDir): 넘치면 마지막 "\이름"을 남기고 앞을 "..."로 줄인다.
#[test]
fn path_ellipsis_keeps_the_last_name_like_the_original() {
    let ctx = egui::Context::default();
    let mut shown = Vec::new();
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            let font = egui::FontId::proportional(13.0);
            shown.push(super::original_dialogs::path_ellipsis(ui, "C:\\short", 353.0, &font));
            shown.push(super::original_dialogs::path_ellipsis(ui, "C:\\a very long folder name\\another long folder name\\yet another folder\\document.txt", 160.0, &font));
        });
    });
    assert_eq!(shown[0], "C:\\short");
    assert!(shown[1].starts_with("C:") && shown[1].ends_with("...\\document.txt"), "{}", shown[1]);
}

/// 원본 FUN_00422068/FUN_00422314: "<b>1,234,567</b> bytes ( 긴 날짜, 시간 )", 더 새로운 쪽은 <c0000FF>(빨강).
#[test]
fn conflict_file_info_follows_the_original_tffileexists() {
    use super::file_operation::conflict_file_info;
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
    let (date, clock) = ndr::listing::long_date_and_time(time);
    assert!(!date.is_empty() && !clock.is_empty());
    assert_eq!(conflict_file_info(Some(1_234_567), Some(time), false, true), format!("<b>1,234,567</b> bytes ( {date}, {clock} )"));
    assert_eq!(conflict_file_info(Some(5), Some(time), true, false), format!("<c0000FF><b>5</b> bytes ( {date}, {clock} )</c>"));
    assert_eq!(conflict_file_info(Some(5), None, false, false), "<b>5</b> bytes");
    assert_eq!(ndr::markup::plain_text(&conflict_file_info(Some(5), None, true, false)), "5 bytes");
}

/// 원본 FUN_004116dc·FUN_00422870의 TfFileExists 단추 규칙과 DFM 자리.
#[test]
fn conflict_buttons_follow_the_original_tffileexists() {
    use super::file_operation::{ConflictButton as B, ConflictForm, conflict_buttons, is_sub_folder};
    let form = |source: &str, destination: &str, source_dir: bool, destination_dir: bool| ConflictForm {
        source: source.into(),
        destination: destination.into(),
        source_dir,
        destination_dir,
        source_size: (!source_dir).then_some(10),
        destination_size: (!destination_dir).then_some(20),
        source_modified: None,
        destination_modified: None,
        allow_resume: false,
        apply_all: false,
        icons: [None, None],
        textures: [None, None],
    };
    // 파일: 덮어쓰기·건너뛰기·새 파일 덮어쓰기·이름 바꾸기·취소(폴더 교체 없음), 모두 켜짐.
    assert_eq!(conflict_buttons(&form("c:\\a\\x.txt", "c:\\b\\x.txt", false, false)), [(B::Overwrite, true), (B::Skip, true), (B::OverwriteNewer, true), (B::Rename, true), (B::Cancel, true)]);
    // 새 항목이 폴더: 같은 자리(118)에 새 파일 덮어쓰기 대신 폴더 교체.
    assert_eq!(conflict_buttons(&form("c:\\a\\d", "c:\\b\\d", true, true)), [(B::Overwrite, true), (B::Skip, true), (B::ReplaceFolder, true), (B::Rename, true), (B::Cancel, true)]);
    assert_eq!(B::ReplaceFolder.top(), B::OverwriteNewer.top());
    // 같은 경로: 덮어쓰기·새 파일 덮어쓰기·폴더 교체를 끈다(숨기지 않는다).
    assert_eq!(conflict_buttons(&form("C:\\a\\x.txt", "c:\\A\\X.TXT", false, false)), [(B::Overwrite, false), (B::Skip, true), (B::OverwriteNewer, false), (B::Rename, true), (B::Cancel, true)]);
    // 새 항목이 기존 폴더 안이면 폴더 교체만, 기존이 새 폴더 안이면 폴더 교체와 덮어쓰기를 끈다.
    assert_eq!(conflict_buttons(&form("c:\\d\\d", "c:\\d", true, true)), [(B::Overwrite, true), (B::Skip, true), (B::ReplaceFolder, false), (B::Rename, true), (B::Cancel, true)]);
    assert_eq!(conflict_buttons(&form("c:\\d", "c:\\d\\d", true, true)), [(B::Overwrite, false), (B::Skip, true), (B::ReplaceFolder, false), (B::Rename, true), (B::Cancel, true)]);
    assert!(is_sub_folder(std::path::Path::new("C:\\Data\\"), std::path::Path::new("c:\\data\\x")));
    assert!(!is_sub_folder(std::path::Path::new("c:\\data"), std::path::Path::new("c:\\data2\\x")));
    assert!(!is_sub_folder(std::path::Path::new("c:\\data"), std::path::Path::new("c:\\data")));
    // 이어서 전송: 이어 받을 수 있는 작업이면 보이고(맨 위 25), 기존이 새 파일보다 작을 때만 켜진다(원본 FUN_00422870).
    let mut resumable = form("c:\\a\\x.bin", "c:\\b\\x.bin", false, false);
    (resumable.source_size, resumable.destination_size, resumable.allow_resume) = (Some(20), Some(10), true);
    assert_eq!(conflict_buttons(&resumable)[0], (B::Resume, true));
    resumable.destination_size = Some(30);
    assert_eq!(conflict_buttons(&resumable)[0], (B::Resume, false), "a larger existing file cannot be resumed");
    assert_eq!((B::Resume.top(), B::Overwrite.top(), B::Skip.top(), B::Rename.top(), B::Cancel.top()), (25.0, 56.0, 87.0, 149.0, 220.0));
}

#[test]
fn conflict_window_progress_window_and_reports_follow_csharp() {
    let (root, ctx, mut app) = fixture("conflict");
    let (left, right) = (root.join("left"), root.join("right"));
    // 이어서 전송: 기존 파일이 원본의 앞부분이면 나머지만 쓴다. 진행 창은 원본·대상·전체 파일 수를 안다.
    copy_to_other(&mut app, &ctx, &["a.txt"]);
    let job = app.job.as_ref().expect("a job");
    let info = job.progress.clone().expect("FileOperationProgressWindow");
    assert_eq!((job.title.as_str(), info.from.as_deref(), info.to.as_deref(), info.total_files), ("항목 복사", Some(&*left.to_string_lossy()), Some(&*right.to_string_lossy()), Some(1)));
    wait(&mut app, &ctx, |app| conflict(app).is_some());
    {
        let form = conflict(&app).unwrap();
        assert!(form.can_resume() && !form.source_dir && form.compatible(ConflictChoice::Resume));
        // 원본 imgOrg·imgNew: 두 항목의 32x32 셸 아이콘(시험에서는 가짜 4x4).
        assert!(form.icons.iter().all(|i| i.as_ref().is_some_and(|(w, h, _)| (*w, *h) == (4, 4))));
    }
    assert_eq!(app.original_dialogs.last().unwrap().title, app.language_pack.text("formFileExists.Form.Caption", "?"));
    draw(&mut app, &ctx);
    assert!(conflict(&app).unwrap().textures.iter().all(|t| t.as_ref().is_some_and(|t| t.size() == [4, 4])), "icons are drawn");
    app.answer_dialog(DialogResult::FileConflict(Some((ConflictChoice::Resume, false))), &ctx);
    wait(&mut app, &ctx, finished);
    assert_eq!(fs::read(right.join("a.txt")).unwrap(), b"resumable payload");
    assert_eq!(app.status, "요청한 1개 항목을 복사했습니다.");
    // 폴더 충돌: 폴더 교체 단추가 있고 이어서 전송은 없다. 모두 적용한 건너뛰기는 맞는 충돌에만 쓴다.
    copy_to_other(&mut app, &ctx, &["folder"]);
    wait(&mut app, &ctx, |app| conflict(app).is_some());
    {
        let form = conflict(&app).unwrap();
        assert!(form.source_dir && form.destination_dir && !form.can_resume() && !form.compatible(ConflictChoice::Resume));
    }
    app.answer_dialog(DialogResult::FileConflict(Some((ConflictChoice::Skip, true))), &ctx);
    wait(&mut app, &ctx, finished);
    assert_eq!(app.status, "복사 완료: 1개 항목을 건너뛰었습니다.");
    assert!(!right.join("folder").join("x.txt").exists());
    // 새 파일 덮어쓰기: 대상이 더 새로우면 건너뛴다.
    fs::write(right.join("b.txt"), b"newer").unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    fs::File::options().write(true).open(left.join("b.txt")).unwrap().set_modified(old).unwrap();
    copy_to_other(&mut app, &ctx, &["b.txt"]);
    wait(&mut app, &ctx, |app| conflict(app).is_some());
    app.answer_dialog(DialogResult::FileConflict(Some((ConflictChoice::OverwriteNewer, false))), &ctx);
    wait(&mut app, &ctx, finished);
    assert_eq!(fs::read(right.join("b.txt")).unwrap(), b"newer");
    assert_eq!(app.status, "복사 완료: 1개 항목을 건너뛰었습니다.");
    // 이름 바꾸기 = 원본 FUN_00411bb4: TITLE_RENAME_AND_COPY 입력(MSG_RENAME_AND_COPY의 %s와 처음 값 = 원래 이름), 확인이면 그 이름으로 복사.
    copy_to_other(&mut app, &ctx, &["b.txt"]);
    wait(&mut app, &ctx, |app| conflict(app).is_some());
    app.answer_dialog(DialogResult::FileConflict(Some((ConflictChoice::Rename, false))), &ctx);
    let renaming = |app: &App| app.original_dialogs.last().is_some_and(|d| matches!(d.kind, super::original_dialogs::DialogKind::Prompt { .. }));
    wait(&mut app, &ctx, renaming);
    {
        let dialog = app.original_dialogs.last().unwrap();
        assert_eq!(dialog.title, app.language_pack.text("RES.TITLE_RENAME_AND_COPY", "?"));
        let super::original_dialogs::DialogKind::Prompt { message, value, .. } = &dialog.kind else { unreachable!() };
        assert_eq!(*message, app.language_pack.text("RES.MSG_RENAME_AND_COPY", "?").replace("%s", "b.txt"));
        assert_eq!(value, "b.txt");
    }
    app.answer_dialog(DialogResult::Text(Some("b-copy.txt".into())), &ctx);
    wait(&mut app, &ctx, finished);
    assert_eq!(fs::read(right.join("b-copy.txt")).unwrap(), b"bee");
    // 이름 입력을 취소하면 그 항목은 건너뛴다(차이: 원본은 그대로 진행).
    copy_to_other(&mut app, &ctx, &["b.txt"]);
    wait(&mut app, &ctx, |app| conflict(app).is_some());
    app.answer_dialog(DialogResult::FileConflict(Some((ConflictChoice::Rename, false))), &ctx);
    wait(&mut app, &ctx, renaming);
    app.answer_dialog(DialogResult::Text(None), &ctx);
    wait(&mut app, &ctx, finished);
    assert_eq!(app.status, "복사 완료: 1개 항목을 건너뛰었습니다.");
    assert_eq!(fs::read(right.join("b.txt")).unwrap(), b"newer");
    // 창을 닫으면(취소) 작업을 멈춘다.
    copy_to_other(&mut app, &ctx, &["b.txt"]);
    wait(&mut app, &ctx, |app| conflict(app).is_some());
    app.answer_dialog(DialogResult::FileConflict(None), &ctx);
    wait(&mut app, &ctx, finished);
    assert_eq!(app.status, "복사 작업을 취소했습니다. 완료된 항목은 유지됩니다.");
    // 오류마다 묻는다(TITLE_FILEOPERROR, 그다음 MSG_WANTTOCONTINUE 예/아니요). 예면 나머지를 계속한다.
    let locked = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::File::options().read(true).share_mode(0).open(left.join("locked.txt")).unwrap()
    };
    fs::remove_file(right.join("b.txt")).unwrap();
    fs::remove_file(right.join("b-copy.txt")).unwrap();
    copy_to_other(&mut app, &ctx, &["b.txt", "locked.txt"]);
    wait(&mut app, &ctx, |app| !app.msgboxes.is_empty() || finished(app));
    assert_eq!(app.msgboxes[0].title, app.language_pack.text("RES.TITLE_FILEOPERROR", "?"));
    // locked.txt가 마지막이면 뒤에 남은 일이 없으므로 계속할지 묻지 않는다.
    app.answer_message_box(1, &ctx);
    assert!(app.msgboxes.is_empty(), "no continue prompt for the last item");
    wait(&mut app, &ctx, finished);
    drop(locked);
    assert_eq!(fs::read(right.join("b.txt")).unwrap(), b"bee");
    assert_eq!(app.status, "복사 완료: 요청 2개, 실패 1건, 건너뜀 0개");
    assert!(app.msgboxes.is_empty(), "the error was already shown");
    // 영구 삭제: 지운 파일·폴더 하나하나를 센다(C# deletedItems). 진행 창은 없다.
    app.active = 1;
    app.panes[1].selected.clear();
    let folder = app.panes[1].entries.iter().position(|e| e.name == "folder").unwrap();
    app.panes[1].cursor = folder;
    fs::write(right.join("folder").join("y.txt"), b"y").unwrap();
    app.job = None;
    app.dispatch("deletePermanent", &ctx);
    app.answer_dialog(DialogResult::DeleteWarning(DeleteWarningChoice::Yes), &ctx);
    wait(&mut app, &ctx, |app| app.job.is_some());
    assert!(app.job.as_ref().unwrap().progress.is_none());
    wait(&mut app, &ctx, finished);
    assert!(!right.join("folder").exists());
    assert_eq!(app.status, "2개 항목을 삭제했습니다. 실패 0개");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn external_copy_handler_follows_csharp() {
    let (root, ctx, mut app) = fixture("external");
    let (left, right) = (root.join("left"), root.join("right"));
    let tools = root.join("tools");
    fs::create_dir_all(&tools).unwrap();
    let handler = tools.join("handler.exe");
    fs::write(&handler, b"").unwrap();
    app.settings.values.insert("UseCopyHandler".into(), serde_json::json!(true));
    app.settings.values.insert("CopyHandlerPath".into(), serde_json::json!(handler.to_string_lossy()));
    let mine = |program: &Path| EXTERNAL.lock().unwrap().iter().filter(|r| r.0 == program.to_string_lossy()).cloned().collect::<Vec<_>>();
    // 여러 항목: "Copy *목록 대상 /Close", 목록은 UTF-8 BOM + 줄마다 CRLF.
    copy_to_other(&mut app, &ctx, &["a.txt", "b.txt"]);
    assert!(app.job.is_none(), "the handler replaces the internal copy");
    assert_eq!(app.status, "외부 복사 프로그램에서 2개 항목을 처리하고 있습니다.");
    wait(&mut app, &ctx, |app| app.status == "2개 항목을 복사했습니다." && finished(app));
    let runs = mine(&handler);
    assert_eq!(runs.len(), 1);
    let (_, args, list) = &runs[0];
    assert_eq!((args[0].as_str(), args[1].starts_with('*'), args[2].as_str(), args[3].as_str()), ("Copy", true, &*right.to_string_lossy(), "/Close"));
    assert_eq!(list, &format!("\u{feff}{}\r\n{}\r\n", left.join("a.txt").display(), left.join("b.txt").display()));
    assert!(!Path::new(args[1].trim_start_matches('*')).exists(), "the list file is deleted");
    // kc(KillCopy 콘솔) 이동: ANSI 목록, 대상, -m.
    let kc = tools.join("kc.exe");
    fs::write(&kc, b"").unwrap();
    app.settings.values.insert("CopyHandlerPath".into(), serde_json::json!(kc.to_string_lossy()));
    select(&mut app, &["b.txt"]);
    let active = app.active;
    app.copy_to_other(active, true, &ctx);
    app.answer_message_box(IDYES, &ctx);
    assert_eq!(app.status, "External copy handler is processing 1 item(s).");
    wait(&mut app, &ctx, |app| app.status == "1개 항목을 이동했습니다." && finished(app));
    let runs = mine(&kc);
    let (_, args, list) = &runs[0];
    assert_eq!((args.len(), args[1].as_str(), args[2].as_str()), (3, &*right.to_string_lossy(), "-m"));
    assert_eq!(list, &format!("{}\r\n", left.join("b.txt").display()));
    // 꺼져 있으면 NDR이 직접 복사한다.
    app.settings.values.insert("UseCopyHandler".into(), serde_json::json!(false));
    copy_to_other(&mut app, &ctx, &["locked.txt"]);
    assert!(app.job.is_some());
    wait(&mut app, &ctx, finished);
    assert!(right.join("locked.txt").exists());
    fs::remove_dir_all(root).unwrap();
}

thread_local! {
    static SAVE_TO: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}
fn save_to(_: &str, _: Option<&Path>, _: &str, _: &[(&str, &str)], _: &str, _: bool) -> Option<PathBuf> {
    SAVE_TO.with(|s| s.borrow().clone())
}

#[test]
fn adding_to_a_multivolume_archive_rebuilds_it_like_csharp() {
    let (root, ctx, mut app) = fixture("multivolume");
    let left = root.join("left");
    // 압축이 잘 안 되는 1.5 MiB로 1 MiB 볼륨 두 개를 만든다.
    let mut state = 0x1234_5678u32;
    let noise: Vec<u8> = (0..1_572_864).map(|_| { state ^= state << 13; state ^= state >> 17; state ^= state << 5; (state >> 24) as u8 }).collect();
    fs::write(left.join("noise.bin"), &noise).unwrap();
    let options = ndr::commands::ArchiveOptions { extension: "7z".into(), level: 1, password: String::new(), volume_bytes: Some(1024 * 1024), recursive: true, append: false };
    ndr::archive::create_with_program(&[left.join("noise.bin")], &left.join("set.7z"), &options, None, &AtomicBool::new(false)).unwrap();
    let first = left.join("set.7z.001");
    assert!(first.exists() && left.join("set.7z.002").exists());
    // 더하기: 7z를 고르고 지금 폴더의 첫 볼륨을 대상으로(압축 파일은 늘 지금 패널 폴더).
    wait(&mut app, &ctx, |app| app.panes.iter().all(|p| !p.loading));
    app.save_dialog = save_to;
    SAVE_TO.with(|s| *s.borrow_mut() = Some(first.clone()));
    select(&mut app, &["b.txt"]);
    app.dispatch("archiveAdd", &ctx);
    let target = first.to_string_lossy().into_owned();
    app.edit_dialog_kind(move |kind| {
        if let DialogKind::ArchiveCreate(form) = kind {
            let index = form.formats.iter().position(|f| f.extension == "7z").unwrap();
            form.set_format(index);
            form.file_name = target.clone();
        }
    });
    app.submit_archive_create(0, &ctx);
    wait(&mut app, &ctx, finished);
    assert!(app.job.as_ref().is_some_and(|j| j.errors.is_empty() && j.fatal.is_none()), "{:?}", app.job.as_ref().map(|j| (&j.errors, &j.fatal)));
    let names: Vec<String> = ndr::archive::list(&first, None).unwrap().into_iter().map(|e| e.key).collect();
    assert!(names.iter().any(|n| n == "noise.bin") && names.iter().any(|n| n == "b.txt"), "{names:?}");
    assert!(left.join("set.7z.002").exists(), "still split at the inferred 1 MiB");
    assert!(!fs::read_dir(left).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with(".ndr-multivolume-update-")), "the staging folder is removed");
    fs::remove_dir_all(root).unwrap();
}
