//! 원본 TfMsgBox 이식 (래퍼 FUN_0040bb38 -> FUN_0044ae58).
//!
//! 원본의 모든 호출은 `FUN_0040bb38(title, summary, text, flags)`를 거친다. 빈 제목은 제품 이름으로 바뀌고,
//! 폼을 만든 뒤 FUN_0044ae58이 배치해서 모달로 보여 주며, 모달 결과를 Win32 방식의 ID로 바꿔 돌려준다.
//! 플래그는 원본이 넘기는 MB_* 값을 그대로 쓴다. 이 모듈은 화면과 무관한 규칙만 담는다(UI는 `ui::msgbox`).

pub const MB_OK: u32 = 0x0;
pub const MB_OKCANCEL: u32 = 0x1;
pub const MB_ABORTRETRYIGNORE: u32 = 0x2;
pub const MB_YESNOCANCEL: u32 = 0x3;
pub const MB_YESNO: u32 = 0x4;
pub const MB_RETRYCANCEL: u32 = 0x5;
pub const MB_CANCELTRYCONTINUE: u32 = 0x6;
pub const MB_ICONHAND: u32 = 0x10;
pub const MB_ICONQUESTION: u32 = 0x20;
pub const MB_ICONEXCLAMATION: u32 = 0x30;
pub const MB_ICONASTERISK: u32 = 0x40;

pub const IDOK: i32 = 1;
pub const IDCANCEL: i32 = 2;
pub const IDABORT: i32 = 3;
pub const IDRETRY: i32 = 4;
pub const IDIGNORE: i32 = 5;
pub const IDYES: i32 = 6;
pub const IDNO: i32 = 7;
pub const IDCONTINUE: i32 = 11;

/// FUN_0040bb38은 빈 제목을 L"nexusfile"(0x8d58f8)로 바꾼다. NDR은 자기 제품 이름을 쓴다(문서화된 차이).
pub const DEFAULT_TITLE: &str = "NDR";
/// fMsgBox DFM Caption("NexusFile", 언어 항목 없음). NDR은 자기 제품 이름을 쓴다(문서화된 차이).
pub const FORM_CAPTION: &str = "NDR";
/// 원본은 바깥 Width를 계산한다(Delphi TControl). Windows 10/11의 bsDialog 테두리 두께
/// 2 x (SM_CXFIXEDFRAME 3 + SM_CXPADDEDBORDER 4) = 14를 빼서 클라이언트 폭으로 바꾼다. egui 창에는 OS 테두리가 없다.
pub const FRAME_WIDTH: f32 = 14.0;

/// 버튼 하나: [RES] 캡션 키와 원본 기본값('&' 포함), Delphi ModalResult, Default, Cancel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ButtonSpec {
    pub key: &'static str,
    pub fallback: &'static str,
    pub modal_result: i32,
    pub is_default: bool,
    pub is_cancel: bool,
}

const fn spec(key: &'static str, fallback: &'static str, modal_result: i32, is_default: bool, is_cancel: bool) -> Option<ButtonSpec> {
    Some(ButtonSpec { key, fallback, modal_result, is_default, is_cancel })
}

/// 원본이 처리하지 않는 종류는 DFM 버튼을 그대로 둔다: 보이고, 캡션 없음, ModalResult 0.
pub const BLANK: ButtonSpec = ButtonSpec { key: "", fallback: "", modal_result: 0, is_default: false, is_cancel: false };

/// FUN_0044ae58의 flags & 0xf 표. 칸 순서는 DFM과 같다:
/// [0] btn3(왼쪽, TabOrder 1), [1] btn2, [2] btn1(오른쪽). None = 숨김.
pub fn buttons(flags: u32) -> [Option<ButtonSpec>; 3] {
    match flags & 0xf {
        MB_OK => [None, None, spec("OK", "&Ok", 1, true, true)],
        MB_OKCANCEL => [None, spec("OK", "&Ok", 1, true, false), spec("CANCEL", "&Cancel", 2, false, true)],
        MB_ABORTRETRYIGNORE => [spec("ABORT", "&Stop", 3, false, false), spec("RETRY", "&Retry", 4, false, false), spec("IGNORE", "&Ignore", 5, false, false)],
        MB_YESNOCANCEL => [spec("YES", "&Yes", 6, true, false), spec("NO", "&No", 7, false, false), spec("CANCEL", "&Cancel", 2, false, true)],
        MB_YESNO => [None, spec("YES", "&Yes", 6, true, false), spec("NO", "&No", 7, false, true)],
        MB_RETRYCANCEL => [None, spec("RETRY", "&Retry", 4, true, false), spec("CANCEL", "&Cancel", 2, false, true)],
        MB_CANCELTRYCONTINUE => [spec("CANCEL", "&Cancel", 2, false, true), spec("RETRY", "&Retry", 4, true, false), spec("CONTINUE", "Conti&nue", 14, false, false)],
        _ => [Some(BLANK), Some(BLANK), Some(BLANK)],
    }
}

/// FUN_0044ae58 끝의 ShowModal 결과 변환(mrContinue 14는 IDCONTINUE 11, 나머지는 IDCANCEL).
pub fn map_result(modal_result: i32) -> i32 {
    match modal_result {
        3 => IDABORT,
        14 => IDCONTINUE,
        5 => IDIGNORE,
        7 => IDNO,
        1 => IDOK,
        4 => IDRETRY,
        6 => IDYES,
        _ => IDCANCEL,
    }
}

/// flags & 0xf0에 따른 LoadIconW(NULL, id). 그 밖의 값은 원본에서 아이콘 변수가 초기화되지 않으므로 아이콘 없음.
pub fn icon_id(flags: u32) -> u16 {
    match flags & 0xf0 {
        MB_ICONEXCLAMATION => 0x7f03,
        MB_ICONASTERISK => 0x7f04,
        MB_ICONQUESTION => 0x7f02,
        MB_ICONHAND => 0x7f01,
        _ => 0,
    }
}

/// FUN_0044ae58이 DFM에서 계산하는 위치와 크기(클라이언트 좌표, 바깥 폭).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub image_top: f32,
    pub summary_top: f32,
    pub text_top: f32,
    pub client_height: f32,
    pub outer_width: f32,
}

pub fn layout(has_title: bool, has_summary: bool, title_width: f32, summary_width: f32, text_width: f32, text_height: f32) -> Geometry {
    // 제목 없음: pTitle 숨김, Height -= 24, Image1/lSummary/lText를 24 위로.
    // 요약 없음: lSummary 숨김, lText가 lSummary의 Top을 받고 Height -= 20.
    let shift = if has_title { 0.0 } else { 24.0 };
    let width = (if has_title { title_width } else { 0.0 })
        .max(if has_summary { summary_width } else { 0.0 })
        .max(text_width);
    Geometry {
        image_top: 40.0 - shift,
        summary_top: 41.0 - shift,
        text_top: (if has_summary { 60.0 } else { 41.0 }) - shift,
        client_height: 119.0 - shift - if has_summary { 0.0 } else { 20.0 } + text_height, // Height += lText.Height
        outer_width: 64.0 + width + 20.0,                                                   // Width = lText.Left + w + 20
    }
}

/// 승인된 수정: 원본은 버튼 때문에 폼을 넓히지 않아서 글이 짧으면 오른쪽에 붙은 버튼이 왼쪽 끝 밖으로 밀린다.
/// NDR은 가장 왼쪽 버튼 앞에 그림 여백(8)을 남긴다.
pub fn minimum_client_width(visible_buttons: usize) -> f32 {
    if visible_buttons == 0 { 0.0 } else { 8.0 + visible_buttons as f32 * 81.0 - 3.0 + 15.0 }
}

pub fn client_width(geometry: &Geometry, visible_buttons: usize) -> f32 {
    (geometry.outer_width - FRAME_WIDTH).max(minimum_client_width(visible_buttons))
}

/// akRight/akBottom: btn1은 오른쪽 15, 아래 8 여백을 유지하고 btn2/btn3은 81씩 왼쪽.
pub fn button_left(client_width: f32, slot: usize) -> f32 {
    client_width - 15.0 - 78.0 - 81.0 * (2 - slot.min(2)) as f32
}

pub fn button_top(client_height: f32) -> f32 {
    client_height - 8.0 - 25.0
}

/// TabOrder(btn3, btn2, btn1)에서 처음 보이는 버튼이 포커스를 받는다.
pub fn first_visible(buttons: &[Option<ButtonSpec>; 3]) -> Option<usize> {
    buttons.iter().position(Option::is_some)
}

pub fn cancel_slot(buttons: &[Option<ButtonSpec>; 3]) -> Option<usize> {
    buttons.iter().position(|b| b.is_some_and(|b| b.is_cancel))
}

pub fn default_slot(buttons: &[Option<ButtonSpec>; 3]) -> Option<usize> {
    buttons.iter().position(|b| b.is_some_and(|b| b.is_default))
}

/// USER32 MessageBoxW의 MB_DEFBUTTON1..3(flags & 0xf00 = 0, 0x100, 0x200): 보이는 버튼 가운데 그 차례(왼쪽부터)의 칸.
/// 그런 버튼이 없으면 첫 버튼. (원본 TfMsgBox FUN_0044ae58은 이 값을 보지 않는다.)
pub fn system_default_slot(buttons: &[Option<ButtonSpec>; 3], flags: u32) -> Option<usize> {
    let wanted = ((flags >> 8) & 0xf) as usize;
    let visible: Vec<usize> = (0..3).filter(|&slot| buttons[slot].is_some()).collect();
    visible.get(wanted).or(visible.first()).copied()
}

#[cfg(test)]
mod default_button_tests {
    use super::*;
    #[test]
    fn message_box_w_default_button_follows_mb_defbutton() {
        let yes_no = buttons(MB_YESNO);
        assert_eq!(system_default_slot(&yes_no, MB_YESNO), Some(1), "Yes");
        assert_eq!(system_default_slot(&yes_no, MB_YESNO | 0x100), Some(2), "MB_DEFBUTTON2 = No");
        assert_eq!(system_default_slot(&yes_no, MB_YESNO | 0x200), Some(1), "no third button → first");
        assert_eq!(system_default_slot(&buttons(MB_YESNOCANCEL), MB_YESNOCANCEL | 0x200), Some(2), "MB_DEFBUTTON3 = Cancel");
    }
}

/// Delphi 캡션의 단축 문자(대문자). '&&'는 문자 그대로의 '&'.
pub fn access_key(caption: &str) -> Option<char> {
    let mut chars = caption.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '&' {
            continue;
        }
        match chars.next() {
            Some('&') => continue,
            Some(next) => return next.to_uppercase().next(),
            None => return None,
        }
    }
    None
}

/// 표시할 캡션('&' 제거, '&&'는 '&')과 밑줄 칠 문자의 위치(문자 단위).
pub fn display_caption(caption: &str) -> (String, Option<usize>) {
    let mut text = String::new();
    let mut underline = None;
    let mut chars = caption.chars();
    let mut count = 0;
    while let Some(c) = chars.next() {
        if c == '&' {
            match chars.next() {
                Some('&') => {
                    text.push('&');
                    count += 1;
                }
                Some(next) => {
                    if underline.is_none() {
                        underline = Some(count);
                    }
                    text.push(next);
                    count += 1;
                }
                None => {}
            }
        } else {
            text.push(c);
            count += 1;
        }
    }
    (text, underline)
}

/// lText의 줄(CR LF, LF, CR 모두 줄바꿈). 빈 글은 빈 줄 하나.
pub fn lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n").replace('\r', "\n").split('\n').map(str::to_owned).collect()
}

/// TNXLabel LineSpace 4: 줄 높이 h인 n줄의 높이는 n*h + (n-1)*4.
pub fn text_height(line_count: usize, line_height: f32) -> f32 {
    let n = line_count.max(1) as f32;
    n * line_height + (n - 1.0) * 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    type Expected = Option<(&'static str, i32, bool, bool)>;
    // FUN_0044ae58 디컴파일 표(칸 0 = btn3, 1 = btn2, 2 = btn1)와 0x8f7f38 리터럴.
    const TABLE: [(u32, [Expected; 3]); 7] = [
        (0, [None, None, Some(("OK", 1, true, true))]),
        (1, [None, Some(("OK", 1, true, false)), Some(("CANCEL", 2, false, true))]),
        (2, [Some(("ABORT", 3, false, false)), Some(("RETRY", 4, false, false)), Some(("IGNORE", 5, false, false))]),
        (3, [Some(("YES", 6, true, false)), Some(("NO", 7, false, false)), Some(("CANCEL", 2, false, true))]),
        (4, [None, Some(("YES", 6, true, false)), Some(("NO", 7, false, true))]),
        (5, [None, Some(("RETRY", 4, true, false)), Some(("CANCEL", 2, false, true))]),
        (6, [Some(("CANCEL", 2, false, true)), Some(("RETRY", 4, true, false)), Some(("CONTINUE", 14, false, false))]),
    ];
    fn native_default(key: &str) -> &'static str {
        match key {
            "ABORT" => "&Stop",
            "RETRY" => "&Retry",
            "IGNORE" => "&Ignore",
            "CANCEL" => "&Cancel",
            "CONTINUE" => "Conti&nue",
            "OK" => "&Ok",
            "YES" => "&Yes",
            "NO" => "&No",
            _ => "",
        }
    }

    #[test]
    fn button_table_matches_the_decompiled_original() {
        for (flags, expected) in TABLE {
            let actual = buttons(flags);
            for slot in 0..3 {
                match (actual[slot], expected[slot]) {
                    (None, None) => {}
                    (Some(a), Some((key, mr, def, can))) => {
                        assert_eq!((a.key, a.modal_result, a.is_default, a.is_cancel), (key, mr, def, can), "type {flags} slot {slot}");
                        assert_eq!(a.fallback, native_default(key));
                    }
                    other => panic!("type {flags} slot {slot}: {other:?}"),
                }
            }
        }
        for flags in 7..16 {
            assert!(buttons(flags).iter().all(|b| *b == Some(BLANK)), "unhandled type {flags}");
        }
        assert_eq!(buttons(MB_YESNO | MB_ICONQUESTION), buttons(MB_YESNO));
    }

    #[test]
    fn result_and_icon_mappings() {
        for (mr, id) in [(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6), (7, 7), (14, 11), (0, 2), (8, 2), (13, 2)] {
            assert_eq!(map_result(mr), id, "mr {mr}");
        }
        for (flags, id) in [(0x00, 0), (0x10, 0x7f01), (0x20, 0x7f02), (0x30, 0x7f03), (0x40, 0x7f04), (0x50, 0), (0xf0, 0), (0x24, 0x7f02), (0x41, 0x7f04)] {
            assert_eq!(icon_id(flags), id, "flags {flags:#x}");
        }
    }

    #[test]
    fn layout_follows_the_dfm_rules() {
        assert_eq!(layout(true, false, 30.0, 999.0, 100.0, 13.0), Geometry { image_top: 40.0, summary_top: 41.0, text_top: 41.0, client_height: 112.0, outer_width: 184.0 });
        assert_eq!(layout(true, true, 200.0, 150.0, 100.0, 30.0), Geometry { image_top: 40.0, summary_top: 41.0, text_top: 60.0, client_height: 149.0, outer_width: 284.0 });
        assert_eq!(layout(false, false, 500.0, 0.0, 120.0, 13.0), Geometry { image_top: 16.0, summary_top: 17.0, text_top: 17.0, client_height: 88.0, outer_width: 204.0 });
        assert_eq!(layout(false, true, 0.0, 50.0, 40.0, 13.0), Geometry { image_top: 16.0, summary_top: 17.0, text_top: 36.0, client_height: 108.0, outer_width: 134.0 });
        assert_eq!((minimum_client_width(1), minimum_client_width(2), minimum_client_width(3)), (101.0, 182.0, 263.0));
        let g = layout(true, false, 30.0, 0.0, 10.0, 13.0);
        assert_eq!(client_width(&g, 3), 263.0, "short text with three buttons is widened");
        assert_eq!(client_width(&layout(true, false, 30.0, 0.0, 300.0, 13.0), 1), 384.0 - FRAME_WIDTH);
        assert_eq!([button_left(413.0, 0), button_left(413.0, 1), button_left(413.0, 2)], [158.0, 239.0, 320.0], "DFM positions");
        assert_eq!(button_top(119.0), 86.0);
        assert_eq!(text_height(1, 13.0), 13.0);
        assert_eq!(text_height(2, 13.0), 30.0);
    }

    #[test]
    fn focus_default_cancel_and_captions() {
        assert_eq!(first_visible(&buttons(MB_OK)), Some(2));
        assert_eq!(first_visible(&buttons(MB_YESNO)), Some(1));
        assert_eq!(first_visible(&buttons(MB_YESNOCANCEL)), Some(0));
        assert_eq!(cancel_slot(&buttons(MB_YESNO)), Some(2));
        assert_eq!(cancel_slot(&buttons(MB_ABORTRETRYIGNORE)), None);
        assert_eq!(default_slot(&buttons(MB_CANCELTRYCONTINUE)), Some(1));
        assert_eq!(access_key("예(&Y)"), Some('Y'));
        assert_eq!(access_key("Conti&nue"), Some('N'));
        assert_eq!(access_key("a&&b"), None);
        assert_eq!(access_key("x&&&y"), Some('Y'));
        assert_eq!(display_caption("예(&Y)"), ("예(Y)".into(), Some(2)));
        assert_eq!(display_caption("A&&B&C"), ("A&BC".into(), Some(3)));
        assert_eq!(lines("Two\r\nlines"), vec!["Two", "lines"]);
        assert_eq!(lines(""), vec![""]);
    }
}
