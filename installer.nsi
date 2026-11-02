; =========================================================================
; Gerlink TelemetryAgent NSIS Installer Script
; Multi-Brand (GLESCOPE, ANTECH, EASTTECH, ZEUS)
; Output: Gerlink-TelemetryAgent-Setup-v1.2.0.exe
; =========================================================================

!include "MUI2.nsh"
!include "LogicLib.nsh"
!include "x64.nsh"

; General Definitions
!define PRODUCT_NAME "Gerlink TelemetryAgent"
!define PRODUCT_VERSION "1.2.0"
!define PRODUCT_PUBLISHER "PT Gerlink Utama Mandiri"
!define PRODUCT_WEB_SITE "https://gerlink.id"
!define PRODUCT_DIR_REGKEY "Software\Microsoft\Windows\CurrentVersion\App Paths\telemetry-agent.exe"
!define PRODUCT_UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"
!define PRODUCT_UNINST_ROOT_KEY "HKLM"
!define TASK_NAME "Gerlink TelemetryAgent Daemon"

Name "${PRODUCT_NAME} ${PRODUCT_VERSION}"
OutFile "target\installer\Gerlink-TelemetryAgent-Setup-v1.2.0.exe"
InstallDir "$PROGRAMFILES64\Gerlink\TelemetryAgent"
InstallDirRegKey HKLM "${PRODUCT_DIR_REGKEY}" ""
ShowInstDetails show
ShowUnInstDetails show
RequestExecutionLevel admin

; Interface Configuration
!define MUI_ABORTWARNING
!define MUI_ICON "${NSISDIR}\Contrib\Graphics\Icons\modern-install.ico"
!define MUI_UNICON "${NSISDIR}\Contrib\Graphics\Icons\modern-uninstall.ico"

; Pages
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\telemetry-agent.exe"
!define MUI_FINISHPAGE_RUN_PARAMETERS "status --config $\"$INSTDIR\agent_config.json$\""
!define MUI_FINISHPAGE_RUN_TEXT "Tampilkan Status Agen (Verifikasi Fingerprint & DB)"
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_UNPAGE_FINISH

!insertmacro MUI_LANGUAGE "Indonesian"
!insertmacro MUI_LANGUAGE "English"

Section "MainSection" SEC01
  SetOutPath "$INSTDIR"
  SetOverwrite try

  ; Stop previous task scheduler entry if exists
  nsExec::Exec 'schtasks /End /TN "${TASK_NAME}"'
  nsExec::Exec 'schtasks /Delete /TN "${TASK_NAME}" /F'

  ; Kill existing running telemetry-agent.exe if active
  nsExec::Exec 'taskkill /F /IM telemetry-agent.exe'

  ; Install primary binary and multi-brand config
  File "target\release\telemetry-agent.exe"
  
  ; Only overwrite config if it doesn't already exist on user PC
  IfFileExists "$INSTDIR\agent_config.json" skip_config
    File "/oname=agent_config.json" "agent_config.all_brands.json"
  skip_config:

  File "README.md"

  ; Create default directories for medical DBs if they do not exist
  CreateDirectory "D:\GLESCOPE_DB"
  CreateDirectory "D:\ANTECH_DB"
  CreateDirectory "D:\EASTTECH_DB"
  CreateDirectory "D:\ZEUS_DB"

  ; Register Windows Task Scheduler (runs silently on user logon, hidden console)
  ; Uses /SC ONLOGON with SYSTEM/HIGHEST privileges so background run has access
  nsExec::Exec 'schtasks /Create /TN "${TASK_NAME}" /TR "\"$INSTDIR\telemetry-agent.exe\" run --config \"$INSTDIR\agent_config.json\"" /SC ONLOGON /RL HIGHEST /F'

  ; Start the daemon immediately in background via Task Scheduler
  nsExec::Exec 'schtasks /Run /TN "${TASK_NAME}"'

SectionEnd

Section -Post
  WriteUninstaller "$INSTDIR\uninst.exe"
  WriteRegStr HKLM "${PRODUCT_DIR_REGKEY}" "" "$INSTDIR\telemetry-agent.exe"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "DisplayName" "$(^Name)"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "UninstallString" "$INSTDIR\uninst.exe"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "DisplayIcon" "$INSTDIR\telemetry-agent.exe"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "DisplayVersion" "${PRODUCT_VERSION}"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "URLInfoAbout" "${PRODUCT_WEB_SITE}"
  WriteRegStr ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}" "Publisher" "${PRODUCT_PUBLISHER}"
SectionEnd

Function un.onUninstSuccess
  HideWindow
  MessageBox MB_ICONINFORMATION|MB_OK "$(^Name) berhasil dihapus dari komputer."
FunctionEnd

Function un.onInit
  MessageBox MB_ICONQUESTION|MB_YESNO|MB_DEFBUTTON2 "Apakah Anda yakin ingin menghapus $(^Name) beserta seluruh konfigurasinya?" IDYES +2
  Abort
FunctionEnd

Section Uninstall
  ; Stop and remove Task Scheduler daemon
  nsExec::Exec 'schtasks /End /TN "${TASK_NAME}"'
  nsExec::Exec 'schtasks /Delete /TN "${TASK_NAME}" /F'

  ; Kill any running process
  nsExec::Exec 'taskkill /F /IM telemetry-agent.exe'

  ; Remove files
  Delete "$INSTDIR\telemetry-agent.exe"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\uninst.exe"
  
  ; Note: We intentionally do NOT delete agent_queue_*.db or agent_config.json
  ; unless user manually purges, to prevent losing unsynced medical telemetry.

  RMDir "$INSTDIR"

  DeleteRegKey ${PRODUCT_UNINST_ROOT_KEY} "${PRODUCT_UNINST_KEY}"
  DeleteRegKey HKLM "${PRODUCT_DIR_REGKEY}"
  SetAutoClose true
SectionEnd
