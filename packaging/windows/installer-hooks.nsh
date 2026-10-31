; USB-IP-Bridge: additions to the Windows installer (Tauri NSIS, per-machine install).
; Installs the background service and its firewall rules, and - if missing - the USB drivers:
; usbipd-win, whose signed VBoxUSB drivers USB-IP-Bridge uses to share the devices of this
; computer, and usbip-win2, to use devices of other computers here. Both always, so the
; computer can do either later without asking the user anything then.

!macro NSIS_HOOK_PREINSTALL
  ; Close the app first (also in other user sessions): the installer's own check comes after
  ; this hook and would stop the update with the service already stopped.
  nsExec::ExecToLog 'taskkill.exe /f /im usbipb-gui.exe'
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
      MessageBox MB_ICONEXCLAMATION "usbipd-win could not be installed (error $0).$\r$\nWithout it, this computer cannot share its USB devices." /SD IDOK
    ${EndIf}
  ${EndIf}

  ; usbip-win2 (to use devices of other computers), unless it is there already (the tool the
  ; service drives; its driver service may linger after an uninstall until Windows restarts)
  ${IfNot} ${FileExists} "$PROGRAMFILES64\USBip\usbip.exe"
    DetailPrint "Installing usbip-win2 (USB drivers) ..."
    ExecWait '"$INSTDIR\usbip-win2-setup.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /RESTARTEXITCODE=3010' $0
    ${If} $0 == 0
    ${OrIf} $0 == 3010
      ; remember to remove it again with USB-IP-Bridge
      FileOpen $2 "$INSTDIR\installed-usbip-win2" w
      FileClose $2
      ${If} $0 == 3010
        MessageBox MB_ICONINFORMATION "Please restart Windows before using devices of other computers here." /SD IDOK
      ${EndIf}
    ${Else}
      MessageBox MB_ICONEXCLAMATION "usbip-win2 could not be installed (error $0).$\r$\nWithout it, this computer cannot use devices of other computers." /SD IDOK
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
  ; Close the app first (also in other user sessions): the uninstaller's own check comes after
  ; this hook and would stop it halfway, with the service and drivers already removed.
  nsExec::ExecToLog 'taskkill.exe /f /im usbipb-gui.exe'
  nsExec::ExecToLog '"$INSTDIR\usbipb-service.exe" uninstall'
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="USB-IP-Bridge (TCP)"'
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="USB-IP-Bridge (UDP)"'
  ; usbipd-win goes, too, if it came with USB-IP-Bridge (not on an update)
  ${If} $UpdateMode <> 1
    IfFileExists "$INSTDIR\installed-usbipd-win" 0 +3
      ExecWait 'msiexec.exe /x "$INSTDIR\usbipd-win.msi" /qn /norestart'
      Delete "$INSTDIR\installed-usbipd-win"
    IfFileExists "$INSTDIR\installed-usbip-win2" 0 +3
      ExecWait '"$PROGRAMFILES64\USBip\unins000.exe" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART'
      Delete "$INSTDIR\installed-usbip-win2"
  ${EndIf}
!macroend
