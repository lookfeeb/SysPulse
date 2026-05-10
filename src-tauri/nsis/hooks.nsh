; SysPulse NSIS installer hooks
; Kill running instances before installing to prevent file-in-use errors.

!macro NSIS_HOOK_PREINSTALL
  ; Silently kill the main app process
  nsExec::ExecToLog 'taskkill /F /IM "SysPulse.exe" /T'
  ; Kill hw-helper subprocess
  nsExec::ExecToLog 'taskkill /F /IM "hw-helper.exe" /T'
  ; Brief pause to let file handles release
  Sleep 1500
!macroend
