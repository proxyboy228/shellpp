Unicode true
Name "Shell++"
OutFile "${INSTALLER}"
InstallDir "$LOCALAPPDATA\Programs\Shell++"
RequestExecutionLevel user
SetCompressor /SOLID lzma

!include "MUI2.nsh"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\ShellPP"

Section "Shell++"
    SetShellVarContext current
    SetOutPath "$INSTDIR"
    SetOverwrite on
    ClearErrors
    File "${PAYLOAD}\shellpp.exe"
    File "${PAYLOAD}\LICENSE"
    File "${PAYLOAD}\THIRD_PARTY_NOTICES.md"
    IfErrors failed
    WriteUninstaller "$INSTDIR\Uninstall.exe"
    IfErrors failed
    CreateShortcut "$SMPROGRAMS\Shell++.lnk" "$INSTDIR\shellpp.exe"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "Shell++"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\shellpp.exe"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '$\"$INSTDIR\Uninstall.exe$\"'
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1
    IfErrors failed
    Goto done
failed:
    Abort "Installation failed. Close Shell++ and try again."
done:
SectionEnd

Section "Uninstall"
    SetShellVarContext current
    ClearErrors
    Delete "$INSTDIR\shellpp.exe"
    IfErrors failed
    Delete "$INSTDIR\LICENSE"
    Delete "$INSTDIR\THIRD_PARTY_NOTICES.md"
    Delete "$SMPROGRAMS\Shell++.lnk"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir "$INSTDIR"
    Goto done
failed:
    Abort "Close Shell++ and try again."
done:
SectionEnd
