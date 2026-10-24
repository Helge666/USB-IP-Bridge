; USB-IP-Bridge: additions to the Windows installer (Tauri NSIS, per-machine install).
; Installs the background service and its firewall rules, and - if missing - usbipd-win,
; whose signed VBoxUSB drivers USB-IP-Bridge uses to share the devices of this computer.

!macro NSIS_HOOK_PREINSTALL
  ; An update replaces the service program: stop the service first. Stop-Service waits until
  ; it has stopped (older versions of the program have no "stop" command of their own).
  nsExec::ExecToLog 'powershell.exe -NoProfile -NonInteractive -Command "Stop-Service USB-IP-Bridge -Force -ErrorAction SilentlyContinue"'
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; usbipd-win (USB drivers), unless they are there already
  nsExec::ExecToStack 'sc.exe query VBoxUsbMon'
  Pop $0
  Pop $1
  ${If} $0 != 0
    DetailPrint "Installing usbipd-win (USB drivers) ..."
    ExecWait 'msiexec.exe /i "$INSTDIR\usbipd-win.msi" /qn /norestart' $0
    ${If} $0 == 0
    ${OrIf} $0 == 3010
      ; remember to remove it again with USB-IP-Bridge
      FileOpen $2 "$INSTDIR\installed-usbipd-win" w
      FileClose $2
      ; only its drivers are used; its own server stays off
      nsExec::ExecToLog 'sc.exe stop usbipd'
      nsExec::ExecToLog 'sc.exe config usbipd start= disabled'
    ${Else}
      MessageBox MB_ICONEXCLAMATION "usbipd-win could not be installed (error $0).$\r$\nWithout it, this computer cannot share its USB devices."
    ${EndIf}
  ${EndIf}

  ; Firewall: other computers reach the service in private networks only.
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="USB-IP-Bridge (TCP)"'
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="USB-IP-Bridge (UDP)"'
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="USB-IP-Bridge (TCP)" dir=in action=allow profile=private program="$INSTDIR\usbipb-service.exe" protocol=TCP localport=3242,3244'
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="USB-IP-Bridge (UDP)" dir=in action=allow profile=private program="$INSTDIR\usbipb-service.exe" protocol=UDP localport=3244'

  ; The service (installing again brings an existing one up to date and starts it).
  nsExec::ExecToLog '"$INSTDIR\usbipb-service.exe" install'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog '"$INSTDIR\usbipb-service.exe" uninstall'
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="USB-IP-Bridge (TCP)"'
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="USB-IP-Bridge (UDP)"'
  ; usbipd-win goes, too, if it came with USB-IP-Bridge (not on an update)
  ${If} $UpdateMode <> 1
    IfFileExists "$INSTDIR\installed-usbipd-win" 0 +3
      ExecWait 'msiexec.exe /x "$INSTDIR\usbipd-win.msi" /qn /norestart'
      Delete "$INSTDIR\installed-usbipd-win"
  ${EndIf}
!macroend
