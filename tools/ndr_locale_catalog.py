"""Register translatable NDR UI phrases or merge authored locale rows.

Run from any directory with Python 3.11+. Does not access the network or an
installed NDR profile. --write only edits the checked-out English/Korean
catalogs. Other translators can copy individual NDR keys into their catalogs.
"""
import argparse
import collections
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
CLEAN_LITERAL = r'"(?:[^"\\]|\\[\s\S])*"'
BILINGUAL = re.compile(
    r'\.(?:tr|warn|translate_pair)\(\s*(' + CLEAN_LITERAL
    + r')\s*,\s*(' + CLEAN_LITERAL + r')\s*,?\s*\)'
)
LEGACY_CHOICE = re.compile(r'\bif\s+(?:self\.language\s*==\s*"ko"|ko|korean)\s*\{')
RUST_ESCAPE = re.compile(r'\\(u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|\n[ \t\n]*|[\s\S])')


def decode_rust(text):
    def replace(match):
        code = match[1]
        if code.startswith("u{"):
            return chr(int(code[2:-1].replace("_", ""), 16))
        if code.startswith("x") and len(code) == 3:
            return chr(int(code[1:], 16))
        if code.startswith("\n"):
            return ""
        return {"n": "\n", "r": "\r", "t": "\t", "0": "\0", '"': '"', "\\": "\\", "'": "'"}[code]
    return RUST_ESCAPE.sub(replace, text[1:-1])


def literal_id(ko, en):
    result = 0xCBF29CE484222325
    for value in (ko + "\0" + en).encode("utf8"):
        result = ((result ^ value) * 0x100000001B3) & ((1 << 64) - 1)
    return f"UI_PAIR_{result:016X}"


def catalog_literal(value):
    return value.replace("&", "&&").replace("\r", "").replace("\n", "\\n")


def inspect_literals():
    phrases = {}
    conditional = {}
    for file in sorted((ROOT / "src/ui").glob("*.rs")):
        if file.name.endswith("_tests.rs") or file.name == "tests.rs":
            continue
        text = file.read_text(encoding="utf8")
        # Remove source comments without altering text in quoted literals.
        tokens = re.compile(CLEAN_LITERAL + r'|//[^\n]*|/\*.*?\*/', re.S)
        text = tokens.sub(lambda m: m[0] if m[0].startswith('"') else '\n' * m[0].count('\n'), text)
        for match in BILINGUAL.finditer(text):
            ko, en = map(decode_rust, match.groups())
            if not (ko.strip() and en.strip()) or "\0" in ko + en or "\t" in ko + en:
                continue
            key = literal_id(ko, en)
            pair = (catalog_literal(en), catalog_literal(ko))
            if key in phrases and phrases[key] != pair:
                raise ValueError(f"literal key collision: {key} ({file.name})")
            phrases[key] = pair
        count = len(LEGACY_CHOICE.findall(text))
        if count:
            conditional[file.name] = count
    return phrases, conditional


def load_sidecar(path):
    for number, line in enumerate(path.read_text(encoding="utf-8-sig").splitlines(), 1):
        if not line.strip() or line.startswith("#") or line.lower().startswith("section\t") or line.lower().startswith("key\t"):
            continue
        fields = line.split("\t")
        if len(fields) == 4 and fields[0].strip("[]").upper() == "NDR":
            fields.pop(0)
        if len(fields) != 3:
            raise ValueError(f"{path}:{number}: expected key<TAB>English<TAB>Korean")
        key, en, ko = fields
        key = key.removeprefix("NDR.")
        if not re.fullmatch(r"[A-Z0-9_]+", key) or not (en and ko):
            raise ValueError(f"{path}:{number}: invalid localization row {key!r}")
        yield key, (en, ko)


def merge(path, rows, write):
    raw = path.read_bytes()
    crlf = b"\r\n" in raw
    bom = raw.startswith(b"\xef\xbb\xbf")
    lines = raw.decode("utf-8-sig").splitlines()
    section = None
    indexed = {}
    section_end = None
    seen_section = False
    for index, line in enumerate(lines):
        text = line.strip()
        if text.startswith("[") and text.endswith("]"):
            if section == "ndr":
                section_end = index
            section = text[1:-1].casefold()
            if section == "ndr":
                assert not seen_section, f"{path}: duplicate [NDR]"
                seen_section = True
        elif section == "ndr" and text and not text.startswith(("#", ";")) and "=" in line:
            key = line.split("=", 1)[0].strip().upper()
            if key in indexed:
                raise ValueError(f"{path}: duplicate NDR key {key}")
            indexed[key] = index
    if not seen_section:
        lines += ["", "[NDR]"]
    section_end = len(lines) if section_end is None else section_end
    owned = set(rows)
    expected = {key: f"{key}={value}" for key, value in rows.items()}
    missing = [key for key in sorted(owned) if key not in indexed]
    changed = [
        key for key in sorted(owned & indexed.keys())
        if lines[indexed[key]] != expected[key]
    ]
    if write and (missing or changed):
        for key in changed:
            lines[indexed[key]] = expected[key]
        lines[section_end:section_end] = [expected[key] for key in missing]
        sep = "\r\n" if crlf else "\n"
        encoded = (sep.join(lines) + sep).encode("utf8")
        path.write_bytes((b"\xef\xbb\xbf" if bom else b"") + encoded)
    return len(missing), len(changed)


def main():
    args = argparse.ArgumentParser(description=__doc__)
    args.add_argument("--write", action="store_true", help="update stock en.lang and ko.lang")
    args.add_argument("--sidecar", action="append", type=pathlib.Path, default=[])
    args.add_argument("--locale-sidecar", action="append", type=pathlib.Path, default=[],
                      help="merge authored locale<TAB>key<TAB>value rows")
    opts = args.parse_args()
    rows, residual = inspect_literals()
    count = len(rows)
    for sidecar in opts.sidecar:
        for key, pair in load_sidecar(sidecar):
            if key in rows and rows[key] != pair:
                raise ValueError(f"conflicting authorship for {key}: {sidecar}")
            rows[key] = pair
    print(f"Registered literal pairs: {count}; authored explicit keys: {len(rows)-count}")
    for index, language in enumerate(("en", "ko")):
        path = ROOT / "Resources" / "Languages" / f"{language}.lang"
        counters = merge(path, {key: pair[index] for key, pair in rows.items()}, opts.write)
        print(f"{language}: {counters[0]} missing, {counters[1]} changed")
    overlays = collections.defaultdict(dict)
    locale_files = {path.stem: path for path in (ROOT / "Resources/Languages").glob("*.lang")}
    for sidecar in opts.locale_sidecar:
        for number, line in enumerate(sidecar.read_text("utf-8-sig").splitlines(), 1):
            if not line.strip() or line.startswith("#") or line.lower().startswith("locale\t"):
                continue
            fields = line.split("\t")
            if len(fields) != 3:
                raise ValueError(f"{sidecar}:{number}: expected locale<TAB>key<TAB>value")
            code, key, value = fields
            if code not in locale_files or code in {"en", "ko"} or key not in rows or not value:
                raise ValueError(f"{sidecar}:{number}: unknown locale/key: {code}/{key}")
            pattern = r"\{[A-Za-z0-9_]+\}|%[sd]|\%[0-9]+\$[sd]|\\n|</?[biu]>|<c[0-9A-Fa-f]{6}>|</c>"
            tokens = lambda text: collections.Counter(re.findall(pattern, text))
            if tokens(rows[key][0]) != tokens(value):
                raise ValueError(f"{sidecar}:{number}: changed format tokens for {code}/{key}")
            if key in overlays[code] and overlays[code][key] != value:
                raise ValueError(f"{sidecar}:{number}: conflicting row for {code}/{key}")
            overlays[code][key] = value
    for code, additions in sorted(overlays.items()):
        counters = merge(locale_files[code], additions, opts.write)
        print(f"{code}: {len(additions)} authored overlay rows ({counters[0]} new, {counters[1]} changed)")
    print("Bilingual conditional branches to review (includes logic checks, not a UI text count):")
    for module, count in sorted(residual.items(), key=lambda item: (-item[1], item[0])):
        print(f"  {module}: {count}")


if __name__ == "__main__":
    main()
