; ChatDock installer hooks (Tauri NSIS installer, see installer.nsi)
;
; "ChatDockUpdTest" is a throw-away copy for testing installs and updates: it has its own Electron
; copy, data folder and "start with Windows" entry, and never touches the real ChatDock's.

!define CHATDOCK_RUN_KEY "Software\Microsoft\Windows\CurrentVersion\Run"
!define CHATDOCK_APPROVED_KEY "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run"

!macro CHATDOCK_NAMES
  !if "${PRODUCTNAME}" == "ChatDockUpdTest"
    StrCpy $R5 "$LOCALAPPDATA\Programs\chatdock-updtest"
    StrCpy $R6 "Uninstall ChatDockUpdTest.exe"
    StrCpy $R7 "com.chatdock.updtest"
  !else
    StrCpy $R5 "$LOCALAPPDATA\Programs\chatdock"
    StrCpy $R6 "Uninstall ChatDock.exe"
    StrCpy $R7 "com.chatdock.app"
  !endif
!macroend

; One line in %APPDATA%\<product>\install.log (next to chatdock.log), for tracing a failed update.
!macro CHATDOCK_LOG text
  CreateDirectory "$APPDATA\${PRODUCTNAME}"
  FileOpen $R4 "$APPDATA\${PRODUCTNAME}\install.log" a
  ${If} $R4 != ""
    FileSeek $R4 0 END
    FileWrite $R4 "${VERSION}: ${text}$\r$\n"
    FileClose $R4
  ${EndIf}
!macroend

; Moving over from an Electron build of ChatDock (Beta Build 1.4 and older): remove that copy
; first. Its settings and chats in %APPDATA%\ChatDock stay, and this version carries on with them
; (the app copies the logins over when it first starts).
; --updated tells its uninstaller that this is an update (it then keeps "Start with Windows").
; It runs as a copy in the temp folder with _?= (what NSIS uninstallers do themselves): then it
; can remove its own file too, and ExecWait really waits for it.
!macro NSIS_HOOK_PREINSTALL
  !insertmacro CHATDOCK_NAMES
  ${If} ${FileExists} "$R5\$R6"
    DetailPrint "Removing the previous ChatDock..."
    ReadRegStr $R8 HKCU "${CHATDOCK_RUN_KEY}" "$R7"
    !insertmacro CHATDOCK_LOG "Electron copy found in $R5 (start with Windows: '$R8')"
    InitPluginsDir
    CopyFiles /SILENT "$R5\$R6" "$PLUGINSDIR\chatdock-old-uninstall.exe"
    ExecWait '"$PLUGINSDIR\chatdock-old-uninstall.exe" /S --updated _?=$R5' $R9
    Delete "$PLUGINSDIR\chatdock-old-uninstall.exe"
    !insertmacro CHATDOCK_LOG "its uninstaller finished with $R9"
    Delete "$R5\$R6"
    RMDir /r "$R5"
    ${If} $R8 != ""
      ; "Start with Windows" stays on, now pointing at this ChatDock
      WriteRegStr HKCU "${CHATDOCK_RUN_KEY}" "$R7" '"$INSTDIR\${MAINBINARYNAME}.exe" --hidden'
      ReadRegStr $R8 HKCU "${CHATDOCK_RUN_KEY}" "$R7"
      !insertmacro CHATDOCK_LOG "start with Windows now '$R8'"
    ${EndIf}
  ${EndIf}
!macroend

; ChatDock writes "Start with Windows" itself, so the uninstaller wouldn't know to remove it; and
; its data folder is %APPDATA%\<product name> (Tauri's own checkbox only knows the bundle id folders).
; Neither happens when the uninstaller runs as part of an update.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    !insertmacro CHATDOCK_NAMES
    DeleteRegValue HKCU "${CHATDOCK_RUN_KEY}" "$R7"
    DeleteRegValue HKCU "${CHATDOCK_APPROVED_KEY}" "$R7"
    ${If} $DeleteAppDataCheckboxState = 1
      RMDir /r "$APPDATA\${PRODUCTNAME}"
    ${EndIf}
  ${EndIf}
!macroend
