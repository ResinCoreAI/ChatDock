; Extra uninstall step: remove "Start with Windows" (ChatDock writes it itself, so the installer
; wouldn't know about it). Skipped when the uninstaller runs as part of an update.
!macro customUnInstall
  ${ifNot} ${isUpdated}
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "com.chatdock.app"
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run" "com.chatdock.app"
  ${endIf}
!macroend
