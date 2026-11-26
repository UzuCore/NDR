"""Reviewed semantic reuse only: each destination/source pair is explicit.
No fuzzy matching and no English fallback materialized as a translation."""
from pathlib import Path
import json,re
from audit_ndr_localization import catalog,clean
from apply_reviewed_locale_matrix import apply,LANG,LEDGER

C={p.stem:catalog(p) for p in LANG.glob("*.lang")}
E=C["en"]
P="formpreferences."
M="formmain.menu."
S="ndr.ui_settings_"
# A translated command/menu caption and its same-meaning function caption.
FUNCTIONS={
 "openwithparam":"miparameteropen","gotomusic":"mitomusic",
 "gotopictures":"mitopictures","gotovideos":"mitovideos",
 "view":"mitextviewer","edit":"mitexteditor","autoextract":"miarcextractauto",
 "findnext":"mifindnext","networkdialog":"minetwork","selectsamename":"miselectsamename",
 "selectsameext":"miselectsameext","ftpsitemanager":"miftpmanagesites",
 "gotolastdest":"mitolastdest","invertselection":"miinvertselection",
 "commandline":"micommandline","splitfile":"misplitfile","joinfiles":"mijoinfile",
}
# These labels denote the same existing setting, despite stylistic differences.
SETTINGS={
 "caption.chkusedateage":S+"show_recent_dates_as_b31039",
 "caption.lsizeformat":S+"size_unit_048cb0",
 "caption.chkthousandsseparator":S+"show_thousands_separators_in_41af28",
 "caption.lthumbsize":S+"thumbnail_size_px_670baf",
 "caption.lsort1":M+"misortby","caption.lsort2":M+"misortby",
 "caption.chkremovereadonly":S+"remove_the_read_only_f6122f",
 "caption.chkcompletesound":S+"play_a_sound_when_ffcbec",
 "caption.chkconfirmdrop":S+"confirm_drag_operations_758376",
 "caption.chkexplorermenu":S+"right_click_use_the_7bfb18",
 "caption.chkstartpath1":S+"use_a_fixed_left_99ccc9",
 "caption.chkstartpath2":S+"use_a_fixed_right_d7dec1",
 "caption.lview":S+"text_viewer_program_bf3c33",
 "caption.ledit":S+"text_editor_program_0ef879",
 "caption.lmymusic":M+"mitomusic",
 "caption.lmypictures":M+"mitopictures",
 "caption.lmyvideos":M+"mitovideos",
 "caption.lmydocuments":M+"mitodocuments",
 "caption.lpathdownloads":M+"mitodownloads",
 "caption.litemwidth":S+"folder_tree_name_width_b9ab39",
 "caption.chktreemainwndsize":S+"open_the_separate_folder_9bfa18",
 "caption.chktreeshowhidden":S+"show_hidden_folders_in_7a053e",
 "caption.chktreeshowicon":S+"show_folder_tree_icons_e752d8",
 "groupbox.gbtreefont":S+"folder_tree_font_9996b6",
 "caption.ltreefont":S+"folder_tree_font_9996b6",
 "caption.chkcheckupdate":S+"check_the_configured_feed_8a0d4c",
 "caption.chkrunonce":S+"run_only_one_ndr_4792bb",
 "caption.chkuselogicalsort":S+"natural_sort_that_considers_6aa921",
 "caption.chkusetab":S+"use_tabs_0b7ee8",
 "caption.chkhidewhenminimized":S+"hide_in_the_notification_be1f1b",
 "caption.btnok":"res.ok",
 "caption.lextcolor":"res.pref_setcolor",
}
def plain(v, key):
    v=re.sub(r"\s*\(&[A-Za-z0-9]\)","",v)
    v=clean(v)
    if key.startswith(M):v=re.sub(r"^[A-Za-z0-9]\.\s+","",v)
    return v.strip()
def native(locale,key):
    v=C[locale].get(key)
    return plain(v,key) if v else None
def reuse(locale, dest, sources):
    values=[native(locale,k) for k in sources]
    if not all(values): return None
    return ";".join(values)
pairs={**{"res.func_"+d:[M+s] for d,s in FUNCTIONS.items()},
       **{P+d:[s] for d,s in SETTINGS.items()}}
pairs[P+"combobox.cbdefdelbtn"]=["res.func_delete","res.yes","res.no"]
pairs[P+"combobox.cbsizeformat"]=[S+"auto_44b5d1","ndr.ui_size_bytes"]
pairs[P+"combobox.cbsort1"]=["ndr.settings_choice_none","res.field_name","res.field_ext","res.field_size","res.field_datemodified","res.field_attr","ndr.settings_choice_color"]
pairs[P+"combobox.cbsort2"]=pairs[P+"combobox.cbsort1"]
# In these original lists, the values remain fixed and only captions change.
data={l:{} for l in C}
for d,sources in pairs.items():
    assert d in E,d
    for source in sources:assert source in E,source
    for l in C:
        if l=="en":data[l][d]=E[d];continue
        value=reuse(l,d,sources)
        if d.endswith("cbsizeformat") and value:value+=";KB;MB;GB"
        if value and value.count(";")==E[d].count(";"):data[l][d]=value
path=LEDGER.parent/"batch51c-reviewed-reuse.json"
path.write_text(json.dumps(data,ensure_ascii=False,indent=2),encoding="utf-8")
apply(data)
