!macro NSIS_HOOK_PREINSTALL
  IfFileExists "$INSTDIR\salamander-proxy.exe" 0 salamander_proxy_preinstall_done

  DetailPrint "Stopping Salamander proxy workers before replacing application files"
  nsExec::ExecToStack '"$SYSDIR\taskkill.exe" /F /T /IM "salamander-proxy.exe"'
  Pop $0
  Pop $1
  Sleep 1000

  ; Removing the old sidecar first prevents NSIS from retaining a same-version
  ; or previously locked executable while updating the main application.
  Delete "$INSTDIR\salamander-proxy.exe"

  salamander_proxy_preinstall_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "Stopping Salamander network sidecars before uninstall"
  nsExec::ExecToStack '"$SYSDIR\taskkill.exe" /F /T /IM "salamander-proxy.exe"'
  Pop $0
  Pop $1
!macroend
