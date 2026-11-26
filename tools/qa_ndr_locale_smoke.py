"""Run isolated off-screen GUI smoke checks against the current NDR release.

All fixture files, profiles, reports and screenshots stay in the output directory.
This never opens the installed executable or touches its settings.
"""
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path
import argparse, hashlib, json, os, subprocess, time
from PIL import Image, ImageDraw

ROOT=Path(__file__).resolve().parents[1]
RESOURCES=ROOT/'Resources'
EXE=ROOT/'target/release/ndr.exe'
LOCALES=sorted(p.stem for p in (RESOURCES/'Languages').glob('*.lang'))
SAMPLES=['ko','en','ja','zh-CN','zh-TW','de','ru','el','vi','fi','fil','pt-PT']

def run_case(case,out,size=None):
    locale,kind=case
    work=out/f'{locale}-{kind}'
    work.mkdir(parents=True,exist_ok=True)
    left,right,profile=(work/x for x in ('left','right','profile'))
    for directory in (left,right,profile): directory.mkdir(exist_ok=True)
    (left/'document.txt').write_text('test document\n',encoding='utf-8')
    (left/'sample-folder').mkdir(exist_ok=True)
    (right/'different.txt').write_text('test\n',encoding='utf-8')
    report,shot=work/'report.json',work/'screen.png'
    for old in (report,shot):
        if old.exists(): old.unlink()
    command=[str(EXE),'--profile',str(profile),'--left',str(left),'--right',str(right),'--language',locale,'--smoke-seconds','6','--report',str(report),'--shot',kind,str(shot)]
    if size:
        command.extend(['--smoke-size',str(size[0]),str(size[1])])
    env={**os.environ,'NDR_RESOURCE_DIR':str(RESOURCES)}
    start=time.monotonic()
    result={'locale':locale,'kind':kind,'report':str(report),'screenshot':str(shot)}
    try:
        p=subprocess.run(command,cwd=str(ROOT),env=env,capture_output=True,timeout=35,creationflags=getattr(subprocess,'CREATE_NO_WINDOW',0))
        result['returncode']=p.returncode
        (work/'stderr.log').write_bytes(p.stderr)
        if not report.is_file() or not shot.is_file():
            raise RuntimeError('No report or screenshot')
        r=json.loads(report.read_text(encoding='utf-8-sig'))
        with Image.open(shot) as im:
            im.verify()
        with Image.open(shot) as im:
            result['size']=list(im.size)
            result['image_nonuniform']=len(im.convert('RGB').getcolors(1000000) or [])>20
        result['runtime']={k:r.get(k) for k in ['language','frames','leftCount','rightCount','leftError','rightError','leftLoading','rightLoading','foreground','renderer','viewportPoints','nativePixelsPerPoint','pixelsPerPoint']}
        result['ok']=(p.returncode==0 and r.get('language')==locale and r.get('frames',0)>=20 and r.get('foreground') is False
            and not r.get('leftError') and not r.get('rightError') and not r.get('leftLoading') and not r.get('rightLoading')
            and result['image_nonuniform'])
        if size:
            result['ok']=result['ok'] and bool(r.get('nativePixelsPerPoint')) and len(r.get('viewportPoints',[]))==2 and all(abs(a-b)<=2 for a,b in zip(r['viewportPoints'],size))
    except Exception as e:
        result['ok']=False; result['error']=str(e)
    result['seconds']=round(time.monotonic()-start,2)
    return result

def contact_sheets(results,out):
    # Crop top-left menus; full image is preserved alongside every report.
    for kind in sorted({r['kind'] for r in results}):
        group=[r for r in results if r['kind']==kind and r['ok']]
        for start in range(0,len(group),8):
            part=group[start:start+8]
            w,h=530,380
            sheet=Image.new('RGB',(w*2,h*((len(part)+1)//2)),(238,238,238))
            draw=ImageDraw.Draw(sheet)
            for i,r in enumerate(part):
                x,y=(i%2)*w,(i//2)*h
                with Image.open(r['screenshot']) as im:
                    # Main-menu crops preserve the text near its original size.
                    if kind=='mainmenu': im=im.crop((0,0,min(650,im.width),min(460,im.height)))
                    im.thumbnail((w-8,h-24))
                    sheet.paste(im,(x+4,y+22))
                draw.text((x+4,y+3),f"{r['locale']} / {kind}",fill='black')
            sheet.save(out/f'contact-{kind}-{start//8+1:02}.jpg',quality=90)

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--output',required=True,type=Path)
    p.add_argument('--workers',type=int,default=4)
    p.add_argument('--size',nargs=2,type=int,metavar=('WIDTH','HEIGHT'),help='isolated client size in logical points')
    p.add_argument('--pilot',action='store_true')
    p.add_argument('--case', action='append', metavar='LOCALE:KIND', help='repeat to render specific regression cases')
    a=p.parse_args(); out=a.output.resolve(); out.mkdir(parents=True,exist_ok=True)
    cases=[('ko','mainmenu'),('de','compareopts'),('ja','remote')] if a.pilot else [(l,'mainmenu') for l in LOCALES]+[(l,k) for l in SAMPLES for k in ('compareopts','remote','delconfirm')]
    if a.case:
        cases=[tuple(c.split(':', 1)) for c in a.case]
        assert all(len(c)==2 and c[0] in LOCALES and (c[1] in ('mainmenu','compareopts','compareresult','remote','delwarning','delconfirm','vaultcreate','vaultunlock','search','foldertree','foldertree-search','foldertree-search-many','foldertree-favorites','syncpreview','archive','split','join','shortcuts','editshortcut','dropmenu','progress-copy','progress-move','progress-delete','progress-recycle','progress-active','progress-archive','update-download','update-verify','update-prepare','update-install','update-failed','error-transfer','error-archive','error-update','error-retry','error-continue','error-long','ftpquick','ftpattr','sites','names','auto','narrowdetails','advrename-delete','advrename-replace','advrename-case','advrename-number','advrename-extension','advrename-rules','advrename-preview','credentials','funckey','attributes','conflict','conflict-resume','conflict-folder','advrename','prompt','rename','destination','savelist') or (c[1].startswith('settings') and c[1][8:].isdigit())) for c in cases), cases
    results=[]
    with ThreadPoolExecutor(max_workers=a.workers) as pool:
        futures={pool.submit(run_case,c,out,a.size):c for c in cases}
        for f in as_completed(futures):
            r=f.result(); results.append(r); print(f"{len(results)}/{len(cases)} {r['locale']} {r['kind']} {'PASS' if r['ok'] else 'FAIL'}",flush=True)
            (out/'progress.json').write_text(json.dumps(results,indent=2,ensure_ascii=False),encoding='utf-8')
    # Retry only failures, sequentially, to exclude concurrent-GPU startup delay.
    for i,r in enumerate(results):
        if not r['ok']:
            print(f"Retry {r['locale']} {r['kind']}",flush=True)
            retry=run_case((r['locale'],r['kind']),out,a.size)
            retry['first_attempt']=r; results[i]=retry
    results.sort(key=lambda r:(r['kind'],r['locale']))
    summary={'exe_sha256':hashlib.sha256(EXE.read_bytes()).hexdigest(),'total':len(results),'passed':sum(r['ok'] for r in results),'failed':sum(not r['ok'] for r in results),'cases':results}
    (out/'manifest.json').write_text(json.dumps(summary,indent=2,ensure_ascii=False),encoding='utf-8')
    contact_sheets(results,out)
    print(json.dumps({k:v for k,v in summary.items() if k!='cases'}),flush=True)
    raise SystemExit(1 if summary['failed'] else 0)

if __name__=='__main__': main()
