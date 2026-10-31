//! Port of the original TfMsgBox (wrapper FUN_0040bb38 -> FUN_0044ae58).
//!
//! Every original call goes through `FUN_0040bb38(title, summary, text, flags)`. An empty title becomes the product name,
//! and after creating the form FUN_0044ae58 lays it out, shows it modally and returns the modal result as a Win32-style ID.
//! Flags are the MB_* values the original passes. This module holds only screen-independent rules (the UI is `ui::msgbox`).

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

/// FUN_0040bb38 turns an empty title into L"nexusfile" (0x8d58f8). NDR uses its own product name (documented difference).
pub const DEFAULT_TITLE: &str = "NDR";
/// fMsgBox DFM Caption ("NexusFile", no language item). NDR uses its own product name (documented difference).
pub const FORM_CAPTION: &str = "NDR";
/// The original computes the outer Width (Delphi TControl). The Windows 10/11 bsDialog frame thickness
/// 2 x (SM_CXFIXEDFRAME 3 + SM_CXPADDEDBORDER 4) = 14 is subtracted to get the client width. egui windows have no OS frame.
pub const FRAME_WIDTH: f32 = 14.0;

/// One button: [RES] caption key and original default (with '&'), Delphi ModalResult, Default, Cancel.
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

/// Kinds the original does not handle leave the DFM buttons as is: visible, no caption, ModalResult 0.
pub const BLANK: ButtonSpec = ButtonSpec { key: "", fallback: "", modal_result: 0, is_default: false, is_cancel: false };

/// The flags & 0xf table of FUN_0044ae58. Slot order matches the DFM:
/// [0] btn3 (left, TabOrder 1), [1] btn2, [2] btn1 (right). None = hidden.
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

/// ShowModal result conversion at the end of FUN_0044ae58 (mrContinue 14 becomes IDCONTINUE 11, the rest IDCANCEL).
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

/// LoadIconW(NULL, id) by flags & 0xf0. Other values leave the icon variable uninitialized in the original, so no icon.
pub fn icon_id(flags: u32) -> u16 {
    match flags & 0xf0 {
        MB_ICONEXCLAMATION => 0x7f03,
        MB_ICONASTERISK => 0x7f04,
        MB_ICONQUESTION => 0x7f02,
        MB_ICONHAND => 0x7f01,
        _ => 0,
    }
}

/// Position and size FUN_0044ae58 computes from the DFM (client coordinates, outer width).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geometry {
    pub image_top: f32,
    pub summary_top: f32,
    pub text_top: f32,
    pub client_height: f32,
    pub outer_width: f32,
}

pub fn layout(has_title: bool, has_summary: bool, title_width: f32, summary_width: f32, text_width: f32, text_height: f32) -> Geometry {
    // No title: pTitle hidden, Height -= 24, Image1/lSummary/lText move up by 24.
    // No summary: lSummary hidden, lText takes lSummary's Top and Height -= 20.
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

/// Approved fix: the original does not widen the form for buttons, so with short text the right-anchored buttons are pushed past the left edge.
/// NDR keeps the image margin (8) before the leftmost button.
pub fn minimum_client_width(visible_buttons: usize) -> f32 {
    if visible_buttons == 0 { 0.0 } else { 8.0 + visible_buttons as f32 * 81.0 - 3.0 + 15.0 }
}

pub fn client_width(geometry: &Geometry, visible_buttons: usize) -> f32 {
    (geometry.outer_width - FRAME_WIDTH).max(minimum_client_width(visible_buttons))
}

/// akRight/akBottom: btn1 keeps a right margin of 15 and bottom margin of 8, btn2/btn3 are 81 further left each.
pub fn button_left(client_width: f32, slot: usize) -> f32 {
    client_width - 15.0 - 78.0 - 81.0 * (2 - slot.min(2)) as f32
}

pub fn button_top(client_height: f32) -> f32 {
    client_height - 8.0 - 25.0
}

/// The first visible button in TabOrder (btn3, btn2, btn1) gets focus.
pub fn first_visible(buttons: &[Option<ButtonSpec>; 3]) -> Option<usize> {
    buttons.iter().position(Option::is_some)
}

pub fn cancel_slot(buttons: &[Option<ButtonSpec>; 3]) -> Option<usize> {
    buttons.iter().position(|b| b.is_some_and(|b| b.is_cancel))
}

pub fn default_slot(buttons: &[Option<ButtonSpec>; 3]) -> Option<usize> {
    buttons.iter().position(|b| b.is_some_and(|b| b.is_default))
}

/// MB_DEFBUTTON1..3 of USER32 MessageBoxW (flags & 0xf00 = 0, 0x100, 0x200): the slot at that position (from the left) among visible buttons.
/// The first button when there is no such button. (The original TfMsgBox FUN_0044ae58 ignores this value.)
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

/// Access key of a Delphi caption (upper case). '&&' is a literal '&'.
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

/// Caption to display ('&' removed, '&&' as '&') and the position (in characters) of the underlined character.
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

/// Lines of lText (CR LF, LF and CR all break lines). Empty text is one empty line.
pub fn lines(text: &str) -> Vec<String> {
    text.replace("\r\n", "\n").replace('\r', "\n").split('\n').map(str::to_owned).collect()
}

/// TNXLabel LineSpace 4: n lines of height h are n*h + (n-1)*4 high.
pub fn text_height(line_count: usize, line_height: f32) -> f32 {
    let n = line_count.max(1) as f32;
    n * line_height + (n - 1.0) * 4.0
}

#[cfg(test)]
mod tests {
    use super::*;

    type Expected = Option<(&'static str, i32, bool, bool)>;
    // FUN_0044ae58 decompiled table (slot 0 = btn3, 1 = btn2, 2 = btn1) and the 0x8f7f38 literals.
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
