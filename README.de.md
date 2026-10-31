# USB-IP-Bridge

USB-Geräte eines anderen Computers über das Netzwerk nutzen, als wären sie direkt eingesteckt.

*[English](README.md)*

**Download:** [Windows 10 / 11](https://github.com/Helge666/USB-IP-Bridge/releases/latest/download/USB-IP-Bridge_0.2.0_x64-setup.exe)
· [Linux (Debian, Ubuntu)](https://github.com/Helge666/USB-IP-Bridge/releases/latest/download/USB-IP-Bridge_0.2.0_amd64.deb)
· [alle Versionen](https://github.com/Helge666/USB-IP-Bridge/releases) – danach siehe [Installieren](#installieren).

Dein Audio-Interface hängt am Windows-PC, aber aufnehmen willst du am Linux-PC? Mit
USB-IP-Bridge holt sich der Linux-PC das Gerät über das Netzwerk und gibt es zurück, wenn du
fertig bist. Das klappt mit Audio- und MIDI-Interfaces, Kameras, Grafiktabletts,
Synthesizern, Laufwerken und den meisten anderen USB-Geräten.

- Läuft als Hintergrunddienst auf jedem Computer; Computer im lokalen Netzwerk finden sich selbst.
- Eine einfache App: Gerät freigeben, an einen anderen Computer geben, zurückholen.
- Nur gekoppelte Computer: Die Computer werden einmalig mit einem 6-stelligen Code
  gekoppelt; die Verbindung zwischen ihnen ist verschlüsselt.
- Basiert auf USB/IP: Unter Linux arbeiten die Treiber des Kernels, unter Windows signierte
  Treiber (usbipd-win, usbip-win2).

<p align="center"><img src="docs/screenshot.png" width="520" alt="USB-IP-Bridge auf einem Windows-PC: seine USB-Geräte (die meisten freigegeben, einige gesperrt), ein zweiter Windows-PC und drei gekoppelte Linux-PCs, dazu die Details eines davon, der eigene Geräte teilt und die anderer nutzt"></p>

> **Stand: v0.2.0.** Jeder Computer kann eigene Geräte teilen und die anderer nutzen,
> zwischen Windows und Linux in alle Richtungen. Täglich im Einsatz mit einem Behringer XR18
> (18×18 Audio, Vollduplex, ~26 ms Rundweg über kabelgebundenes Ethernet), getestet mit
> MIDI-Interfaces, Webcams, Grafiktablett, Synthesizer, Headsets, Maus, USB-Sticks und
> Festplatten. Mit Ecken und Kanten ist zu rechnen.

## Was geht wo

| System | Eigene Geräte teilen | Geräte anderer nutzen |
|--------|----------------------|-----------------------|
| Windows 10 / 11, x64 (getestet mit 11) | ✅ | ✅ |
| Linux x64 mit systemd (getestet mit Ubuntu 24.04) | ✅ | ✅ |
| macOS, Android | später | später |

## Installieren

Den Installer gibt es bei den [Releases](https://github.com/Helge666/USB-IP-Bridge/releases).

**Windows:** `USB-IP-Bridge_0.2.0_x64-setup.exe` ausführen. Er installiert die App, den
Hintergrunddienst und Firewall-Regeln für private Netzwerke. Für die USB-Treiber installiert
er außerdem [usbipd-win](https://github.com/dorssel/usbipd-win) (zum Teilen) und
[usbip-win2](https://github.com/vadimgrn/usbip-win2) (zum Nutzen von Geräten anderer
Computer), falls sie noch fehlen.

**Linux (Debian, Ubuntu und Verwandte):**

```bash
sudo apt install ./USB-IP-Bridge_0.2.0_amd64.deb
```

Das installiert die App (Menüeintrag „USB-IP-Bridge“), den Dienst, die USB/IP-Kernelmodule
und eine Firewall-Regel (ufw) für andere Computer.

## Erste Schritte

1. USB-IP-Bridge auf jedem Computer installieren, der Geräte teilen oder nutzen soll.
2. Die App öffnen. Die anderen Computer erscheinen links (im selben lokalen Netzwerk;
   andere lassen sich per Name oder Adresse hinzufügen).
3. Die Computer einmalig **koppeln**: den anderen Computer wählen, *Koppeln…* und den hier
   angezeigten Code in der App auf dem anderen Computer eingeben.
4. Am Computer, an dem das Gerät steckt, es wählen und *An … geben* (oder *Freigeben*). Am
   anderen Computer das freigegebene Gerät wählen und *Hierher holen*. *Zurückgeben* /
   *Zurückholen* bringt es wieder zurück.

Für Programme, die ein Gerät nur brauchen, solange sie laufen, holt die Kommandozeile es
und gibt es danach zurück:

```bash
usbipb run --device 1397:00d4 -- meine-daw
```

## Gut zu wissen

- Ein USB-Gerät kann immer nur ein Computer zur Zeit nutzen. Solange es ein anderer
  benutzt, fehlt es am eigenen Computer.
- Das Netzwerk zählt: Audio-Interfaces und Kameras wollen ein Kabel (Gigabit empfohlen);
  für MIDI, Tabletts und Ähnliches reicht WLAN.
- Programme, die ein Gerät offen halten (z. B. das Kontrollfeld des Herstellers oder eine
  laufende Aufnahme), verhindern das Teilen. Unter Windows bietet die App an, sie zu beenden,
  und startet sie wieder, wenn das Gerät zurück ist; unter Linux nennt sie das Programm.
- Tastaturen, Mäuse, Netzwerk- und Bluetooth-Adapter sind automatisch gesperrt, damit sie nicht
  aus Versehen weggegeben werden. Rechtsklick auf ein Gerät sperrt oder entsperrt es.
- Windows: Das Netzwerk muss auf *privat* stehen (Einstellungen → Netzwerk → Eigenschaften); in
  einem *öffentlichen* Netzwerk finden sich die Computer nicht.
- Nur Computer koppeln, die dir gehören.
- Die Daten auf der USB/IP-Verbindung selbst sind nicht verschlüsselt (angenommen werden
  nur gekoppelte Computer); im eigenen Netz verwenden, oder über ein VPN zwischen Netzen.

## Kommandozeile

`usbipb panel` öffnet ein Bedienpanel im Terminal (nur ASCII, auch über SSH) mit denselben
Funktionen wie die App.

`usbipb list | status | attach [N] | detach [N] | share/unshare VID:PID | pair COMPUTER |
pair-answer ID CODE | unpair COMPUTER | run [--device D] -- PROGRAMM`

Konfiguration (optional): `C:\ProgramData\USB-IP-Bridge\config.toml` unter Windows,
`/etc/usb-ip-bridge/config.toml` unter Linux; siehe [packaging/examples](packaging/examples).

## Deinstallieren

Windows: *Einstellungen → Apps → USB-IP-Bridge → Deinstallieren* (entfernt auch usbipd-win
und usbip-win2, falls sie mit USB-IP-Bridge kamen). Linux: `sudo apt remove usb-ip-bridge`
(`purge` entfernt zusätzlich den Schlüssel und die Liste der gekoppelten Computer).

## Selbst bauen

Siehe [README.md](README.md#build-from-source).

## Lizenz

[GPL-3.0-or-later](LICENSE)
