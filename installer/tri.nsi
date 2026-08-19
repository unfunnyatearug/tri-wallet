; Installer for tri. Built with:  makensis installer\tri.nsi
; Everything here is command line driven. There is no GUI step in the build.

!include "MUI2.nsh"

!define APP_NAME    "tri wallet"
!define APP_ID      "tri"
!define APP_VERSION "0.1.1"
!define PUBLISHER   "unfunnyatearug"
!define REG_UNINST  "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APP_ID}"

Name "${APP_NAME}"
OutFile "..\dist\tri-setup.exe"
Unicode true

; Per user install. No administrator rights are needed and nothing is written
; outside the user profile.
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\tri"
InstallDirRegKey HKCU "Software\${APP_ID}" "InstallDir"

VIProductVersion "0.1.1.0"
VIAddVersionKey "ProductName"     "${APP_NAME}"
VIAddVersionKey "FileDescription" "Wallet for Bitcoin, Solana and USDC"
VIAddVersionKey "FileVersion"     "${APP_VERSION}"
VIAddVersionKey "ProductVersion"  "${APP_VERSION}"
VIAddVersionKey "CompanyName"     "${PUBLISHER}"
VIAddVersionKey "LegalCopyright"  "MIT License"

!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES

!define MUI_FINISHPAGE_RUN "$INSTDIR\tri-gui.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Open tri wallet"
!define MUI_FINISHPAGE_TEXT "tri has been installed.$\r$\n$\r$\nThe command line program is available as 'tri' in a new terminal window. Existing terminals will not see it until they are restarted."
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

Section "Install"
  SetOutPath "$INSTDIR"
  File "..\target\release\tri.exe"
  File "..\target\release\tri-gui.exe"
  File "..\README.md"
  File "..\LICENSE"

  WriteRegStr HKCU "Software\${APP_ID}" "InstallDir" "$INSTDIR"

  CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\tri-gui.exe"

  ; The user PATH is edited through PowerShell rather than the registry
  ; directly, because NSIS strings are capped and a long PATH would be
  ; silently truncated on write.
  DetailPrint "Adding $INSTDIR to the user PATH"
  nsExec::ExecToLog `powershell -NoProfile -NonInteractive -Command "$$p = [Environment]::GetEnvironmentVariable('Path','User'); if ($$null -eq $$p) { $$p = '' }; if (-not ($$p -split ';' -contains '$INSTDIR')) { [Environment]::SetEnvironmentVariable('Path', ($$p.TrimEnd(';') + ';$INSTDIR').TrimStart(';'), 'User') }`

  WriteUninstaller "$INSTDIR\uninstall.exe"

  WriteRegStr   HKCU "${REG_UNINST}" "DisplayName"     "${APP_NAME}"
  WriteRegStr   HKCU "${REG_UNINST}" "DisplayVersion"  "${APP_VERSION}"
  WriteRegStr   HKCU "${REG_UNINST}" "Publisher"       "${PUBLISHER}"
  WriteRegStr   HKCU "${REG_UNINST}" "DisplayIcon"     "$INSTDIR\tri-gui.exe"
  WriteRegStr   HKCU "${REG_UNINST}" "InstallLocation" "$INSTDIR"
  WriteRegStr   HKCU "${REG_UNINST}" "UninstallString" "$\"$INSTDIR\uninstall.exe$\""
  WriteRegDWORD HKCU "${REG_UNINST}" "NoModify" 1
  WriteRegDWORD HKCU "${REG_UNINST}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  DetailPrint "Removing $INSTDIR from the user PATH"
  nsExec::ExecToLog `powershell -NoProfile -NonInteractive -Command "$$p = [Environment]::GetEnvironmentVariable('Path','User'); if ($$null -ne $$p) { [Environment]::SetEnvironmentVariable('Path', (($$p -split ';' | Where-Object { $$_ -ne '$INSTDIR' -and $$_ -ne '' }) -join ';'), 'User') }`

  Delete "$INSTDIR\tri.exe"
  Delete "$INSTDIR\tri-gui.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\LICENSE"
  Delete "$INSTDIR\uninstall.exe"
  RMDir  "$INSTDIR"

  Delete "$SMPROGRAMS\${APP_NAME}.lnk"

  DeleteRegKey HKCU "${REG_UNINST}"
  DeleteRegKey HKCU "Software\${APP_ID}"

  ; The wallet file in %USERPROFILE%\.tri is deliberately left in place.
  ; Removing it would destroy funds for anyone who uninstalls without having
  ; written down their recovery phrase.
SectionEnd
