# USB-IP-Bridge

USB-Geräte eines anderen Computers über das Netzwerk nutzen, als wären sie direkt eingesteckt.

*[English](README.md)*

Dein Audio-Interface hängt am Windows-PC, aber aufnehmen willst du am Linux-PC? Mit
USB-IP-Bridge holt sich der Linux-PC das Gerät über das Netzwerk und gibt es zurück, wenn du
fertig bist. Das klappt mit Audio- und MIDI-Interfaces, Kameras, Grafiktabletts,
Synthesizern, Laufwerken und den meisten anderen USB-Geräten.

- Läuft als Hintergrunddienst auf jedem Computer; Computer im lokalen Netzwerk finden sich selbst.
- Eine einfache App: Gerät freigeben, an einen anderen Computer geben, zurückholen.
- Nur gekoppelte Computer: Die Computer werden einmalig mit einem 6-stelligen Code
  gekoppelt; die Verbindung zwischen ihnen ist verschlüsselt.
- Basiert auf USB/IP: Auf der empfangenden Seite kümmert sich der Treiber des Linux-Kernels um das Gerät.

<p align="center"><img src="docs/screenshot.png" width="520" alt="USB-IP-Bridge auf einem Windows-PC: seine USB-Geräte (einige freigegeben, einige gesperrt, drei von einem Linux-PC benutzt), zwei gekoppelte Linux-PCs und die Details dieses Computers"></p>

> **Stand: v0.1.1, die erste öffentliche Version.** Täglich im Einsatz mit einem Behringer
> XR18 (18×18 Audio, Vollduplex, ~26 ms Rundweg über kabelgebundenes Ethernet), getestet mit
> MIDI-Interfaces, Webcam, Grafiktablett, Synthesizer und USB-Sticks. Mit Ecken und Kanten ist zu rechnen.

## Was geht wo

| System | Eigene Geräte teilen | Geräte anderer nutzen |
|--------|----------------------|-----------------------|
| Windows 10 / 11, x64 (getestet mit 11) | ✅ | geplant |
| Linux x64 mit systemd (getestet mit Ubuntu 24.04) | geplant | ✅ |
| macOS, Android | später | später |

## Installieren

Den Installer gibt es bei den [Releases](https://github.com/Helge666/USB-IP-Bridge/releases).

**Windows:** `USB-IP-Bridge_0.1.1_x64-setup.exe` ausführen. Er installiert die App, den
Hintergrunddienst und Firewall-Regeln für private Netzwerke. Für die USB-Treiber installiert
er außerdem [usbipd-win](https://github.com/dorssel/usbipd-win), falls es noch fehlt.

**Linux (Debian, Ubuntu und Verwandte):**

```bash
sudo apt install ./USB-IP-Bridge_0.1.1_amd64.deb
```

Das installiert die App (Menüeintrag „USB-IP-Bridge“), den Dienst und den virtuellen USB-Host-Controller.

## Erste Schritte

1. USB-IP-Bridge auf jedem Computer installieren, der Geräte teilen oder nutzen soll.
2. Die App öffnen. Die anderen Computer erscheinen links (im selben lokalen Netzwerk;
   andere lassen sich per Name oder Adresse hinzufügen).
3. Die Computer einmalig **koppeln**: den anderen Computer wählen, *Koppeln…* und den hier
   angezeigten Code in der App auf dem anderen Computer eingeben.
4. Unter Windows ein Gerät wählen und *An … geben* (oder *Freigeben*). Unter Linux ein
   freigegebenes Gerät des anderen Computers wählen und *Hierher holen*. *Zurückgeben* /
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
- Programme, die ein Gerät offen halten (z. B. das Kontrollfeld des Herstellers),
  verhindern das Teilen; die App bietet an, sie zu beenden, und startet sie wieder, wenn das
  Gerät zurück ist.
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

Windows: *Einstellungen → Apps → USB-IP-Bridge → Deinstallieren* (entfernt auch usbipd-win,
falls es mit USB-IP-Bridge kam). Linux: `sudo apt remove usb-ip-bridge` (`purge` entfernt
zusätzlich den Schlüssel und die Liste der gekoppelten Computer).

## Selbst bauen

Siehe [README.md](README.md#build-from-source).

## Lizenz

[GPL-3.0-or-later](LICENSE)
