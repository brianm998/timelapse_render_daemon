; SPDX-License-Identifier: GPL-3.0-or-later
; Copyright (C) 2026 Timelapse Render Daemon contributors
;
; The per-user Windows installer of trd (spec 1 §10.3). build-installer.sh runs makensis with:
;   /DVERSION=<x.y.z[-pre]>      the version shown in the installer and the uninstall entry
;   /DNUMERIC_VERSION=<x.y.z.0>  the file version (the pre-release suffix stripped)
;   /DSTAGE_DIR=<dir>            the staged tree: trd.exe, trdw.exe, ffmpeg.exe, ffprobe.exe,
;                                exiftool\ and licenses\
;   /DOUT_FILE=<setup.exe>       where the installer is written
;
; No administrator rights (RequestExecutionLevel user). The install directory is
; %LOCALAPPDATA%\Programs\TimelapseRenderDaemon. LOCALAPPDATA is read from the environment, so
; test-installer.sh can redirect a silent install into a temporary directory; the NSIS constant
; $LOCALAPPDATA is only the fallback.
;
; Nothing on the silent (/S) path shows a message box, so /S never blocks.

Unicode true
RequestExecutionLevel user
SetCompressor lzma
ShowInstDetails show

!include "LogicLib.nsh"

!ifndef VERSION
  !error "VERSION is not defined (pass /DVERSION=x.y.z)"
!endif
!ifndef NUMERIC_VERSION
  !error "NUMERIC_VERSION is not defined (pass /DNUMERIC_VERSION=x.y.z.0)"
!endif
!ifndef STAGE_DIR
  !error "STAGE_DIR is not defined (pass /DSTAGE_DIR=<staged tree>)"
!endif
!ifndef OUT_FILE
  !error "OUT_FILE is not defined (pass /DOUT_FILE=<setup.exe>)"
!endif

!define UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\TimelapseRenderDaemon"
!define TASK_NAME "TimelapseRenderDaemon"

Name "Timelapse Render Daemon ${VERSION}"
OutFile "${OUT_FILE}"
VIProductVersion "${NUMERIC_VERSION}"
VIAddVersionKey "ProductName" "Timelapse Render Daemon"
VIAddVersionKey "FileDescription" "Timelapse Render Daemon installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "CompanyName" "Timelapse Render Daemon contributors"
VIAddVersionKey "LegalCopyright" "Copyright (C) 2026 Timelapse Render Daemon contributors"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Function .onInit
  SetShellVarContext current
  ReadEnvStr $0 "LOCALAPPDATA"
  ${If} $0 == ""
    StrCpy $0 "$LOCALAPPDATA"
  ${EndIf}
  StrCpy $INSTDIR "$0\Programs\TimelapseRenderDaemon"
FunctionEnd

Function un.onInit
  SetShellVarContext current
  ; The install directory is the one the installer recorded; an empty value makes the
  ; uninstall section abort before it removes anything.
  ReadRegStr $INSTDIR HKCU "${UNINST_KEY}" "InstallLocation"
FunctionEnd

Section "Timelapse Render Daemon" SecMain
  ; An earlier install's trdw.exe locks its files, so the old task is stopped and removed before
  ; anything is copied over it. The result is ignored: a first install has no task to remove.
  IfFileExists "$INSTDIR\trd.exe" 0 +2
    ExecWait '"$INSTDIR\trd.exe" service uninstall'

  SetOutPath "$INSTDIR"
  File "${STAGE_DIR}\trd.exe"
  File "${STAGE_DIR}\trdw.exe"
  File "${STAGE_DIR}\ffmpeg.exe"
  File "${STAGE_DIR}\ffprobe.exe"
  File /r "${STAGE_DIR}\exiftool"
  File /r "${STAGE_DIR}\licenses"

  ; The uninstaller and its registry entry come before the service is registered, so a failed
  ; install can still be removed.
  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayName" "Timelapse Render Daemon"
  WriteRegStr HKCU "${UNINST_KEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKCU "${UNINST_KEY}" "Publisher" "Timelapse Render Daemon contributors"
  WriteRegStr HKCU "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${UNINST_KEY}" "NoRepair" 1

  DetailPrint "Creating the logon task"
  ExecWait '"$INSTDIR\trd.exe" service install' $0
  ${If} $0 != 0
    DetailPrint "trd service install failed with exit code $0"
    Abort
  ${EndIf}

  DetailPrint "Starting the daemon"
  ExecWait '"$SYSDIR\schtasks.exe" /Run /TN ${TASK_NAME}' $0
  ${If} $0 != 0
    DetailPrint "schtasks /Run exited with $0; the task starts at the next logon"
  ${EndIf}
SectionEnd

Section "Uninstall"
  ; Without an install directory there is nothing to remove, and RMDir /r on an empty path must
  ; not run.
  ${If} $INSTDIR == ""
    Abort
  ${EndIf}

  ExecWait '"$INSTDIR\trd.exe" service uninstall'
  DeleteRegKey HKCU "${UNINST_KEY}"
  ; NSIS uninstallers copy themselves out of the install directory before they run this, so
  ; removing uninstall.exe here is safe.
  RMDir /r "$INSTDIR"
SectionEnd
