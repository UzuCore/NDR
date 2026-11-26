"""Inventory NDR's source strings and catalog gaps without hiding fallback.

All Rust files are scanned recursively. Reports are candidates for review,
not proof that a string is user-visible or that equal spellings are wrong.
"""
import argparse
import collections
import csv
import pathlib
import re
import subprocess

from ndr_locale_catalog import decode_rust, literal_id

ROOT = pathlib.Path(__file__).resolve().parent.parent
TOKEN = re.compile(
    r'//[^\n]*|/\*|(?:br|cr|r)(?P<hash>\#*)"(?P<raw>.*?)"(?P=hash)'
    r'|"(?:[^"\\]|\\[\s\S])*"|\'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^\'\\\n])\''
    r'|[A-Za-z_]\w*|[^\s]', re.S,
)
CATALOG_CALLS = {"text", "raw", "primary_raw", "localized", "operation_format", "root_text"}
PAIR_CALLS = {"tr", "warn", "translate_pair"}
UI_CALLS = {"label", "heading", "button", "checkbox", "hover_text", "on_hover_text", "selectable_label", "message_box", "system_box", "error_notice"}
KEY_PATTERN = re.compile(r'(?:RES|NDR(?:Vault|Update|RemoteSites|Search|SFTPHostKey|CopyHandler|Compatibility)?|Root|form\w+|Menu|ToolButton)\.[\w.]+', re.I)
LANG_BRANCH = re.compile(r'\bif\s+(?:self\.language\s*==\s*"ko"|ko|korean)\s*[{]')
EQUAL_MNEMONIC = re.compile(r'\s*\(&[A-Za-z0-9]\)')


def tokens(text):
    pos = 0
    while match := TOKEN.search(text, pos):
        pos = match.end()
        token = match[0]
        if token == "/*":
            level = 1
            while level and (edge := re.search(r'/\*|\*/', text[pos:])):
                pos += edge.end()
                level += 1 if edge[0] == "/*" else -1
            if level:
                break
        elif not token.startswith("//"):
            if match["raw"] is not None:
                yield (match.start(), token, match["raw"])
            elif token.startswith('"'):
                try:
                    yield (match.start(), token, decode_rust(token))
                except (KeyError, ValueError):
                    yield (match.start(), token, token[1:-1])
            else:
                yield (match.start(), token, None)


def scan():
    rows, coverage = [], []
    sources = [ROOT / "build.rs", *(ROOT / "src").rglob("*.rs"), *(ROOT / "tests").rglob("*.rs")]
    for path in sorted(p for p in sources if p.is_file()):
        src = path.read_text(encoding="utf-8-sig")
        prefix = path.relative_to(ROOT).as_posix()
        ts = list(tokens(src))
        # Deliberately missing fixture keys in inline test modules are not UI references.
        test_regions = []
        for j in range(len(ts) - 9):
            if [t[1] for t in ts[j:j+8]] == ['#','[','cfg','(','test',')',']','mod'] and ts[j+9][1] == '{':
                depth = 1
                for end in range(j+10, len(ts)):
                    if ts[end][2] is None:
                        depth += (ts[end][1] == '{') - (ts[end][1] == '}')
                    if depth == 0:
                        test_regions.append((ts[j][0], ts[end][0]))
                        break
        coverage.append((prefix, len(src.splitlines()), sum(value is not None for _, _, value in ts)))
        for i, (pos, token, value) in enumerate(ts):
            if value is None:
                continue
            kind, key = "literal", ""
            if KEY_PATTERN.fullmatch(value) and value.casefold() != 'ndr.ui_pair_':
                kind, key = "key-reference", value
                # These forms prefix their local keys before lookup. A bare
                # Menu.* here would be mistaken for the main menu catalog.
                if path.name == "folder_tree_dialog.rs" and value.startswith(("Menu.", "ToolButton.")):
                    key = "formFolderTree." + value
                elif path.name == "destination_dialog.rs" and value.startswith("Menu.pmi"):
                    key = "formSelectDest." + value
            left = [t[1] for t in ts[max(0, i - 9):i]]
            if left[-2:] == ['update_text','('] and re.fullmatch(r'[A-Za-z][A-Za-z0-9]*', value):
                kind, key = 'key-reference', 'NDRUpdate.' + value
            elif left[-2:] == ['root_text','('] and re.fullmatch(r'[A-Z][A-Z0-9_]*', value):
                kind, key = 'key-reference', 'RES.' + value
            elif left[-2:] == ['raw_menu','('] and re.fullmatch(r'mi[A-Za-z0-9_]+', value):
                kind, key = 'key-reference', 'Menu.' + value
            if len(left) >= 2 and left[-1] == "(" and left[-2] in PAIR_CALLS:
                if i+2 < len(ts) and ts[i+1][1] == "," and ts[i+2][2] is not None:
                    key = "NDR." + literal_id(value, ts[i+2][2])
                    kind = "bilingual-pair"
            elif kind == "literal" and any(call in left for call in UI_CALLS) and any(c.isalpha() for c in value):
                kind = "review-ui-literal"
            elif kind == "literal" and any(call in left for call in CATALOG_CALLS):
                kind = "catalog-argument"
            line = src.count("\n", 0, pos) + 1
            test_source = (path.stem.endswith("_tests") or path.stem == "tests" or prefix.startswith("tests/")
                           or any(start <= pos <= end for start,end in test_regions))
            rows.append((prefix, line, ("test-" if test_source else "") + kind, key, value))
        for match in LANG_BRANCH.finditer(src):
            # A match in comments is not a useful signal.
            if not src[src.rfind("\n", 0, match.start()) + 1:match.start()].lstrip().startswith("//"):
                rows.append((prefix, src.count("\n", 0, match.start()) + 1, "review-language-branch", "", match[0]))
    script = ROOT / "src/apply-update.ps1"
    if script.exists():
        src = script.read_text(encoding="utf-8-sig")
        count = 0
        for line, text in enumerate(src.splitlines(), 1):
            if not text.strip() or text.lstrip().startswith("#"):
                continue
            for match in re.finditer(r"""'(?:[^']|'')*'|"(?:[^"\x60]|\x60.)*" """.strip(), text):
                rows.append((script.relative_to(ROOT).as_posix(), line, "review-installer-literal", "", match[0]))
                count += 1
        coverage.append((script.relative_to(ROOT).as_posix(), len(src.splitlines()), count))
    return sorted(rows), coverage


def catalog(path, revision=None):
    section = "root"
    values = {}
    if revision:
        raw = subprocess.check_output(["git", "-C", str(ROOT), "show",
                                       f"{revision}:{path.relative_to(ROOT).as_posix()}"])
        source = raw.decode("utf-8-sig", errors="strict")
    else:
        source = path.read_text(encoding="utf-8-sig", errors="strict")
    for line in source.splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped[1:-1].casefold()
        elif "=" in line and not stripped.startswith(("#", ";")):
            key, value = line.split("=", 1)
            values[f"{section}.{key.strip().casefold()}"] = value
    return values


def look(cat, key):
    if key.startswith(("res.", "root.")):
        suffix = key.split(".", 1)[1]
        return cat.get("res."+suffix) or cat.get("root."+suffix)
    if key.startswith(("menu.", "toolbutton.")):
        return cat.get(key) or cat.get("formmain."+key)
    return cat.get(key)


def clean(value):
    return EQUAL_MNEMONIC.sub("", value).replace("&", "").strip()


def output(path, header, rows):
    with path.open("w", newline="", encoding="utf-8-sig") as stream:
        writer = csv.writer(stream, delimiter="\t")
        writer.writerow(header)
        writer.writerows(rows)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    parser.add_argument("--catalog-ref", help="Read bundled catalogs from a baseline git revision")
    args = parser.parse_args()
    rows, coverage = scan()
    args.output.mkdir(parents=True, exist_ok=True)
    cats = {p.stem: catalog(p, args.catalog_ref) for p in (ROOT/"Resources/Languages").glob("*.lang")}
    references = collections.defaultdict(list)
    for file, line, kind, key, value in rows:
        if key and not kind.startswith("test-"):
            references[key.casefold()].append(f"{file}:{line}")
    # Catalog-only keys are included too: many forms construct their key at
    # run time, so looking at only literal key references would miss them.
    keys = sorted(set(references) | set(cats["en"]) | set(cats["ko"]))
    gaps, totals, domain_totals, reference_totals = [], [], [], []
    for locale, cat in sorted(cats.items()):
        counts = collections.Counter()
        ref_counts = collections.Counter()
        domains = collections.defaultdict(collections.Counter)
        for key in keys:
            value = look(cat, key)
            base = look(cats["en"], key)
            if not value:
                state = "missing" if base or look(cats["ko"], key) else "unresolved-reference"
            elif "\ufffd" in value or "\x00" in value:
                state = "invalid-character"
            elif locale != "en" and base and clean(value) == clean(base):
                state = "same-as-English"
            else:
                state = "present"
            counts[state] += 1
            if references.get(key):
                ref_counts[state] += 1
            section, _, label = key.partition(".")
            domain = "ndr." + label.split("_", 1)[0] if section == "ndr" else section
            domains[domain][state] += 1
            if state != "present":
                gaps.append((locale, key, state, value or "", base or "", " ".join(references.get(key, ()))))
        states = ("present", "missing", "same-as-English", "invalid-character", "unresolved-reference")
        totals.append((locale, *(counts[s] for s in states)))
        reference_totals.append((locale, *(ref_counts[s] for s in states)))
        domain_totals.extend((locale, domain, *(counts[s] for s in states))
                             for domain, counts in sorted(domains.items()))
    output(args.output/"source.tsv", ("file", "line", "category", "key", "source_text"), rows)
    output(args.output/"files.tsv", ("file", "lines", "string_literals"), coverage)
    output(args.output/"catalog-gaps.tsv", ("locale", "key", "state", "selected", "english", "source"), gaps)
    output(args.output/"locale-totals.tsv", ("locale", "present", "missing", "same_as_english", "invalid_character", "unresolved_reference"), totals)
    # Explicit source references are only a scoped subset; dynamic keys and
    # hardcoded UI literals still need separate review in source.tsv.
    output(args.output/"source-reference-totals.tsv", ("locale", "present", "missing", "same_as_english", "invalid_character", "unresolved_reference"), reference_totals)
    # This separates explicit code references without calling other keys unused. ("locale", "section", "present", "missing", "same_as_english", "invalid_character", "unresolved_reference"), domain_totals)
    output(args.output/"locale-by-section.tsv", ("locale", "section", "present", "missing", "same_as_english", "invalid_character", "unresolved_reference"), domain_totals)
    print(f"{len(coverage)} source files, {sum(c[1] for c in coverage)} lines, {sum(c[2] for c in coverage)} string literals; {len(cats)} catalogs, {len(keys)} key candidates")
    print(collections.Counter(row[2] for row in rows))


if __name__ == "__main__":
    main()
