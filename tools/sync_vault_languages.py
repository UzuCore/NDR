"""Add the NDRVault strings to every shipped language resource.

English is the safe fallback for messages without a reviewed translation.
No original NexusFile translation keys are changed.
"""
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1] / "Resources" / "Languages"

EN = {
    "CreateTitle": "New encrypted folder",
    "UnlockTitle": "Unlock encrypted folder",
    "CreateHint": "Enter the name of the new encrypted folder.",
    "FolderName": "Folder name",
    "Password": "Password",
    "ConfirmPassword": "Confirm password",
    "CreateNote": "Names and files are encrypted. Password: at least 6 characters.",
    "NoRecovery": "A forgotten password cannot be recovered.",
    "PasswordNotStored": "The password is not saved.",
    "OK": "OK",
    "Cancel": "Cancel",
    "SameContainer": "Open two different encrypted containers for cross-container transfer.",
    "TransferTitle": "Encrypted container transfer",
    "MoveConfirm": "For each selected item, copy it to the other encrypted container, then remove the original?",
    "CopyConfirm": "Copy all selected items to the other encrypted container?",
    "TransferDone": "Encrypted items transferred: {count}",
    "SourceRetained": "Destination committed, but source deletion failed. Both copies are retained",
    "TransferFailed": "Encrypted transfer failed; destination rolled back",
    "ExportInvalidTarget": "Export destination must be a regular directory.",
    "ExportTitle": "Export encrypted items",
    "ExportMoveConfirm": "Move {count} items to unencrypted folder {destination}? Originals are removed only after each successful export. Existing targets require duplicate-file confirmation.",
    "ExportCopyConfirm": "Export {count} items as plaintext to {destination}? Existing targets require duplicate-file confirmation.",
    "ExportDone": "Exported {count} items as unencrypted files.",
    "ExportFailed": "Export/move failed",
    "MoveLocalTitle": "Move into encrypted folder",
    "MoveLocalConfirm": "Encrypt each selected item first, then remove its local original. Continue?",
    "MoveLocalFailed": "Encrypted move failed; local sources preserved",
    "MoveLocalDone": "Moved {count} encrypted items.",
    "MoveLocalPartial": "Encrypted copy completed; some local originals could not be deleted",
    "ImportDone": "Imported {count} encrypted items.",
    "ImportFailed": "Encrypted import failed; changes rolled back",
    "InsideCopyTitle": "Copy within encrypted container",
    "InsideMoveTitle": "Move within encrypted container",
    "InsidePrompt": "Destination path relative to container root (e.g. documents/archive or /)",
    "InsideDone": "Updated {count} items within the encrypted container.",
    "InsideFailed": "Encrypted folder operation failed; changes rolled back",
    "StatusLocked": "Encrypted folder locked. Reopen to authenticate.",
    "TextEditorTitle": "Encrypted folder",
    "SaveEncrypted": "Save encrypted",
    "NewFolderTitle": "New folder",
    "NewFileTitle": "New file",
    "NewFolderPrompt": "Enter a folder name inside the encrypted container.",
    "NewFilePrompt": "Enter a file name inside the encrypted container.",
    "RenameTitle": "Rename",
    "RenamePrompt": "Enter a new name.",
    "DeleteTitle": "Encrypted folder",
    "DeleteConfirm": "Permanently delete {count} encrypted items?",
    "NoPaste": "No files or folders to paste.",
    "ClipboardCut": "Cut/paste into encrypted folders is not supported. Use F4 move instead.",
    "SettingsSection": "Encrypted Folder (.ndr)",
    "SessionPolicy": "Session retention",
    "UntilExit": "Keep session until NDR exits",
    "Timed": "Keep session for a specified duration",
    "SessionMinutes": "Session duration (minutes, 1-1440)",
    "LockOnMinimize": "Lock when minimized",
    "SessionNote": "Reopening during the session needs no password. When it expires, the password is required again.",
    "AutoFolderSizeLeft": "Left: show folder sizes in the list",
    "AutoFolderSizeRight": "Right: show folder sizes in the list",
    "BinaryUnsupported": "Binary files cannot be opened in the encrypted text editor.",
    "TextSize": "Only text files up to 1 MiB can be edited.",
    "InvalidFolderName": "Enter a valid encrypted folder name.",
    "ReservedFolderName": "This folder name is reserved by Windows.",
    "PasswordMismatch": "Passwords do not match.",
    "WrongPassword": "Wrong password, damaged container, or unsupported format.",
    "WrongFormat": "Unsupported encrypted container format.",
    "WrongVersion": "Unsupported encrypted container version.",
    "MissingContainer": "Encrypted container file does not exist.",
    "ShortPassword": "Password must contain at least 6 characters.",
    "MenuCreate": "New encrypted folder...",
    "MenuOpen": "Open selected encrypted folder",
    "JobBusy": "Finish the current transfer first.",
    "ProgressCopy": "Copying encrypted items...",
    "ProgressMove": "Moving encrypted items...",
    "ProgressCancel": "Cancel",
    "ProgressCount": "{done} / {total}",
    "ProgressSummary": "Completed: {done}; skipped: {skipped}; total: {total}",
    "JobFailed": "Encrypted transfer interrupted",
    "JobCancelled": "Encrypted transfer cancelled; completed items retained",
    "JobDone": "Encrypted transfer completed",
    "InvalidImportPath": "Cannot transfer a path without a valid Unicode filename.",
}

KO = {
    "CreateTitle": "새 암호화 폴더", "UnlockTitle": "암호화 폴더 잠금 해제",
    "CreateHint": "암호화할 새 폴더의 이름을 입력하세요.", "FolderName": "폴더 이름",
    "Password": "비밀번호", "ConfirmPassword": "비밀번호 확인",
    "CreateNote": "폴더 안의 파일과 이름이 암호화됩니다. 비밀번호는 6자 이상입니다.",
    "NoRecovery": "비밀번호를 잊으면 복구할 수 없습니다.",
    "PasswordNotStored": "비밀번호는 저장되지 않습니다.", "OK": "확인", "Cancel": "취소",
    "SameContainer": "서로 다른 암호화 컨테이너를 양쪽 패널에서 여세요.",
    "TransferTitle": "암호화 컨테이너 간 전송",
    "MoveConfirm": "선택한 항목을 하나씩 다른 암호화 컨테이너에 복사한 후 해당 원본을 삭제할까요?",
    "CopyConfirm": "선택한 항목을 다른 암호화 컨테이너에 복사할까요?",
    "TransferDone": "암호화 항목 {count}개를 전송했습니다.",
    "SourceRetained": "대상 저장은 완료됐지만 원본 삭제에 실패했습니다. 양쪽 복사본을 보존했습니다",
    "TransferFailed": "암호화 컨테이너 전송 실패: 대상 변경을 취소했습니다",
    "ExportInvalidTarget": "내보내기 대상은 일반 디렉터리여야 합니다.",
    "ExportTitle": "암호화 항목 내보내기",
    "ExportMoveConfirm": "{count}개 항목을 암호화되지 않은 {destination} 폴더로 이동할까요? 각 항목의 내보내기가 성공한 후 원본을 삭제합니다. 중복 파일은 처리 방법을 확인합니다.",
    "ExportCopyConfirm": "{count}개 항목을 암호화되지 않은 {destination} 폴더로 내보낼까요? 중복 파일은 처리 방법을 확인합니다.",
    "ExportDone": "{count}개 항목을 암호화되지 않은 파일로 내보냈습니다.",
    "ExportFailed": "내보내기/이동 실패",
    "MoveLocalTitle": "암호화 폴더로 이동",
    "MoveLocalConfirm": "선택한 항목을 하나씩 암호화한 뒤 해당 일반 폴더 원본을 삭제합니다. 계속할까요?",
    "MoveLocalFailed": "암호화 이동 실패: 일반 폴더 원본을 보존했습니다",
    "MoveLocalDone": "암호화 항목 {count}개를 이동했습니다.",
    "MoveLocalPartial": "암호화 복사 완료. 일부 일반 폴더 원본을 삭제하지 못했습니다",
    "ImportDone": "암호화 항목 {count}개를 가져왔습니다.",
    "ImportFailed": "암호화 가져오기 실패: 변경 사항을 취소했습니다",
    "InsideCopyTitle": "암호화 폴더 내부 복사", "InsideMoveTitle": "암호화 폴더 내부 이동",
    "InsidePrompt": "컨테이너 루트 기준 대상 폴더 경로 (예: 문서/보관 또는 /)",
    "InsideDone": "암호화 폴더 내부의 {count}개 항목을 처리했습니다.",
    "InsideFailed": "암호화 폴더 내부 작업 실패: 변경 사항을 취소했습니다",
    "StatusLocked": "암호화 폴더가 잠겼습니다. 다시 열어 인증하세요.",
    "TextEditorTitle": "암호화 폴더", "SaveEncrypted": "암호화하여 저장",
    "NewFolderTitle": "새 폴더", "NewFileTitle": "새 파일",
    "NewFolderPrompt": "암호화 폴더에 만들 폴더 이름을 입력하세요.",
    "NewFilePrompt": "암호화 폴더에 만들 파일 이름을 입력하세요.",
    "RenameTitle": "이름 바꾸기", "RenamePrompt": "새 이름을 입력하세요.",
    "DeleteTitle": "암호화 폴더", "DeleteConfirm": "암호화 폴더에서 {count}개 항목을 영구 삭제할까요?",
    "NoPaste": "붙여넣을 파일 또는 폴더가 없습니다.",
    "ClipboardCut": "암호화 폴더로 잘라내기/붙여넣기는 지원하지 않습니다. F4 이동을 사용하세요.",
    "SettingsSection": "암호화 폴더 (.ndr)", "SessionPolicy": "세션 유지 방식",
    "UntilExit": "NDR 종료 시까지 세션 유지", "Timed": "지정한 시간 동안 세션 유지",
    "SessionMinutes": "세션 유지 시간 (분, 1~1440)", "LockOnMinimize": "최소화 시 잠금",
    "SessionNote": "세션이 유지되는 동안 다시 열면 비밀번호가 필요하지 않습니다. 만료 후에는 다시 입력해야 합니다.",
    "AutoFolderSizeLeft": "왼쪽 폴더 크기를 목록에 자동 표시",
    "AutoFolderSizeRight": "오른쪽 폴더 크기를 목록에 자동 표시",
    "BinaryUnsupported": "바이너리 파일은 암호화 텍스트 편집기로 열 수 없습니다.",
    "TextSize": "텍스트 편집은 1 MiB 이하 파일에만 지원됩니다.",
    "InvalidFolderName": "올바른 암호화 폴더 이름을 입력해 주세요.",
    "ReservedFolderName": "Windows에서 사용할 수 없는 폴더 이름입니다.",
    "PasswordMismatch": "비밀번호가 일치하지 않습니다.",
    "WrongPassword": "비밀번호가 틀렸거나 컨테이너가 손상되었거나 지원하지 않는 형식입니다.",
    "WrongFormat": "지원하지 않는 암호화 컨테이너 형식입니다.",
    "WrongVersion": "지원하지 않는 암호화 컨테이너 버전입니다.",
    "MissingContainer": "암호화 컨테이너 파일을 찾을 수 없습니다.",
    "ShortPassword": "비밀번호는 최소 6자 이상이어야 합니다.",
    "MenuCreate": "새 암호화 폴더...",
    "MenuOpen": "선택한 암호화 폴더 열기",
    "JobBusy": "진행 중인 전송을 먼저 완료하세요.",
    "ProgressCopy": "암호화 항목 복사 중...",
    "ProgressMove": "암호화 항목 이동 중...",
    "ProgressCancel": "취소",
    "ProgressCount": "{done} / {total}",
    "ProgressSummary": "완료: {done}개 / 건너뜀: {skipped}개 / 전체: {total}개",
    "JobFailed": "암호화 전송 중 오류가 발생했습니다",
    "JobCancelled": "암호화 전송을 취소했습니다. 완료된 항목은 유지됩니다",
    "JobDone": "암호화 전송을 완료했습니다",
    "InvalidImportPath": "올바른 유니코드 파일 이름이 없는 경로는 전송할 수 없습니다.",
}

# Widely used UI terms have manually reviewed short translations. Longer
# security-sensitive messages use English where the translation isn't reviewed.
COMMON = {
    "ja": ("パスワード", "パスワードの確認", "確認", "キャンセル", "フォルダー名", "暗号化フォルダーを開く", "新しい暗号化フォルダー"),
    "zh-CN": ("密码", "确认密码", "确定", "取消", "文件夹名称", "解锁加密文件夹", "新建加密文件夹"),
    "zh-TW": ("密碼", "確認密碼", "確定", "取消", "資料夾名稱", "解鎖加密資料夾", "新增加密資料夾"),
    "de": ("Passwort", "Passwort bestätigen", "OK", "Abbrechen", "Ordnername", "Verschlüsselten Ordner entsperren", "Neuer verschlüsselter Ordner"),
    "fr": ("Mot de passe", "Confirmer le mot de passe", "OK", "Annuler", "Nom du dossier", "Déverrouiller le dossier chiffré", "Nouveau dossier chiffré"),
    "es": ("Contraseña", "Confirmar contraseña", "Aceptar", "Cancelar", "Nombre de carpeta", "Desbloquear carpeta cifrada", "Nueva carpeta cifrada"),
    "it": ("Password", "Conferma password", "OK", "Annulla", "Nome cartella", "Sblocca cartella crittografata", "Nuova cartella crittografata"),
    "pt-BR": ("Senha", "Confirmar senha", "OK", "Cancelar", "Nome da pasta", "Desbloquear pasta criptografada", "Nova pasta criptografada"),
    "ru": ("Пароль", "Подтвердите пароль", "ОК", "Отмена", "Имя папки", "Разблокировать зашифрованную папку", "Новая зашифрованная папка"),
    "nl": ("Wachtwoord", "Wachtwoord bevestigen", "OK", "Annuleren", "Mapnaam", "Versleutelde map ontgrendelen", "Nieuwe versleutelde map"),
    "pl": ("Hasło", "Potwierdź hasło", "OK", "Anuluj", "Nazwa folderu", "Odblokuj zaszyfrowany folder", "Nowy zaszyfrowany folder"),
    "tr": ("Parola", "Parolayı onayla", "Tamam", "İptal", "Klasör adı", "Şifreli klasörün kilidini aç", "Yeni şifreli klasör"),
    "uk": ("Пароль", "Підтвердити пароль", "OK", "Скасувати", "Назва папки", "Розблокувати зашифровану папку", "Нова зашифрована папка"),
    "sv": ("Lösenord", "Bekräfta lösenord", "OK", "Avbryt", "Mappnamn", "Lås upp krypterad mapp", "Ny krypterad mapp"),
    "fi": ("Salasana", "Vahvista salasana", "OK", "Peruuta", "Kansion nimi", "Avaa salatun kansion lukitus", "Uusi salattu kansio"),
    "cs": ("Heslo", "Potvrdit heslo", "OK", "Zrušit", "Název složky", "Odemknout šifrovanou složku", "Nová šifrovaná složka"),
    "sk": ("Heslo", "Potvrdiť heslo", "OK", "Zrušiť", "Názov priečinka", "Odomknúť šifrovaný priečinok", "Nový šifrovaný priečinok"),
    "hu": ("Jelszó", "Jelszó megerősítése", "OK", "Mégse", "Mappanév", "Titkosított mappa feloldása", "Új titkosított mappa"),
    "ca": ("Contrasenya", "Confirma la contrasenya", "D'acord", "Cancel·la", "Nom de carpeta", "Desbloqueja la carpeta xifrada", "Carpeta xifrada nova"),
}


def main():
    for path in sorted(ROOT.glob("*.lang")):
        code = path.stem
        values = dict(EN)
        if code == "ko":
            values.update(KO)
        elif code in COMMON:
            words = COMMON[code]
            values.update(zip(("Password", "ConfirmPassword", "OK", "Cancel", "FolderName", "UnlockTitle", "CreateTitle"), words))
        original = path.read_bytes()
        decoded = original.decode("utf-8-sig")
        if "[NDRVault]" in decoded:
            decoded = decoded.split("[NDRVault]")[0].rstrip("\r\n")
            original = decoded.encode("utf-8")
        sep = "\r\n" if b"\r\n" in original else "\n"
        block = "[NDRVault]" + sep + sep.join(f"{key}={value}" for key, value in values.items())
        path.write_bytes(original.rstrip(b"\r\n") + (sep * 2 + block + sep).encode("utf-8"))
        print(code, len(values))


if __name__ == "__main__":
    main()
