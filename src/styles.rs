//! Original NexusFile skin and color-map data, independent of any GUI toolkit.
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub type Rgba = [u8; 4];
/// Six-digit RGB or WPF-compatible eight-digit ARGB, returned as RGBA.
pub fn parse_color(text: &str) -> Option<Rgba> {
    let text = text.trim().trim_start_matches('#');
    if !matches!(text.len(), 6 | 8) || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let number = u32::from_str_radix(text, 16).ok()?;
    Some([
        (number >> 16) as u8,
        (number >> 8) as u8,
        number as u8,
        if text.len() == 8 {
            (number >> 24) as u8
        } else {
            255
        },
    ])
}
/// Original TNXBasicColor order (same order as the color rows of lbBasicColor in Preferences): 14 file list colors, 6 folder tree colors.
pub const DEFAULT_ELEMENTS: &[(&str, &str)] = &[
    ("NXBACK", "000000"),
    ("NXFORE", "C0C0C0"),
    ("NXFOLDER", "EA2222"),
    ("NXDRIVE", "8080FF"),
    ("NXREADONLY", "FFFF00"),
    ("NXHIDDEN", "FF0000"),
    ("NXDESC", "3682AF"),
    ("NXMB", "FF0000"),
    ("NXGB", "FF0000"),
    ("NXSIZE", "606060"),
    ("NXTIME", "808080"),
    ("NXATTR", "606060"),
    ("NXSELECT", "FFFFFF"),
    ("NXLINE", "222222"),
    ("NXTBACK", "FFFFFF"),
    ("NXTFORE", "000000"),
    ("NXTOPENED", "FF0000"),
    ("NXTFOCUSED", "0000FF"),
    ("NXTLINE", "C0C0C0"),
    ("NXTHINT", "808080"),
];
fn rgb_hex(color: Rgba) -> String {
    format!("{:02X}{:02X}{:02X}", color[0], color[1], color[2])
}
fn extension_key(extension: &str) -> Option<String> {
    let extension = extension.trim().trim_start_matches('*');
    let extension = if extension.starts_with('.') {
        extension.to_lowercase()
    } else {
        format!(".{}", extension.to_lowercase())
    };
    if extension.encode_utf16().count() <= 1
        || extension.encode_utf16().count() > 32
        || extension
            .chars()
            .any(|c| c < ' ' || "\\/:*?\"<>|".contains(c))
    {
        return None;
    }
    Some(extension)
}
#[derive(Clone, Debug, Default)]
pub struct ColorMap {
    pub elements: BTreeMap<String, Rgba>,
    pub extensions: BTreeMap<String, Rgba>,
}
impl ColorMap {
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut map = Self::default();
        let mut section = "";
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_start_matches('#');
            if value.len() != 6 {
                continue;
            }
            let Some(color) = parse_color(value) else {
                continue;
            };
            if section.eq_ignore_ascii_case("Ext") {
                if let Some(key) = extension_key(key) {
                    map.extensions.insert(key, color);
                }
            } else if section.eq_ignore_ascii_case("NEXUSFILECOLOR5")
                && key.trim().to_ascii_uppercase().starts_with("NX")
            {
                map.elements.insert(key.trim().to_ascii_uppercase(), color);
            }
        }
        if map.elements.is_empty() && map.extensions.is_empty() {
            return Err("The NexusFile color map contains no usable colors".into());
        }
        Ok(map)
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        Self::parse(&crate::compat::decode(
            &std::fs::read(path).map_err(|e| e.to_string())?,
            false,
        ))
    }
    pub fn serialize(&self) -> String {
        let mut merged: BTreeMap<_, _> = DEFAULT_ELEMENTS
            .iter()
            .filter_map(|(key, value)| parse_color(value).map(|color| ((*key).to_owned(), color)))
            .collect();
        merged.extend(
            self.elements
                .iter()
                .map(|(key, color)| (key.to_ascii_uppercase(), *color)),
        );
        let mut text = "[NEXUSFILECOLOR5]\r\n".to_owned();
        for (key, color) in merged {
            text.push_str(&format!("{key}={}\r\n", rgb_hex(color)));
        }
        text.push_str("[Ext]\r\n");
        for (extension, color) in &self.extensions {
            text.push_str(&format!(
                "{}={}\r\n",
                extension.trim_start_matches('.').to_ascii_uppercase(),
                rgb_hex(*color)
            ));
        }
        text
    }
    pub fn element(&self, key: &str) -> Option<Rgba> {
        self.elements
            .get(&key.to_ascii_uppercase())
            .copied()
            .or_else(|| {
                DEFAULT_ELEMENTS
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case(key))
                    .and_then(|(_, value)| parse_color(value))
            })
    }
    pub fn extension(&self, extension: &str) -> Option<Rgba> {
        self.extensions.get(&extension_key(extension)?).copied()
    }
}
pub fn tcolor_sort_key(color: Rgba) -> u32 {
    (color[2] as u32) << 16 | (color[1] as u32) << 8 | color[0] as u32
}
#[derive(Clone, Debug)]
pub struct Skin {
    pub values: BTreeMap<String, String>,
    pub colors: BTreeMap<String, Rgba>,
    directory: PathBuf,
}
impl Skin {
    pub fn load(directory: &Path, theme: &str) -> Result<Self, String> {
        let theme = theme.trim().to_ascii_lowercase();
        if !matches!(theme.as_str(), "black" | "classic" | "gray") {
            return Err(format!("No original skin for theme: {theme}"));
        }
        let directory = std::fs::canonicalize(directory.join(theme)).map_err(|e| e.to_string())?;
        let bytes = std::fs::read(directory.join("conf.ini")).map_err(|e| e.to_string())?;
        let text = crate::compat::decode(&bytes, false);
        let mut values = BTreeMap::new();
        let mut section = "";
        for line in text.lines().map(str::trim) {
            if line.is_empty()
                || line.starts_with([';', '#'])
                || line.to_ascii_uppercase().starts_with("NEXUSFILESKIN")
            {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line[1..line.len() - 1].trim();
                continue;
            }
            if !section.is_empty()
                && let Some((key, value)) = line.split_once('=')
                && !key.trim().is_empty()
            {
                values.insert(
                    format!("{section}.{}", key.trim()).to_lowercase(),
                    value.trim().trim_matches('"').to_owned(),
                );
            }
        }
        let colors = values
            .iter()
            .filter_map(|(key, value)| parse_color(value).map(|color| (key.clone(), color)))
            .collect();
        Ok(Self {
            directory,
            values,
            colors,
        })
    }
    pub fn color(&self, section: &str, key: &str) -> Option<Rgba> {
        self.colors
            .get(&format!("{section}.{key}").to_lowercase())
            .copied()
    }
    pub fn positive_integer(&self, section: &str, key: &str, fallback: u32) -> u32 {
        self.values
            .get(&format!("{section}.{key}").to_lowercase())
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|v| (1..=256).contains(v))
            .unwrap_or(fallback)
    }
    pub fn asset(&self, section: &str, key: &str) -> Option<PathBuf> {
        self.image_path(
            self.values
                .get(&format!("{section}.{key}").to_lowercase())?,
        )
    }
    /// Resolve only existing files beneath the canonical skin directory, including a symlink escape check.
    pub fn image_path(&self, file_name: &str) -> Option<PathBuf> {
        let file_name = file_name.trim().trim_matches('"');
        if file_name.is_empty()
            || file_name.contains('\0')
            || Path::new(file_name).is_absolute()
            || file_name.contains(':')
        {
            return None;
        }
        let path = std::fs::canonicalize(self.directory.join(file_name)).ok()?;
        if path.starts_with(&self.directory) && path.is_file() {
            Some(path)
        } else {
            None
        }
    }
}
/// Original skin BMPs (toolbar_back.bmp, btnbar_back.bmp, btnbar_divider.bmp): only uncompressed 24/32-bit BITMAPINFOHEADER.
/// (width, height, RGBA top to bottom). A 32-bit image whose alpha is all 0 is treated as opaque (GDI BitBlt ignores alpha).
pub fn decode_bmp(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let u16_at = |at: usize| bytes.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |at: usize| bytes.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if bytes.get(..2) != Some(b"BM") {
        return Err("Not a BMP file".into());
    }
    let (offset, header, width, height, bits, compression) = (
        u32_at(10).ok_or("Truncated BMP")? as usize,
        u32_at(14).ok_or("Truncated BMP")?,
        u32_at(18).ok_or("Truncated BMP")? as i32,
        u32_at(22).ok_or("Truncated BMP")? as i32,
        u16_at(28).ok_or("Truncated BMP")?,
        u32_at(30).ok_or("Truncated BMP")?,
    );
    if header < 40 || compression != 0 || !matches!(bits, 24 | 32) || width <= 0 || height == 0 || width > 4096 || height.unsigned_abs() > 4096 {
        return Err("Unsupported BMP".into());
    }
    let (width, rows) = (width as usize, height.unsigned_abs() as usize);
    let channels = bits as usize / 8;
    let stride = (width * channels).div_ceil(4) * 4;
    let data = bytes.get(offset..offset + stride * rows).ok_or("Truncated BMP pixels")?;
    let mut rgba = vec![0u8; width * rows * 4];
    let mut any_alpha = false;
    for row in 0..rows {
        // A positive height means rows are stored bottom-up.
        let source = if height > 0 { rows - 1 - row } else { row };
        for x in 0..width {
            let p = &data[source * stride + x * channels..][..channels];
            let alpha = if channels == 4 { p[3] } else { 255 };
            any_alpha |= channels == 4 && alpha != 0;
            rgba[(row * width + x) * 4..][..4].copy_from_slice(&[p[2], p[1], p[0], alpha]);
        }
    }
    if channels == 4 && !any_alpha {
        rgba.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    Ok((width, rows, rgba))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_skin_bitmaps_decode() {
        for (file, size) in [("toolbar_back.bmp", (8, 51)), ("btnbar_back.bmp", (8, 22)), ("btnbar_divider.bmp", (1, 14))] {
            let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/Resources/Skins/black/").to_owned() + file).unwrap();
            let (width, height, rgba) = decode_bmp(&bytes).unwrap();
            assert_eq!((width, height, rgba.len()), (size.0, size.1, size.0 * size.1 * 4), "{file}");
            assert!(rgba.chunks_exact(4).all(|p| p[3] == 255), "{file} is opaque");
        }
        assert!(decode_bmp(b"PNG").is_err());
    }
    #[test]
    fn original_rgb_argb_and_color_map_roundtrip() {
        assert_eq!(parse_color("#80112233"), Some([0x11, 0x22, 0x33, 0x80]));
        assert_eq!(parse_color("FF0080"), Some([255, 0, 128, 255]));
        assert_eq!(parse_color("broken"), None);
        let map = ColorMap::parse(
            "[NEXUSFILECOLOR5]\r\nnxback=112233\r\n[Ext]\r\n*.TXT=ABCDEF\r\nbad?=112233\r\n",
        )
        .unwrap();
        assert_eq!(map.element("NXBACK"), Some([17, 34, 51, 255]));
        assert_eq!(map.extension(".txt"), Some([171, 205, 239, 255]));
        assert!(map.extension("bad?").is_none());
        let reloaded = ColorMap::parse(&map.serialize()).unwrap();
        assert_eq!(reloaded.extension("TXT"), map.extension("txt"));
        assert_eq!(map.element("NXSELECT"), Some([255, 255, 255, 255]));
        assert!(ColorMap::parse("[Ext]\r\nTXT=broken\r\n").is_err());
    }
    #[test]
    fn real_original_black_skin_values_load() {
        let skin = Skin::load(
            Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/Resources/Skins")),
            "Black",
        )
        .unwrap();
        assert_eq!(skin.color("Menu", "mmainback"), Some([42, 42, 42, 255]));
        assert_eq!(skin.positive_integer("Toolbar", "tbheight", 10), 51);
        assert!(skin.asset("Toolbar", "tbbackimg").is_some());
    }
    #[test]
    fn skin_assets_reject_escape_and_integer_limits() {
        let root = std::env::temp_dir().join(format!("ndr-style-{}", std::process::id()));
        let black = root.join("black");
        std::fs::create_dir_all(&black).unwrap();
        std::fs::write(root.join("outside.bin"), "owned outside fixture").unwrap();
        std::fs::write(black.join("inside.bin"), "owned inside fixture").unwrap();
        std::fs::write(black.join("conf.ini"),"NEXUSFILESKIN1.0\r\n[Test]\r\nOutside=../outside.bin\r\nInside=inside.bin\r\nTooLarge=257\r\nOkay=256\r\nAlpha=80112233\r\n").unwrap();
        let skin = Skin::load(&root, "black").unwrap();
        assert!(skin.asset("Test", "Outside").is_none());
        assert!(skin.asset("Test", "Inside").is_some());
        assert_eq!(skin.positive_integer("Test", "TooLarge", 5), 5);
        assert_eq!(skin.positive_integer("Test", "Okay", 5), 256);
        assert_eq!(skin.color("Test", "Alpha"), Some([17, 34, 51, 128]));
        assert!(Skin::load(&root, "../black").is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
