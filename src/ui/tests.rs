use super::*;
use std::fs;

fn fixture(label:&str)->(PathBuf,egui::Context,App){
    let root=std::env::temp_dir().join(format!("ndir-ui-{}-{}-{label}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    let left=root.join("left");let right=root.join("right");fs::create_dir_all(&left).unwrap();fs::create_dir(&right).unwrap();
    fs::write(left.join("한글 문서.txt"),b"owned original payload").unwrap();
    let ctx=egui::Context::default();let options=Options{profile:Some(root.join("profile")),left:Some(left),right:Some(right),language:Some(if label=="password"{"en"}else{"ko"}.into()),..Default::default()};
    let mut app=App::from_context(&ctx,options);wait_listing(&mut app,&ctx);first_item(&mut app);(root,ctx,app)
}
/// 목록은 ".."에서 시작한다. 시험은 첫 실제 항목에 커서를 둔다.
fn first_item(app:&mut App){for p in &mut app.panes{p.cursor=p.entries.iter().position(|e|!e.is_parent()).unwrap_or(0);}}
fn wait_listing(app:&mut App,ctx:&egui::Context){let end=Instant::now()+Duration::from_secs(10);while app.panes.iter().any(|p|p.loading){assert!(Instant::now()<end,"listing timeout");app.poll(ctx);thread::sleep(Duration::from_millis(5));}}
fn wait_job(app:&mut App,ctx:&egui::Context,password:Option<&str>,choice:ConflictChoice){let end=Instant::now()+Duration::from_secs(30);loop{assert!(Instant::now()<end,"job timeout");app.poll(ctx);if app.original_dialogs.last().is_some_and(|d|matches!(d.kind,original_dialogs::DialogKind::FileConflict(_))){app.answer_dialog(original_dialogs::DialogResult::FileConflict(Some((choice,false))),ctx);}if let Some(p)=app.password_request.take(){p.reply.send(password.map(str::to_owned)).unwrap();}if app.job.as_ref().is_some_and(|j|j.finished){break;}thread::sleep(Duration::from_millis(5));}let job=app.job.as_ref().unwrap();assert!(job.errors.is_empty(),"{:?}",job.errors);wait_listing(app,ctx);}

#[test]
fn ui_file_commands_preserve_conflicts_and_form_validation(){
    let(root,ctx,mut app)=fixture("copy");let left=root.join("left");let right=root.join("right");
    app.copy_to_other(0,false,&ctx);app.answer_message_box(6,&ctx);wait_job(&mut app,&ctx,None,ConflictChoice::Overwrite);assert_eq!(fs::read(right.join("한글 문서.txt")).unwrap(),b"owned original payload");
    fs::write(left.join("한글 문서.txt"),b"changed source").unwrap();first_item(&mut app);app.copy_to_other(0,false,&ctx);app.answer_message_box(6,&ctx);wait_job(&mut app,&ctx,None,ConflictChoice::Skip);assert_eq!(fs::read(right.join("한글 문서.txt")).unwrap(),b"owned original payload");
    first_item(&mut app);app.active=1;app.dispatch("rename",&ctx);app.set_dialog_text("renamed","txt");app.submit_rename(&ctx);wait_listing(&mut app,&ctx);assert!(right.join("renamed.txt").exists());assert!(!right.join("한글 문서.txt").exists());assert_eq!(app.panes[1].entries[app.panes[1].cursor].name,"renamed.txt");
    app.dispatch("newFolder",&ctx);app.answer_dialog(original_dialogs::DialogResult::Text(Some("새 폴더".into())),&ctx);wait_listing(&mut app,&ctx);assert!(right.join("새 폴더").is_dir());
    app.save();let saved=Settings::load(&root.join("profile/settings.json")).unwrap();assert_eq!(saved.text("Language",""),"ko");assert_eq!(saved.text("RightPath",""),right.to_string_lossy());
    fs::remove_dir_all(root).unwrap();
}

thread_local!{static SAVE_AS:std::cell::RefCell<Option<PathBuf>>=const{std::cell::RefCell::new(None)};static PICK:std::cell::RefCell<Option<PathBuf>>=const{std::cell::RefCell::new(None)};}
fn save_as(_:&str,_:Option<&Path>,_:&str,_:&[(&str,&str)],_:&str,_:bool)->Option<PathBuf>{SAVE_AS.with(|s|s.borrow().clone())}
fn pick(_:&str,_:Option<&Path>)->Option<PathBuf>{PICK.with(|s|s.borrow().clone())}

#[test]
fn ui_archive_masked_password_channel_and_payload_roundtrip(){
    let(root,ctx,mut app)=fixture("password");let output=root.join("left/protected.zip");
    app.save_dialog=save_as;app.folder_picker=pick;SAVE_AS.with(|s|*s.borrow_mut()=Some(output.clone()));PICK.with(|s|*s.borrow_mut()=Some(root.join("right")));
    app.dispatch("createArchive",&ctx);
    let named=output.to_string_lossy().into_owned();
    app.edit_dialog_kind(move|kind|if let original_dialogs::DialogKind::ArchiveCreate(form)=kind{form.set_format(0);form.password="fixture-password".into();form.file_name=named.clone();});
    app.submit_archive_create(0,&ctx);wait_job(&mut app,&ctx,None,ConflictChoice::Overwrite);
    assert_eq!(app.status,"Created the ZIP archive.");assert!(output.is_file());
    let archive_row=app.panes[0].entries.iter().position(|e|e.path==output).unwrap();app.panes[0].cursor=archive_row;
    app.dispatch("extractSelected",&ctx);app.destination_action(super::destination_dialog::DestinationAction::Browse,&ctx);
    wait_job(&mut app,&ctx,Some("fixture-password"),ConflictChoice::Overwrite);
    assert_eq!(fs::read(root.join("right/한글 문서.txt")).unwrap(),b"owned original payload");assert!(output.exists());
    assert_eq!(app.status,format!("Extracted to {}",root.join("right").display()));fs::remove_dir_all(root).unwrap();
}