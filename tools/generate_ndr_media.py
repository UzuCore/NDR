"""Generate NDR-owned skin sprites, palette presets, macro icons and completion audio.

Run with Python 3.10+ and Pillow. The generator never reads existing assets.
Only geometry/slot counts and config keys needed by the app are kept compatible.
"""
from pathlib import Path
import math
import struct
import wave

from PIL import Image, ImageDraw


ROOT = Path(__file__).resolve().parents[1]
RES = ROOT / "Resources"
S = 4


def rgb(hex_string):
    return tuple(bytes.fromhex(hex_string))


def mix(a, b, t):
    return tuple(round(x * (1 - t) + y * t) for x, y in zip(a, b))


class Sprite:
    def __init__(self, size, background=None):
        self.image = Image.new("RGBA", (size[0] * S, size[1] * S), background)
        self.draw = ImageDraw.Draw(self.image)
        self.size = size

    @staticmethod
    def coords(items):
        return tuple(round(x * S) for x in items)

    def line(self, points, color, width=1.5):
        self.draw.line([self.coords(p) for p in points], fill=color, width=round(width*S), joint="curve")

    def rect(self, box, fill=None, outline=None, radius=0, width=1):
        self.draw.rounded_rectangle(self.coords(box), radius=round(radius*S), fill=fill,
                                    outline=outline, width=max(1, round(width*S)))

    def ellipse(self, box, color, fill=True, width=1.4):
        self.draw.ellipse(self.coords(box), fill=color if fill else None,
                          outline=None if fill else color, width=round(width*S))

    def arc(self, box, start, end, ink):
        self.draw.arc(self.coords(box), start, end, fill=ink, width=round(1.45*S))

    def result(self):
        return self.image.resize(self.size, Image.Resampling.LANCZOS)


def atlas(path, tile_size, cols, rows, draw):
    w, h = tile_size
    out = Image.new("RGBA", (w*cols, h*rows))
    for row in range(rows):
        for col in range(cols):
            icon = Sprite(tile_size)
            draw(icon, row, col)
            out.paste(icon.result(), (col*w, row*h))
    out.save(path)


def tile_bmp(path, size, top, base, edge):
    w, h = size
    out = Image.new("RGB", size)
    for y in range(h):
        c = edge if y == h-1 else mix(top, base, y / max(1, h-2))
        for x in range(w):
            out.putpixel((x, y), c)
    out.save(path)


def arrow(icon, center, direction, ink, stem=False):
    x, y = center
    dx, dy = direction
    side = (-dy, dx)
    pt = lambda shift, across: (x + dx*shift + side[0]*across, y + dy*shift + side[1]*across)
    icon.line([pt(-1.5, -3.2), pt(1.7, 0), pt(-1.5, 3.2)], ink)
    if stem:
        icon.line([pt(-4.5, 0), pt(1.2, 0)], ink)


# Skin schemes are independently chosen. The light scheme uses blue-gray chrome;
# the dark scheme uses graphite chrome with sea-glass focus and muted amber labels.
SKINS = {
    "black": {
        "surface": "161C23", "chrome": "202933", "raised": "2D3946", "edge": "4B5B6C",
        "ink": "D4DEE8", "strong": "F4FAFF", "soft": "A5B6C8", "muted": "78899C",
        "accent": "8FE5D2", "amber": "F2BD72", "active": "344C56", "back": "11171E",
        "local": "5AAFA2", "network": "70A9DB", "archive": "AD8751", "hover": "324553",
    },
    "classic": {
        "surface": "EDF2F8", "chrome": "E1E9F3", "raised": "F8FBFE", "edge": "A9B9CB",
        "ink": "25354A", "strong": "102842", "soft": "445D74", "muted": "697D92",
        "accent": "155B85", "amber": "84501B", "active": "D6E7F6", "back": "F7FAFD",
        "local": "377F79", "network": "4775AC", "archive": "846139", "hover": "C6D9EE",
    },
}


def skin_config(path, skin, dark):
    c = skin
    values = {
        "Menu": {
            "mmainback": c["chrome"], "mmaintext": c["ink"], "mmainoverback": c["hover"],
            "mmainover": c["strong"], "mback": c["surface"], "mborder": c["edge"],
            "mtext": c["ink"], "mdisabled": c["muted"], "mhighlight": c["hover"],
            "mover": c["strong"], "mrback": c["surface"], "mrtext": c["soft"],
            "mrover": c["accent"], "mdivider": c["edge"],
        },
        "Toolbar": {
            "tbbackimg": "toolbar_back.bmp", "tbbtnwidth": 35 if dark else 20,
            "tbbtnheight": 35 if dark else 20, "tbheight": 51 if dark else 36,
            "tbtopoffset": 7 if dark else 8, "tbtext": c["ink"],
        },
        "List": {
            "ldrivesback": c["surface"], "ldrivestext": c["ink"], "ldriveshot": c["accent"], "ldrivesframe": c["edge"],
            "labtnimgs": "addressbtns.png", "labtnheight": 18, "labtnwidth": 18, "laback": c["surface"],
            "latext": c["soft"], "laover": c["accent"], "lacurr": c["strong"], "ladivider": c["edge"],
            "lafocus": c["active"], "lfileback": c["surface"], "lfiletext": c["ink"], "lfilesubtext1": c["amber"],
            "lfolderback": c["back"], "lfoldertext": c["ink"], "lfoldersubtext1": c["accent"],
            "lfolderdivider": c["edge"], "ldisksubtext1": c["soft"], "lframelocal": c["local"],
            "lframenet": c["network"], "lframearc": c["archive"], "lframeftp": c["network"],
            "lframeinactive": c["edge"], "lframetree": c["chrome"], "lframewidth": 1,
            "lselinfotext": c["accent"], "lback": c["back"],
            "tabback": c["back"], "tabtext": c["soft"], "tabhot": c["strong"], "tabactive": c["strong"],
            "tabimgs": "tab.png", "tabbtnimgs": "tabbtns.png",
            "scrollbg": "scroll_bg.png", "scrollbtn": "scroll_btn.png", "scrollthumb": "scroll_thumb.png",
        },
        "VertBar": {
            "vbback": c["back"], "vbbtnheight": 16, "vbbtnwidth": 16,
            "vbbtnimgs": "vertbarbtns.png", "vbbtnoverimgs": "vertbarbtns_over.png",
            "vbwfimgset": "workfolders_set.png", "vbwfimgunset": "workfolders_unset.png",
            "vbwfimgover": "workfolders_over.png", "vbthumb": "vertthumb.png",
        },
        "ButtonBar": {
            "bbbackimg": "btnbar_back.bmp", "bbdividerimg": "btnbar_divider.bmp",
            "bbkeytext": c["amber"], "bbtext": c["ink"], "bbover": c["accent"],
        },
        "FTP": {"ftplogback": c["back"], "ftplogtext": c["ink"], "ftpprogback": c["back"], "ftpprogtext": c["ink"]},
    }
    lines = ["NDRSKIN1.0", "; NDR-owned renderer configuration. See tools/generate_ndr_media.py."]
    for section, pairs in values.items():
        lines.extend(["", f"[{section}]", *(f"{k}={v}" for k, v in pairs.items())])
    path.write_text("\n".join(lines) + "\n", encoding="ascii", newline="\n")


def create_skin(name, conf):
    dark = name == "black"
    path = RES / "Skins" / name
    palette = {k: rgb(v) for k, v in conf.items()}
    skin_config(path / "conf.ini", conf, dark)
    surface, raised, chrome, edge, ink, soft, glow, hot, active = (
        palette[k] for k in ("surface", "raised", "chrome", "edge", "ink", "soft", "accent", "hover", "active")
    )

    def pad(icon, w, h, hovered, border=6):
        if hovered:
            icon.rect((1.5, 1.5, w-2, h-2), fill=hot, radius=border/2)

    def toolbar(icon, row, col):
        # All geometry here is authored at 20px logical size. The larger
        # Black atlas has the same accessible silhouettes in a separate raster.
        side = icon.size[0]
        sym = Sprite((20, 20))
        sym.rect((1.4, 1.4, 18.3, 18.3), fill=surface, outline=edge, radius=5, width=0.8)
        c = glow
        if col in (0, 1):
            arrow(sym, (9.95, 10), (-1 if col == 0 else 1, 0), c, stem=True)
        elif col == 2:  # desktop monitor
            sym.rect((5, 5.3, 14.8, 11.9), outline=c, radius=1.2, width=1.2)
            sym.line([(9.9, 12.3), (9.9, 14.4)], c, 1.1)
            sym.line([(7.5, 14.4), (12.4, 14.4)], c, 1.1)
        elif col == 3:  # documents, folded corner
            sym.line([(6.6, 14.7), (6.6, 4.6), (10.7, 4.6), (13.2, 7.2), (13.2, 14.7), (6.6, 14.7)], c, 1.2)
            sym.line([(10.2, 4.8), (10.2, 7.65), (13, 7.65)], c, 0.85)
            sym.line([(8.1, 10.4), (11.6, 10.4)], c, 0.9)
            sym.line([(8.1, 12.5), (11.6, 12.5)], c, 0.9)
        elif col == 4:  # pictures, frame and mountain
            sym.rect((5, 5.6, 14.7, 13.8), outline=c, radius=1.1, width=1.1)
            sym.ellipse((7.35, 7.25, 8.65, 8.55), c)
            sym.line([(5.65, 12.55), (8.25, 10.2), (10, 11.5), (12.3, 8.7), (14.05, 10.4)], c, 1.0)
        elif col == 5:  # music, double note
            sym.line([(8, 13.6), (8, 7), (13, 5.2), (13, 12)], c, 1.25)
            sym.ellipse((5.55, 12.1, 8.35, 14.25), c)
            sym.ellipse((10.65, 10.6, 13.4, 12.7), c)
        elif col == 6:  # videos, play
            sym.rect((4.8, 5.8, 15, 14), outline=c, radius=1.5, width=1.1)
            sym.draw.polygon([sym.coords(x) for x in ((8.6, 7.75), (12.3, 10), (8.6, 12.3))], fill=c)
        elif col == 7:  # favorites, bookmark
            sym.line([(6.6, 14.65), (6.6, 5.05), (13.3, 5.05), (13.3, 14.65), (9.95, 12.2), (6.6, 14.65)], c, 1.3)
            sym.line([(8.6, 7.65), (11.35, 7.65)], c, 0.85)
        elif col == 8:  # recycle bin
            sym.line([(6.8, 8.1), (7.35, 14.1), (12.6, 14.1), (13.15, 8.1)], c, 1.3)
            sym.line([(5.85, 7.1), (14.1, 7.1)], c, 1.3)
            sym.line([(8.6, 6.85), (8.6, 5.65), (11.35, 5.65), (11.35, 6.85)], c, 1.05)
        else:  # neutral icon for a command with no assigned graphic
            for x in (7, 10, 13):
                sym.ellipse((x-0.65, 9.25, x+0.65, 10.55), c)
        icon.image.paste(sym.image.resize((side*S, side*S), Image.Resampling.LANCZOS), (0, 0))

    side = 35 if dark else 20
    atlas(path / "toolbar-icons.png", (side, side), 10, 1, toolbar)

    def address(icon, hovered, col):
        pad(icon, 18, 18, hovered)
        color = glow if hovered else ink
        if col == 0:  # history: short stacked bars over downward mark
            icon.line([(5, 5.5), (12, 5.5)], color, 1.25)
            arrow(icon, (8.5, 10.8), (0, 1), color)
        else:  # refresh
            icon.arc((4.5, 4.5, 13, 13), 10, 307, color)
            icon.line([(12.7, 4.4), (12.8, 7.4), (10, 7.4)], color, 1.25)
    atlas(path / "addressbtns.png", (18, 18), 2, 2, address)

    def tab_buttons(icon, hovered, col):
        pad(icon, 12, 12, hovered, 4)
        color = glow if hovered else ink
        if col == 0:
            for y in (3.5, 6, 8.5):
                icon.ellipse((2.25, y-0.55, 3.35, y+0.55), color)
                icon.line([(4.75, y), (9.75, y)], color, 1)
        elif col in (1, 2):
            arrow(icon, (5.9, 5.75), (-1 if col == 1 else 1, 0), color)
        else:
            icon.line([(3.4, 3.3), (8.4, 8.3)], color, 1.45)
            icon.line([(8.4, 3.3), (3.4, 8.3)], color, 1.45)
    atlas(path / "tabbtns.png", (12, 12), 4, 2, tab_buttons)

    def tab(icon, row, col):
        width, _ = icon.size
        fill = [active, surface, hot][row]
        icon.rect((0, 1.5, width-0.25, 23), fill=fill, outline=edge, radius=3.2)
        icon.line([(2, 22), (width-2, 22)], glow if row == 0 else edge, 1.25)
    atlas(path / "tab.png", (29 if dark else 20, 24), 1, 3, tab)

    def scroll_back(icon, row, col):
        icon.rect((0, 0, 11.75, 7.75), fill=surface)
        # Constant along the long axis to avoid stretching/slice seams.
        icon.line([(0.6, 0), (0.6, 7.75)], edge, 0.8)
    atlas(path / "scroll_bg.png", (12, 8), 1, 1, scroll_back)

    def scroll_button(icon, row, pressed):
        icon.rect((1, 1, 10.5, 14.5), fill=hot if pressed else chrome, outline=edge, radius=2)
        arrow(icon, (5.75, 7.45), (0, -1), glow if pressed else ink)
    atlas(path / "scroll_btn.png", (12, 16), 2, 1, scroll_button)

    thumb = Sprite((12, 22))
    thumb.rect((2, 0.5, 9.5, 21), fill=raised, outline=edge, radius=3)
    thumb.line([(5.75, 4), (5.75, 17.5)], soft, 0.75)
    thumb.result().save(path / "scroll_thumb.png")

    def strip(icon, hovered, col):
        pad(icon, 16, 16, hovered)
        color = glow if hovered else ink
        if col in (0, 1):
            # Directional path-bridge mark; fits the 16x12 visible crop.
            arrow(icon, (8, 6.7), (1 if col == 0 else -1, 0), color, stem=True)
            x = 13 if col == 0 else 2.75
            icon.line([(x, 4), (x, 9.25)], color, 1.2)
        elif col in (2, 3):
            icon.rect((3.3, 3.2, 12.5, 12.4), outline=color, radius=2)
            if col == 2:
                icon.line([(5.35, 7.65), (7, 9.3), (10.6, 5.4)], glow if not hovered else color, 1.35)
            else:
                icon.line([(5.65, 7.75), (10.25, 7.75)], color, 1.3)
        elif col in (4, 5):
            # Copy action: document corner plus direction (distinct from path sync).
            direction = 1 if col == 4 else -1
            x = 2.7 if col == 4 else 8.4
            icon.rect((x, 4, x+4.65, 12.15), outline=color, radius=1.2)
            arrow(icon, (8.75 if col == 4 else 7.2, 7.95), (direction, 0), color, stem=False)
        elif col == 6:
            for y, width in ((4.25, 9), (7.75, 6), (11.25, 3.25)):
                icon.line([(3.4, y), (3.4+width, y)], color)
        elif col == 8:
            icon.rect((3, 4, 12.5, 11.7), outline=color, radius=1)
            icon.line([(7.75, 4), (7.75, 11.7)], color)
        else:
            icon.ellipse((5.4, 5.4, 10.2, 10.2), color, False)
    for hovered, filename in ((False, "vertbarbtns.png"), (True, "vertbarbtns_over.png")):
        atlas(path / filename, (16, 16), 10, 1, lambda icon, row, col: strip(icon, hovered, col))

    for state in ("unset", "set", "over"):
        def work(icon, row, col, state=state):
            pad(icon, 16, 16, state == "over")
            color = {"unset": soft, "set": glow, "over": ink}[state]
            icon.line([(2.75, 11), (2.75, 4.5), (6.65, 4.5), (7.85, 6), (13.2, 6), (13.2, 11), (2.75, 11)], color, 1)
            # A marker makes the assigned and unassigned state distinguishable without hue.
            if state == "set":
                icon.ellipse((6.35, 7.15, 9.35, 10), color)
            elif state == "over":
                icon.line([(6.4, 8.65), (9.6, 8.65)], color, 1)
            # Each cell has its own lower position indicator; users can still set/clear it.
            for i in range(5):
                x = 3 + 2.25*i
                icon.line([(x, 13.5), (x + (1.1 if i == col else 0.4), 13.5)], color, 1)
        atlas(path / f"workfolders_{state}.png", (16, 16), 5, 1, work)

    drag = Sprite((10, 21 if dark else 20))
    for y in (5.15, 9.9, 14.65):
        for x in (3.4, 6.2):
            drag.ellipse((x-0.8, y-0.8, x+0.8, y+0.8), soft)
    drag.result().save(path / "vertthumb.png")
    tile_bmp(path / "toolbar_back.bmp", (8, 51) if dark else (10, 36), raised, chrome, edge)
    tile_bmp(path / "btnbar_back.bmp", (8, 22), chrome, surface, edge)
    tile_bmp(path / "btnbar_divider.bmp", (1 if dark else 2, 14), chrome, edge, chrome)


BASIC = (
    "NXBACK NXFORE NXFOLDER NXDRIVE NXREADONLY NXHIDDEN NXDESC NXMB NXGB NXSIZE NXTIME "
    "NXATTR NXSELECT NXLINE NXTBACK NXTFORE NXTOPENED NXTFOCUSED NXTLINE NXTHINT"
).split()

# Palette architecture is role-based, with file families defined by their format
# rather than a copied extension sequence/table. nocolor intentionally has no
# extension overrides.
PRESETS = {
    "black": "151B24 DBE4EE 8EDDCF AEC9FB F3CF7D DB9EA2 A9BBD0 E8BD8A F1A276 AAC3D5 A7B5CA AEBFCE B9E0E9 2A3748 111722 D5E0ED ABE6D9 8FC5F0 42536A A3AFBF",
    "blue": "172D49 E9F1FB AEE0FF C5CAFF F5DFA1 EFADB8 B5D0F2 FFCE9A FFAC8A C4D8F0 BACCE6 BFCEE1 BBDDEF 33506F 14283D E1EFFC A4ECED BCD1FF 4B6D90 AEC4DA",
    "brown": "392B22 F5E8DC F6D4A4 F3BFA4 FBD687 EDB5AA E4C0A5 F4CE9F F0A57B E3CDBA E7C4AC DBC1AD F7D3AA 624C3D 30241D F5E4D6 F6CD9C EFD5B9 705744 CFB39D",
    "white1": "FFFCF8 293344 2C596D 3D5278 76511E 8B5361 536A82 74613E 92512B 526078 596B7C 536475 3D6B83 E4E8EB FAF7F2 293745 32566D 264D7D CAD7E0 4C6075",
    "white2": "F5FAFF 23324A 285C95 554985 80571F 8F4A68 385779 65492F 8E4826 455876 50637F 4D607C 456FA4 D7E3F0 F0F7FF 23324A 215991 2E4B8C BED1E7 4C6280",
    "white3": "F7FBF6 26372F 326D5A 365D71 78531B 884E58 466656 6E4E2C 874624 486358 4C6356 4E6258 477B65 DCE8DE F2F8F1 2C3D34 2E664D 245A5A C1D4CA 4B6759",
    "nocolor": "FAFAFB 2E343B 2E343B 2E343B 555D66 626973 545B65 555D66 555D66 515961 515961 515961 566672 E1E5E9 F4F6F7 303740 404A55 38434F C9D1D9 59636F",
}
# Existing extension slots remain grouped and sorted as before. The swatches
# themselves belong to NDR and are authored in COLORREF rank order.
PRESET_EXTENSION_SLOTS = {
    "black": (
        ("D75F3A", "BAK TMP"),
        ("C78D48", "CHM HLP"),
        ("EFC961", "7Z ACE ALZ ARJ CAB EGG GZ ISO LZH RAR TAR ZIP"),
        ("B6D668", "BAT CMD LNK PIF"),
        ("EAAA72", "MID MP2 MP3 OGG WAV WMA"),
        ("F2AB82", "ASF AVI K3G MKV MOV MP4 MPEG MPG RAM RM SKM SWF TP TS WMV"),
        ("7BD28A", "EXE MSI"),
        ("75C49A", "CAP DOC DOCX GAL GUL HWP PDF RTF TXT"),
        ("A6BCAD", "CFG INF INI REG"),
        ("7CB6CC", "PPT PPTX XLS XLSX"),
        ("81B5E2", "AS ASP C CPP CS H HPP JS JSP PAS PHP PL VB"),
        ("E0A7EE", "BMP GIF JPEG JPG PNG PSD PSP TIF WMF"),
        ("C49CF5", "DLL SYS"),
        ("A2B8FC", "HTM HTML URL"),
    ),
    "blue": (
        ("F07356", "BAK TMP"),
        ("DF9A5E", "CHM HLP"),
        ("F0BC67", "7Z ACE ALZ ARJ CAB EGG GZ ISO LZH RAR TAR ZIP"),
        ("C6DB75", "BAT CMD LNK PIF"),
        ("ECAE83", "MID MP2 MP3 OGG WAV WMA"),
        ("BFDB96", "ASF AVI K3G MKV MOV MP4 MPEG MPG RAM RM SKM SWF TP TS WMV"),
        ("F0B7A1", "EXE MSI"),
        ("93DDB3", "CAP DOC DOCX GAL GUL HWP PDF RTF TXT"),
        ("BBCBC6", "CFG INF INI REG"),
        ("A8CCE1", "PPT PPTX XLS XLSX"),
        ("8DCBEC", "BMP GIF JPEG JPG PNG PSD PSP TIF WMF"),
        ("C6C6F5", "DLL SYS"),
        ("E9BAF9", "HTM HTML URL"),
    ),
    "brown": (
        ("ED8863", "ASF AVI K3G MKV MOV MP4 MPEG MPG RAM RM SKM SWF TP TS WMV"),
        ("F2A276", "CHM HLP"),
        ("D8BC7B", "7Z ACE ALZ ARJ CAB EGG GZ ISO LZH RAR TAR ZIP"),
        ("F2CD82", "BAT CMD LNK PIF"),
        ("E7B88B", "MID MP2 MP3 MWA OGG WAV"),
        ("C5D693", "EXE MSI"),
        ("EAC5A8", "BAK TMP"),
        ("C6D9B3", "CAP DOC DOCX GAL GUL HWP PDF RTF TXT"),
        ("F1C2C6", "CFG INF INI REG"),
        ("D2D4D2", "PPT XLS"),
        ("BFD4E0", "DLL SYS"),
        ("E2C9ED", "BMP GIF JPEG JPG PNG PSD PSP TIF WMF"),
        ("CFDDF4", "HTM HTML URL"),
    ),
    "nocolor": (
    ),
    "white1": (
        ("823A24", "BAK TMP"),
        ("855835", "CHM HLP"),
        ("785136", "7Z ACE ALZ ARJ CAB EGG GZ ISO LZH RAR TAR ZIP"),
        ("67583F", "BAT CMD LNK PIF"),
        ("8D5B48", "MID MP2 MP3 OGG WAV WMA"),
        ("7A5E56", "ASF AVI K3G MKV MOV MP4 MPEG MPG RAM RM SKM SWF TP TS WMV"),
        ("35605C", "EXE MSI"),
        ("92686B", "CAP DOC DOCX GAL GUL HWP PDF RTF TXT"),
        ("36707E", "CFG INF INI REG"),
        ("5E6B8C", "PPT PPTX XLS XLSX"),
        ("585198", "DLL SYS"),
        ("355E9F", "AI BMP GIF JPEG JPG PNG PSD PSP TIF WMF"),
        ("584FA9", "HTM HTML URL"),
    ),
    "white2": (
        ("85422B", "BAK TMP"),
        ("6A5C3C", "CHM HLP"),
        ("735942", "7Z ACE ALZ ARJ CAB EGG GZ ISO LZH RAR TAR ZIP"),
        ("536948", "BAT CMD LNK PIF"),
        ("9B594D", "MID MP2 MP3 OGG WAV WMA"),
        ("45625B", "ASF AVI K3G MKV MOV MP4 MPEG MPG RAM RM SKM SWF TP TS WMV"),
        ("3A6568", "EXE MSI"),
        ("5E6872", "CAP DOC DOCX GAL GUL HWP PDF RTF TXT"),
        ("416B83", "CFG INF INI REG"),
        ("506895", "PPT PPTX XLS XLSX"),
        ("465396", "DLL SYS"),
        ("59489D", "AI BMP GIF JPEG JPG PNG PSD PSP TIF WMF"),
        ("5964AC", "HTM HTML URL"),
    ),
    "white3": (
        ("85442C", "MID MP2 MP3 OGG WAV WMA"),
        ("80613B", "BAK TMP"),
        ("646442", "CHM HLP"),
        ("546454", "ASF AVI BMP GIF JPEG JPG K3G MKV MOV MP4 MPEG MPG PNG PSD PSP RAM RM SKM SWF TIF TP TS WMF WMV"),
        ("746365", "BAT CMD LNK PIF"),
        ("586970", "EXE MSI"),
        ("3F687C", "CFG DLL INF INI REG SYS"),
        ("576A85", "CAP DOC DOCX GAL GUL HWP PDF RTF TXT"),
        ("4B668D", "7Z ACE ALZ ARJ CAB EGG GZ ISO LZH RAR TAR ZIP"),
        ("56669A", "PPT PPTX XLS XLSX"),
        ("56579F", "COM"),
        ("5753AD", "HTM HTML URL"),
    ),
}


def colors():
    for name, basics in PRESETS.items():
        lines = ["[NDRCOLOR1]", "; NDR color preset: file roles and extension groups", *(
            f"{key}={color}" for key, color in zip(BASIC, basics.split(), strict=True)
        ), "", "[Ext]"]
        ext_values = {ext: color for color, group in PRESET_EXTENSION_SLOTS[name] for ext in group.split()}
        lines.extend(f"{ext}={color}" for ext, color in sorted(ext_values.items()))
        (RES / "Colors" / f"{name}.col").write_text("\n".join(lines) + "\n", encoding="utf-8-sig", newline="\n")


def macro_glyphs():
    for save in (True, False):
        # Existing bitmap loader uses the bottom-left color as a transparent key.
        key = (255, 0, 255)
        icon = Sprite((16, 16), key)
        if save:
            c = (22, 119, 92)
            icon.line([(3, 10.3), (3, 13), (12.7, 13), (12.7, 10.3)], c, 1.65)
            arrow(icon, (7.9, 7.75), (0, 1), c, stem=True)
        else:
            c = (190, 43, 61)
            icon.rect((4.5, 5.2, 11.5, 13), outline=c, radius=1.1, width=1.5)
            icon.line([(3.3, 4.05), (12.7, 4.05)], (218, 54, 73), 1.65)
            icon.line([(6.3, 3), (6.3, 2.3), (9.65, 2.3), (9.65, 3)], c, 1.25)
            icon.line([(6.8, 7), (6.8, 10.5)], c, 1.15)
            icon.line([(9.35, 7), (9.35, 10.5)], c, 1.15)
        quant = Image.new("RGB", (16, 16), key)
        for y in range(16):
            for x in range(16):
                # Use the unfiltered coverage mask because the app uses a 1-bit key.
                p = icon.image.getpixel((x*S+S//2, y*S+S//2))[:3]
                quant.putpixel((x, y), p)
        quant.save(ROOT / "src" / "ui" / "glyphs" / ("advrename-save.bmp" if save else "advrename-delete.bmp"))


def complete():
    # An original, soft two-note rising cue. All samples are synthesized here.
    rate = 44100
    samples = []
    for i in range(round(rate * 0.315)):
        t = i / rate
        y = 0.0
        for start, end, f, amp in ((0.0, 0.159, 659.25, 0.22), (0.067, 0.315, 987.77, 0.28)):
            if start <= t < end:
                dt = t-start
                ease = min(1.0, dt/0.014, (end-t)/0.045)
                gain = math.sin(ease * math.pi/2)**2 * math.exp(-6.5*dt)
                phase = math.tau*f*dt
                y += amp*gain*(math.sin(phase) + 0.12*math.sin(phase*2))
        samples.append(round(y*32767))
    with wave.open(str(RES / "complete.wav"), "wb") as out:
        out.setparams((1, 2, rate, 0, "NONE", "not compressed"))
        out.writeframes(struct.pack(f"<{len(samples)}h", *samples))


if __name__ == "__main__":
    for name, conf in SKINS.items():
        create_skin(name, conf)
    colors()
    macro_glyphs()
    complete()
