"""Validate UTF-8 catalogs and report review candidates without assuming that
identical spelling or absent legacy keys are translation failures."""
from collections import Counter, defaultdict
from pathlib import Path
import argparse, csv, json, re, unicodedata

ROOT = Path(__file__).resolve().parents[1]
VAR = re.compile(r"\{(?:[0-9]+|[A-Za-z_]\w*)\}|%[sd]|\\[rn]|</?[bc](?:[A-Fa-f0-9]{6})?>")
FILTER = re.compile(r"\*\.(?:[A-Za-z0-9*]+)")
SCRIPTS = {
    'bg': 'CYRILLIC', 'ru': 'CYRILLIC', 'uk': 'CYRILLIC',
    'el': 'GREEK', 'ko': 'HANGUL', 'ja': ('HIRAGANA','KATAKANA','CJK'),
    'zh-CN': 'CJK', 'zh-TW': 'CJK',
}
LATIN_LOCALES = set('af ca cs da de es et eu fi fil fr hr hu id is it lt lv ms nb nl pl pt-BR pt-PT ro sk sl sr-Latn sv tr vi en'.split())

def read_catalog(path, issues):
    section, data = 'root', {}
    try:
        source = path.read_bytes().decode('utf-8-sig', errors='strict')
    except UnicodeDecodeError as exc:
        issues.append((path.stem, '', 'invalid-utf8', '', str(exc)))
        return data
    for n, line in enumerate(source.splitlines(), 1):
        trimmed = line.strip()
        if not trimmed or trimmed.startswith((';','#')):
            continue
        if trimmed.startswith('[') and trimmed.endswith(']'):
            section = trimmed[1:-1].lower()
            continue
        if '=' not in line:
            issues.append((path.stem, '', 'malformed-line', '', f'{n}: {line}'))
            continue
        key, value = line.split('=', 1)
        key = section + '.' + key.strip().lower()
        if key in data:
            issues.append((path.stem,key,'duplicate-key',data[key],value))
        if not value.strip():
            issues.append((path.stem,key,'empty-value','',value))
        if any(c == '\ufffd' or unicodedata.category(c) in ('Cc','Cs') for c in value):
            issues.append((path.stem,key,'invalid-character','',value))
        # The external-program pane macros $(???) / $(???1) / $(???2)
        # are intentional. Repeated question marks elsewhere are suspicious.
        checked = re.sub(r'\$\(\?{3}[12]?\)', '', value)
        if key in ('res.msg_advselect', 'res.msg_filter'):
            # '?' is a filename wildcard in these examples, not corruption.
            checked = re.sub(r'\?+\.[A-Za-z0-9]+', '', checked)
        if re.search(r'\?{2,}', checked):
            issues.append((path.stem,key,'question-mark-review','',value))
        data[key] = value
    return data

def main():
    ap=argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--output', type=Path, required=True)
    args=ap.parse_args()
    args.output.mkdir(parents=True,exist_ok=True)
    issues=[]
    cats={p.stem:read_catalog(p,issues) for p in sorted((ROOT/'Resources/Languages').glob('*.lang'))}
    en=cats['en']
    for locale,cat in cats.items():
        for key,value in cat.items():
            base=en.get(key)
            if base is not None:
                orig, trans = VAR.findall(base), VAR.findall(value)
                # The Korean recycle-bin tooltip intentionally uses one line
                # per action, while the English original uses sentences.
                if (locale,key) == ('ko','res.recyclebin_hint'):
                    orig = [x for x in orig if x != r'\n']
                    trans = [x for x in trans if x != r'\n']
                if Counter(orig)!=Counter(trans):
                    issues.append((locale,key,'format-token-mismatch',base,value))
                elif [x for x in orig if x.startswith('%')] != [x for x in trans if x.startswith('%')]:
                    issues.append((locale,key,'printf-order-mismatch',base,value))
                if FILTER.findall(base) != FILTER.findall(value):
                    issues.append((locale,key,'filter-pattern-mismatch',base,value))
                if locale!='en' and value==base and re.search(r'[A-Za-z]{3}',base):
                    issues.append((locale,key,'identical-english-review',base,value))
            if locale in LATIN_LOCALES:
                wrong = [c for c in value if any(s in unicodedata.name(c,'') for s in ('CYRILLIC','GREEK','HANGUL','HIRAGANA','KATAKANA','CJK'))]
                if wrong:
                    issues.append((locale,key,'foreign-script-review',base or '',value))
            elif locale in SCRIPTS and key.startswith('ndr.') and base and len(base.split()) >= 4:
                expected = SCRIPTS[locale]
                if isinstance(expected,str): expected=(expected,)
                if not any(any(s in unicodedata.name(c,'') for s in expected) for c in value):
                    issues.append((locale,key,'no-native-script-review',base,value))
    # Compare explicitly authored batches to the live resource files. Entries
    # deliberately preserved by the apply helper are not automatically errors.
    ledger=ROOT/'target/ndr-manual-localization/authored.jsonl'
    authored=set()
    if ledger.is_file():
        for line in ledger.read_text(encoding='utf-8').splitlines():
            row=json.loads(line)
            authored.update((row['locale'],key) for key in row['keys'])
    for locale,key in sorted(authored):
        if not cats.get(locale,{}).get(key,'').strip():
            issues.append((locale,key,'missing-authored-value',en.get(key,''),''))
    header=('locale','key','issue','english_or_previous','selected')
    with (args.output/'quality-issues.tsv').open('w',encoding='utf-8-sig',newline='') as out:
        w=csv.writer(out,delimiter='\t'); w.writerow(header); w.writerows(issues)
    with (args.output/'catalog-counts.tsv').open('w',encoding='utf-8-sig',newline='') as out:
        w=csv.writer(out,delimiter='\t'); w.writerow(('locale','entries','english_keys_missing','native_ndr_entries'))
        for locale,cat in cats.items():
            w.writerow((locale,len(cat),len(set(en)-set(cat)),sum(k.startswith('ndr.') for k in cat)))
    summary={'catalogs':len(cats),'total_entries':sum(map(len,cats.values())),'english_entries':len(en),'authored_pairs':len(authored),'issues':dict(Counter(i[2] for i in issues))}
    (args.output/'summary.json').write_text(json.dumps(summary,indent=2,ensure_ascii=False)+'\n',encoding='utf-8')
    print(json.dumps(summary,indent=2,ensure_ascii=False))

if __name__=='__main__':
    main()
