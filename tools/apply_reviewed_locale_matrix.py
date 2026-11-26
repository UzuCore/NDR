"""Apply reviewed locale matrices, preserving native translations and validating substitutions."""
import argparse, collections, json, re
from pathlib import Path
from audit_ndr_localization import catalog, clean
from verify_ndr_translation import VAR as TOKEN, FILTER

ROOT=Path(__file__).resolve().parents[1]
LANG=ROOT/"Resources"/"Languages"
LEDGER=ROOT/"target"/"ndr-manual-localization"/"authored.jsonl"

def norm(text):
    return clean(text).strip().casefold()

def apply(matrix, extra=None):
    files={p.stem:p for p in LANG.glob("*.lang")}
    cats={l:catalog(p) for l,p in files.items()}
    assert set(matrix)==set(files), (set(matrix),set(files))
    english=cats["en"]
    allkeys=set(matrix["en"])
    assert all(set(m)<=allkeys for m in matrix.values())
    # Source languages are authoritative for existing keys.
    expected={k:english.get(k,matrix["en"][k]) for k in allkeys}
    changed=collections.Counter()
    staged=[]
    for locale,path in sorted(files.items()):
        raw=path.read_bytes()
        s=raw.decode("utf-8-sig")
        newline="\r\n" if "\r\n" in s else "\n"
        cat=cats[locale]
        lines=s.splitlines(keepends=True)
        positions={}
        section=""
        for i,line in enumerate(lines):
            bare=line.strip()
            if bare.startswith("[") and bare.endswith("]"):
                section=bare[1:-1].casefold()
            elif "=" in line and not bare.startswith((";","#")):
                k,v=line.split("=",1)
                positions[section+"."+k.strip().casefold()]=i
        updates={}
        add=collections.defaultdict(list)
        for k,new in matrix[locale].items():
            old=cat.get(k)
            base=expected[k]
            if old==new:continue
            if old and (locale in ("en","ko") or norm(old)!=norm(base)):
                continue
            assert new and new.strip() and "\n" not in new and "\r" not in new, (locale,k,new)
            assert collections.Counter(TOKEN.findall(base))==collections.Counter(TOKEN.findall(new)),(locale,k,base,new)
            assert FILTER.findall(base)==FILTER.findall(new),(locale,k,base,new)
            if k.split(".")[1].startswith(("combobox.","radiogroupitems.","listviewcols.","listbox.")):
                assert new.count(";")==base.count(";"),(locale,k,base,new)
            updates[k]=new
            if k in positions:
                i=positions[k]; name=lines[i].split("=",1)[0]
                trail=lines[i][len(lines[i].rstrip("\r\n")):]
                lines[i]=name+"="+new+trail
            else:
                section,name=k.split(".",1)
                add[section].append(name.upper() if section=="ndr" else name)
        # Insert in the matching section, leaving all existing key spellings intact.
        for i in range(len(lines)-1,-1,-1):
            m=re.fullmatch(r"\[([^]]+)\]\s*",lines[i])
            if not m:continue
            section=m[1].casefold()
            if section in add:
                names=add.pop(section)
                lines[i+1:i+1]=[name+"="+updates[section+"."+name.casefold()]+newline for name in names]
        for section,names in add.items():
            if lines and not lines[-1].endswith(("\r","\n")):lines[-1]+=newline
            lines.extend([newline,"["+section+"]"+newline,*[name+"="+updates[section+"."+name.casefold()]+newline for name in names]])
        out="".join(lines).encode("utf-8-sig" if raw.startswith(b"\xef\xbb\xbf") else "utf-8")
        staged.append((path,raw,out,updates))
    # All matrices are validated before any file is replaced.
    backup=LEDGER.parent/"batch51-original"
    backup.mkdir(parents=True,exist_ok=True)
    for path,raw,out,updates in staged:
        if not updates:continue
        b=backup/path.name
        if not b.exists():b.write_bytes(raw)
        tmp=path.with_suffix(".review.tmp")
        tmp.write_bytes(out)
        tmp.replace(path)
        end=catalog(path)
        assert all(end[k]==v for k,v in updates.items())
        changed[path.stem]+=len(updates)
        with LEDGER.open("a",encoding="utf-8",newline="\n") as f:f.write(json.dumps({"locale":path.stem,"keys":list(updates)},ensure_ascii=False)+"\n")
    print(json.dumps({"changed":sum(changed.values()),"locales":dict(changed),"matrix_keys":len(allkeys)},ensure_ascii=False))
    return changed

if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("matrix",type=Path)
    args=parser.parse_args()
    apply(json.loads(args.matrix.read_text(encoding="utf-8-sig")))
