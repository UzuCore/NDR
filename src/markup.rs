//! NXLabel markup used by original language strings: &lt;b&gt; &lt;i&gt; &lt;u&gt; &lt;cRRGGBB&gt; and their closing tags.
//! Original TNXLabel.DrawCaption 0x7c5c4c: &lt;b&gt;/&lt;i&gt;/&lt;u&gt; set and their closing tags clear fsBold/fsItalic/fsUnderline;
//! &lt;cRRGGBB&gt; (a '>' 8 characters after '<') saves the current color and switches to RRGGBB; &lt;/c&gt; returns to the saved color.
//! There is one saved slot, not a stack, so after nested colors the second &lt;/c&gt; keeps the outer color.
//! Fixed: the original drops the '<' of a "<c" that is not a full tag and does not check the hex digits; such text stays literal here.
//! The original blue, red, gray and black are brightened so they stay readable on dark backgrounds (same values as C#).

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    /// RGB. None = default text color.
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

/// Length and kind of one tag. None when unrecognized (literal text).
fn tag_at(text: &str) -> Option<(usize, &str)> {
    let rest = text.strip_prefix('<')?;
    let end = rest.find('>')?;
    let tag = &rest[..end];
    let known = matches!(tag, "b" | "/b" | "i" | "/i" | "u" | "/u" | "/c")
        || (tag.len() == 7 && tag.starts_with('c') && tag[1..].chars().all(|c| c.is_ascii_hexdigit()));
    known.then_some((end + 2, tag))
}

/// Splits markup into styled pieces. CR LF becomes LF.
pub fn render(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let (mut bold, mut italic, mut underline) = (false, false, false);
    let mut color: Option<(u8, u8, u8)> = None;
    // Original local_50: the color in effect when the last <c..> was read; </c> returns to it.
    // Fixed: before any <c..> it is the default color (the original slot is uninitialized there).
    let mut saved: Option<(u8, u8, u8)> = None;
    let mut current = String::new();
    let mut index = 0;
    let flush = |current: &mut String, spans: &mut Vec<Span>, style: (bool, bool, bool), color: Option<(u8, u8, u8)>| {
        if !current.is_empty() {
            spans.push(Span { text: std::mem::take(current).replace("\r\n", "\n"), bold: style.0, italic: style.1, underline: style.2, color });
        }
    };
    while index < text.len() {
        let rest = &text[index..];
        if let Some((length, tag)) = tag_at(rest) {
            flush(&mut current, &mut spans, (bold, italic, underline), color);
            match tag {
                "b" => bold = true,
                "/b" => bold = false,
                "i" => italic = true,
                "/i" => italic = false,
                "u" => underline = true,
                "/u" => underline = false,
                "/c" => color = saved,
                rgb => {
                    saved = color;
                    color = theme_color(&rgb[1..]);
                }
            }
            index += length;
            continue;
        }
        let character = rest.chars().next().unwrap();
        current.push(character);
        index += character.len_utf8();
    }
    flush(&mut current, &mut spans, (bold, italic, underline), color);
    spans
}

/// Text without tags, as the original TNXLabel.GetTextOnly 0x7c5368: every "<...>" is removed (known tag or not), a '<' with no
/// later '>' is dropped by itself, and "&lt;" / "&gt;" become '<' / '>'. CR LF becomes LF as in `render`.
pub fn plain_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(character) = rest.chars().next() {
        if character == '<' {
            rest = match rest.find('>') {
                Some(end) => &rest[end + 1..],
                None => &rest[1..],
            };
        } else if let Some(after) = rest.strip_prefix("&lt;") {
            result.push('<');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("&gt;") {
            result.push('>');
            rest = after;
        } else {
            result.push(character);
            rest = &rest[character.len_utf8()..];
        }
    }
    result.replace("\r\n", "\n")
}

/// Delphi-style %s/%d substitution (in order of appearance). With too few values, the remaining slots stay as they are.
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
        let plain = |text: &str, bold: bool, color: Option<(u8, u8, u8)>| Span { text: text.into(), bold, italic: false, underline: false, color };
        assert_eq!(spans[0], plain("Do you really want to ", false, None));
        assert_eq!(spans[1], plain("Delete", true, None));
        assert_eq!(spans[2].text, "?\n");
        assert_eq!(spans[3], plain("a.txt", true, Some((0x60, 0xA5, 0xFA))));
        assert_eq!(spans.last().unwrap(), &plain("(1 / 2)", false, Some((0x94, 0xA3, 0xB8))));
        assert_eq!(render("<b>x</b><c123456>y</c><z>").into_iter().map(|s| s.text).collect::<String>(), "xy<z>", "unknown tags stay as text when drawn");
        // GetTextOnly removes every <...>, drops a lone '<' and decodes &lt; &gt;.
        assert_eq!(plain_text("<b>x</b><c123456>y</c><z>"), "xy");
        assert_eq!(plain_text("a &lt;b&gt; c<d"), "a <b> cd");
        assert_eq!(plain_text("x\r\n<b>y</b>"), "x\ny");
        assert_eq!(render("<c12345G>q")[0].text, "<c12345G>q");
        // <i> and <u> (DrawCaption fsItalic / fsUnderline).
        let spans = render("<i>a<u>b</i>c</u>d");
        let styles: Vec<_> = spans.iter().map(|s| (s.text.as_str(), s.italic, s.underline)).collect();
        assert_eq!(styles, [("a", true, false), ("b", true, true), ("c", false, true), ("d", false, false)]);
        // One saved color slot: the inner </c> returns to the outer color and so does the next one.
        let colors: Vec<_> = render("<c112233>a<c445566>b</c>c</c>d").into_iter().map(|s| s.color).collect();
        assert_eq!(colors, [Some((0x11, 0x22, 0x33)), Some((0x44, 0x55, 0x66)), Some((0x11, 0x22, 0x33)), Some((0x11, 0x22, 0x33))]);
        // An unclosed color continues to the end, and a stray </c> keeps the default color.
        assert_eq!(render("</c>x<c112233>y")[1].color, Some((0x11, 0x22, 0x33)));
        assert_eq!(render("</c>x")[0].color, None);
    }

    #[test]
    fn delphi_substitution_in_order() {
        assert_eq!(substitute("%s (%d / %d)", &["a.txt", "1", "3"]), "a.txt (1 / 3)");
        assert_eq!(substitute("%s and %s", &["x"]), "x and %s");
        assert_eq!(substitute("100%", &["x"]), "100%");
    }
}
