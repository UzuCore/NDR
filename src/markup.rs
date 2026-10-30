//! 원본 언어 문자열이 쓰는 NXLabel 마크업 일부(&lt;b&gt;, &lt;cRRGGBB&gt;와 닫는 태그). C# `NxLabelMarkup`과 같은 규칙.
//! 원본의 파랑·빨강·회색·검정은 어두운 배경에서 읽히도록 밝게 바꾼다(C#과 같은 값).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    /// RGB. None = 기본 글자색.
    pub color: Option<(u8, u8, u8)>,
}

fn theme_color(rgb: &str) -> Option<(u8, u8, u8)> {
    let value = match rgb.to_ascii_uppercase().as_str() {
        "0000FF" => (0x60, 0xA5, 0xFA),
        "FF0000" => (0xF8, 0x71, 0x71),
        "666666" => (0x94, 0xA3, 0xB8),
        "000000" => (0xF1, 0xF5, 0xF9),
        other => (
            u8::from_str_radix(&other[0..2], 16).ok()?,
            u8::from_str_radix(&other[2..4], 16).ok()?,
            u8::from_str_radix(&other[4..6], 16).ok()?,
        ),
    };
    Some(value)
}

/// 태그 하나의 길이와 종류. 인식하지 못하면 None(글자 그대로).
fn tag_at(text: &str) -> Option<(usize, &str)> {
    let rest = text.strip_prefix('<')?;
    let end = rest.find('>')?;
    let tag = &rest[..end];
    let known = tag == "b" || tag == "/b" || tag == "/c"
        || (tag.len() == 7 && tag.starts_with('c') && tag[1..].chars().all(|c| c.is_ascii_hexdigit()));
    known.then_some((end + 2, tag))
}

/// 마크업을 서식 조각으로 나눈다. CR LF는 LF로 바꾼다.
pub fn render(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut bold = false;
    let mut colors: Vec<(u8, u8, u8)> = Vec::new();
    let mut current = String::new();
    let mut index = 0;
    let flush = |current: &mut String, spans: &mut Vec<Span>, bold: bool, colors: &[(u8, u8, u8)]| {
        if !current.is_empty() {
            spans.push(Span { text: std::mem::take(current).replace("\r\n", "\n"), bold, color: colors.last().copied() });
        }
    };
    while index < text.len() {
        let rest = &text[index..];
        if let Some((length, tag)) = tag_at(rest) {
            flush(&mut current, &mut spans, bold, &colors);
            match tag {
                "b" => bold = true,
                "/b" => bold = false,
                "/c" => {
                    colors.pop();
                }
                color => {
                    if let Some(rgb) = theme_color(&color[1..]) {
                        colors.push(rgb);
                    }
                }
            }
            index += length;
            continue;
        }
        let character = rest.chars().next().unwrap();
        current.push(character);
        index += character.len_utf8();
    }
    flush(&mut current, &mut spans, bold, &colors);
    spans
}

/// 태그를 뺀 글.
pub fn plain_text(text: &str) -> String {
    render(text).into_iter().map(|span| span.text).collect()
}

/// Delphi 방식 %s/%d 치환(나타나는 순서대로). 값이 모자라면 나머지 자리는 그대로 둔다.
pub fn substitute(template: &str, values: &[&str]) -> String {
    let mut result = String::with_capacity(template.len());
    let mut values = values.iter();
    let mut chars = template.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c == '%' {
            if let Some(&(_, next)) = chars.peek() {
                if next == 's' || next == 'd' {
                    if let Some(value) = values.next() {
                        result.push_str(value);
                        chars.next();
                        continue;
                    }
                }
            }
        }
        result.push(c);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_matches_nxlabel_rules() {
        let spans = render("Do you really want to <b>Delete</b>?\r\n<b><c0000FF>a.txt</c></b> <c666666>(1 / 2)</c>");
        assert_eq!(spans[0], Span { text: "Do you really want to ".into(), bold: false, color: None });
        assert_eq!(spans[1], Span { text: "Delete".into(), bold: true, color: None });
        assert_eq!(spans[2].text, "?\n");
        assert_eq!(spans[3], Span { text: "a.txt".into(), bold: true, color: Some((0x60, 0xA5, 0xFA)) });
        assert_eq!(spans.last().unwrap(), &Span { text: "(1 / 2)".into(), bold: false, color: Some((0x94, 0xA3, 0xB8)) });
        assert_eq!(plain_text("<b>x</b><c123456>y</c><z>"), "xy<z>", "unknown tags stay as text");
        assert_eq!(render("<c12345G>q")[0].text, "<c12345G>q");
    }

    #[test]
    fn delphi_substitution_in_order() {
        assert_eq!(substitute("%s (%d / %d)", &["a.txt", "1", "3"]), "a.txt (1 / 3)");
        assert_eq!(substitute("%s and %s", &["x"]), "x and %s");
        assert_eq!(substitute("100%", &["x"]), "100%");
    }
}
