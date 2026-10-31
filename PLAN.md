# USB-IP-Bridge – Projektplan

> Stand: 2026-10-31 · v0.2.0 (alle Richtungen)

---

## 1. Vision in einem Satz

USB-IP-Bridge ist ein plattformübergreifender Hintergrunddienst mit App, der USB-Geräte eines
Rechners über das Netzwerk für andere Rechner freigibt, sodass diese die Geräte so nutzen können,
als wären sie lokal angeschlossen.

---

## 2. Anforderungen

| # | Anforderung | Stand |
|---|-------------|-------|
| R1 | Name: **USB-IP-Bridge** | ✅ |
| R2 | Betriebssysteme: **Windows, Linux** (später macOS, Android) | ✅ Windows teilt, Linux nutzt |
| R3 | UI-Sprachen: **Englisch, Deutsch**; von Anfang an lokalisierbar | ✅ |
| R4 | Läuft als **Dienst** (Windows-Dienst / systemd) | ✅ |
| R5 | Erkennt USB-Geräte (inkl. Hotplug) | ✅ |
| R6 | Instanzen verbinden sich – zuerst **LAN**, später **Internet** | ✅ LAN (Internet: über VPN) |
| R7 | Mehrere Verbindungen zu mehreren entfernten Geräten gleichzeitig | ✅ |
| R8 | App zeigt alle Geräte aller bekannten Rechner (inkl. lokaler) | ✅ |
| R9 | App ist **selbsterklärend** | ✅ (Einführung beim ersten Start) |
| R10 | Entfernte Geräte per App **lokal einbinden** | ✅ |
| R11 | **Hinweis**, wenn ein Gerät nicht mehrfach nutzbar ist (siehe 4.3) | ✅ |
| R12 | Transportprotokoll **selbsttätig** wählen (TCP, UDP/QUIC …) | TCP; Auto-Wahl später |
| R13 | **Sicherheit**: Kopplung, verschlüsselter Steuerkanal | ✅ v0.1.0 |
| R14 | Versionierung **vMAJOR.MINOR.PATCH**; Titelleiste zeigt die Version | ✅ |
| R15 | **Open Source**, nicht kommerziell (GPL-3.0-or-later) | ✅ |
| R16 | Datenpfad kompatibel zu Standard-`usbip` (Linux) | ✅ (mit `require_pairing = false`) |
| R17 | **Bestehende Systeme nicht stören:** keine globalen Audio-, Grafik- oder Display-Einstellungen ändern; Spiel-/Systemton nie umleiten | ✅ |
| R18 | **Sperrliste:** systemkritische Geräte (Tastatur, Lizenz-Dongle, …) werden nie freigegeben | ✅ |
| R19 | **Audio-Interfaces im Vollduplex** (Aufnahme und Wiedergabe gleichzeitig, Multitracking) | ✅ |

### 2.1 Zielgruppe & Referenzszenario

**Zielgruppe:** Privatanwender, besonders Musiker. Daraus folgt: Installer, keine
Konfigurationsdateien nötig, einfache Kopplung per Code, Heimnetz als Normalfall.

**Referenzszenario:**

```
┌──────────────────────────┐   Heim-LAN   ┌──────────────────────────┐
│ Windows-PC               │◄────────────►│ Linux-PC                 │
│  └─ USB: Behringer XR18  │              │  nutzt das XR18 in der   │
│     (18×18 Audio + MIDI) │              │  DAW wie lokal           │
│  Rolle: gibt frei        │              │  Rolle: bindet ein       │
└──────────────────────────┘              └──────────────────────────┘
```

| Aspekt | Bedeutung |
|--------|-----------|
| **Richtung Windows → Linux** | Windows muss **freigeben**; der Windows-Export ist Kernbestandteil (siehe 4.1). |
| **Kein eigener Kerneltreiber** | Windows: signierte VBoxUSB-Treiber aus usbipd-win; Linux: `vhci-hcd` (im Kernel). |
| **Audio = isochrone Transfers** | USB-2.0-Audio mit 18 Ein-/Ausgängen; isochron muss von Anfang an funktionieren. |
| **Bandbreite** | ≈ 33 Mbit/s je Richtung bei 18×18 Kanälen → kabelgebundenes LAN unkritisch, WLAN riskant. |
| **Latenz** | Gemessen ~26 ms Rundweg (Puffer 256 bei 48 kHz). Live-Monitoring über den Mixer des Interfaces. |
| **Besitz** | Das Gerät gehört standardmäßig dem Windows-PC; der Linux-PC bekommt es nur auf Wunsch und gibt es danach zurück. |

### 2.2 Audio unter Linux

Ziel: **Jede** Software auf dem Linux-PC kann das entfernte Audio-Interface nutzen, ohne dass
sich am restlichen Ton etwas ändert.

| Regel | Begründung |
|-------|------------|
| **PipeWire beibehalten** | Bedient ALSA-, PulseAudio- und JACK-Programme gleichzeitig. |
| **Keine globalen Änderungen** an PipeWire | Takt und Quantum bleiben; Spiel- und Systemton bleiben auf ihrem Ausgang. |
| **Eine WirePlumber-Regel für per USB/IP eingebundene Soundkarten** | Nie Standardgerät, Pro-Audio-Profil, Puffer (`api.alsa.headroom`/`period-size` 256). Der Dienst schreibt sie im Format der installierten WirePlumber-Version und lädt WirePlumber nur neu, wenn gerade kein Ton läuft. |
| DAWs mit vielen Kanälen über **JACK** (`pipewire-jack`) | PipeWire 1.0.5 verwirft AUX-Kanäle bei gemischter Kanalbelegung (Surround + AUX), wie sie manche DAWs für 18 Kanäle wählen; JACK-Ports haben keine Kanalpositionen. |
| Echtzeit-Priorität über **rtkit** | Keine Systemänderung nötig. |

### 2.3 Schutzregeln (R17, R18)

- Die App ändert **niemals** Display-, Grafik- oder globale Audio-Einstellungen.
- Ein Audio-Gerät wird erst eingebunden, wenn die Regel „nie Standardgerät“ aktiv ist.
- WirePlumber wird nur neu geladen, wenn kein Ton läuft (keine Streams, kein JACK-Programm).
- **Sperren:** Geräte, deren Profil vor Aussperren oder Netzverlust warnt (Tastatur, Maus,
  Netzwerk, Bluetooth), sind automatisch gesperrt; jedes Gerät lässt sich in der App sperren
  bzw. entsperren (gemerkt in `shares.json`). `never_share` in der Konfiguration sperrt endgültig.
  Die Profile warnen zusätzlich vor Datenverlust (Speicher).

### 2.4 Fallback

Falls USB-Audio über das Netz für ein Gerät nicht stabil genug ist: Audio auf höherer Ebene
übertragen (PipeWire/JACK-Netzwerk-Audio, VBAN). Nicht Teil dieser App.

---

## 3. Begriffe

| Begriff | Bedeutung |
|---------|-----------|
| **Host / Instanz** | Ein Rechner, auf dem USB-IP-Bridge läuft. |
| **Exporting Host** | Host, an dem das USB-Gerät steckt und der es freigibt. |
| **Importing Host** | Host, der ein entferntes Gerät einbindet. |
| **Freigeben (Share)** | Gerät für gekoppelte Rechner verfügbar machen. Solange ein anderer Rechner es nutzt, ist es lokal weg. |
| **Geben an / Zurückholen** | Ein freigegebenes Gerät aktiv an einen Rechner geben bzw. zurückfordern. |
| **Einbinden (Attach) / Zurückgeben** | Entferntes Gerät über den virtuellen USB-Controller anmelden bzw. abgeben. |
| **Koppeln (Pairing)** | Zwei Rechner vertrauen sich, nachdem ein Code übertragen wurde (siehe 7). |

---

## 4. Technische Grundlagen und Erkenntnisse

### 4.1 Kernel-Treiber

| OS | Einbinden (Client) | Freigeben (Server) |
|----|--------------------|--------------------|
| **Linux** | `vhci-hcd` (Mainline-Kernel) ✅ | `usbip-host` oder User-Space (geplant) |
| **Windows** | usbip-win2 (geplant) | **VBoxUSB-Treiber aus usbipd-win** (signiert) ✅ |
| **macOS** (später) | virtueller Host-Controller (DriverKit) – aufwendig | User-Space via IOKit/libusb |
| **Android** (später) | ohne Root praktisch nicht möglich | über die USB-Host-API möglich |

Wir entwickeln **keinen eigenen Kerneltreiber**.

**Windows-Export – Erkenntnisse aus den Spikes (2026-10-21):**
- **usbipd-win** direkt: Bulk funktioniert, isochrones High-Speed-Audio nicht. Ursache im
  Zusammenspiel mit VBoxUSB: (1) VBoxUSB setzt sein High-Speed-Flag nur in
  `SUPUSB_IOCTL_GET_DEVICE`, das usbipd-win nie aufruft → isochrone URBs werden wie Full Speed
  geplant (8 ms statt 1 ms pro 8-Paket-URB). (2) Linux schickt Wiedergabe-URBs mit 7 Paketen;
  Windows verlangt bei High-Speed-Isochron ganze 1-ms-Frames (Vielfaches von 8 Paketen).
- **Lösung (eigener Server auf VBoxUSB):** Gerät per VBoxUsbMon übernehmen (kein Treibertausch),
  `GET_DEVICE` senden, isochrone OUT-Pakete auf ganze Frames (8 Pakete) neu bündeln und
  Linux-URBs erst bestätigen, wenn ihre Pakete gesendet sind. Ergebnis: 10 min Vollduplex
  18×18 ohne Xruns, Testton bitgenau zurück.
- **WinUSB** ungeeignet für Audio-Wiedergabe (feste Paketgröße je Intervall) und Vollduplex
  (`SetCurrentAlternateSetting` scheitert, solange die andere Richtung streamt).
- **UsbDk** nicht verwenden (nur Red-Hat-signiert, Berichte über lahmgelegte Windows-11-Systeme mit HVCI).
- Windows verweigert die Herausgabe, solange ein Prozess das Gerät offen hält
  (`PNP_VetoOutstandingOpen`, z. B. das Kontrollfeld des Herstellers) → die App nennt bekannte
  Blockierer und bietet „beenden und freigeben“ an; das Programm startet nach der Rückgabe wieder.
- SuperSpeed-Geräte meldet der Hub-IOCTL als High Speed → Erkennung über `bMaxPacketSize0 = 9`.
- Socket-Optionen gelten unter Windows pro dupliziertem Handle (`try_clone`).
- Der Server muss als echter Windows-Dienst laufen (nicht als Kindprozess eines anderen Programms).

**Linux-Import:** eigener Code statt `usbip`-Werkzeug: Handshake im User-Space, dann Socket an
`vhci_hcd` übergeben. Ein frisch eingebundener Port ist zunächst `NOTASSIGNED` (5), erst nach der
Adressierung `USED` (6).

### 4.2 Protokoll

- **Datenpfad: USB/IP** über TCP (Port 3242, weil usbipd-win 3240 belegt). Kompatibel zu
  Standard-`usbip attach`, wenn `require_pairing = false`.
- **Steuerkanal** (TCP 3244): JSON-Zeilen in einem Noise-Kanal (siehe 7): Hello, Devices,
  Attach („nimm mein Gerät“), Release („gib es zurück“), Pairing.
- **Discovery** (UDP 3244): eigene JSON-Beacons per Broadcast statt mDNS (Port 5353 teilen sich
  sonst Avahi und andere Dienste); Rechner in anderen Netzen per Name/Adresse hinzufügen.
- Gerätename, Rechnername und Geräteklassen stehen im freien `path`-Feld des
  USB/IP-Gerätedatensatzes (`/usb-ip-bridge/serial=…/name=…/host=…/classes=…`).

### 4.3 USB-Geräte sind exklusiv

Ein USB-Gerät gehört immer genau einem Host-Controller. Die App zeigt „In Benutzung durch X“,
warnt beim Freigeben riskanter Geräte und gibt Geräte nach Gebrauch zurück. Echtes Teilen
(z. B. Druckerfreigabe) ist nicht Teil des Projekts.

### 4.4 Transport

Control/Bulk/Interrupt brauchen zuverlässige Übertragung → TCP. Isochrone Geräte laufen im
kabelgebundenen LAN über TCP mit `TCP_NODELAY` gut. Später: QUIC und Auswahl nach Gerätetyp,
RTT und Netz.

### 4.5 Geräteklassen und Geräteprofile

**Klassen** werden aus den Interface-Deskriptoren erkannt (Windows über den Hub, ohne das Gerät zu
öffnen, plus Setup-Klasse des Treibers; Linux über sysfs):

| Klasse | Risiken | Linux-Integration | Prüfung „bereit“ |
|---|---|---|---|
| Audio | isochron, Latenz, Vollduplex | WirePlumber-Regel (siehe 2.2) | ALSA-Karte + PipeWire-Knoten |
| MIDI | unkritisch | – | ALSA-MIDI-Port |
| Video | hohe Bandbreite | – | `/dev/video*` |
| Speicher | **Datenverlust** | vor der Rückgabe aushängen; kein stilles Wiederverbinden | Blockgerät |
| Eingabe | **Aussperren** | – | `/dev/input/*` |
| Seriell | wechselnde Gerätenamen | – | `/dev/tty*` |
| Netzwerk / Bluetooth | eigene Verbindung kann abreißen | – | – |

**Profile** (TOML, `crates/core/profiles/builtin.toml`, eigene Dateien unter
`%ProgramData%\USB-IP-Bridge\profiles\` bzw. `/etc/usb-ip-bridge/profiles/`): Klasse,
Warnungen, bekannte blockierende Programme, Bereitschaftsprüfungen, Wiederverbinden,
Linux-Audio-Einstellungen. Das XR18 ist das erste gerätespezifische Profil.

**`usbipb run --device D -- PROGRAMM`**: Gerät holen, warten bis bereit, Programm starten, Gerät
danach zurückgeben – ersetzt gerätespezifische Startskripte.

**Getestet (2026-10-23):** Behringer XR18 (Audio+MIDI, USB 2 isochron), Elgato Facecam (USB 3
SuperSpeed), Wacom-Tablett, Behringer PRO 800 (MIDI), MIDISPORT 4x4, USB-Sticks (Prüfsumme
identisch, ~14 MB/s über USB/IP vs. ~16 MB/s direkt).

**Erkenntnisse:** Speichertests immer gegen ungecachte Lesevorgänge prüfen (ein defekter Stick
lieferte bei jedem Lesen andere Daten, unter Windows vom Cache verdeckt). Der Linux-Desktop hängt
Sticks automatisch ein → Aushängen vor der Rückgabe ist Pflicht. Manche Programme senden MIDI an
ALSA-Sequencer-Ports, ohne sich anzumelden; Kernel-MIDI-Ports öffnen das Gerät aber nur bei einer
Anmeldung – das betrifft lokale Geräte genauso (Abhilfe: eine dauerhafte Anmeldung, z. B. `aconnect`).

---

## 5. Architektur

```
┌──────────────── Rechner A ────────────────┐            ┌──────── Rechner B ────────┐
│  App (Tauri) ─┐                           │  Steuer-   │                           │
│  CLI usbipb ──┼─ lokale API ─► Dienst ◄───┼──kanal ────┼─► Dienst                  │
│               (127.0.0.1:3243)  │         │ (Noise)    │     │                     │
│                                 │         │  USB/IP    │     │                     │
│                     Export / Import ◄─────┼────────────┼─► Export / Import         │
│                     (VBoxUSB / vhci-hcd)  │            │                           │
└───────────────────────────────────────────┘            └───────────────────────────┘
```

| Komponente | Aufgabe |
|------------|---------|
| **Dienst** (`usbipb-service`) | Läuft als SYSTEM/root; Export (Windows), Import (Linux), Discovery, Steuerkanal, Kopplung, lokale API. |
| **App** (`gui/`, Tauri 2) | Läuft als Benutzer, spricht nur mit dem lokalen Dienst; Baum aller Rechner und Geräte, Details, Aktionen, Kopplung. |
| **CLI** (`usbipb`) | Für Skripte, Tests und Rechner ohne Desktop. |
| **Crates** | `usbip-protocol` (USB/IP-Codec), `core` (Modell, Konfiguration, Profile, API, Steuerprotokoll, Kryptografie), `host` (Windows-/Linux-Geräte, Export, Import, Audio), `service`, `cli`. |

---

## 6. App-Konzept

- Zweispaltig wie ein Datei-Explorer: links ein Baum (dieser Computer oben, dann die anderen;
  Zustands-Chips wie „freigegeben“, „→ Rechner“, „hier“, 🔒), rechts Zustand, Hinweise,
  Verbindung, Details und Geräteprofil des gewählten Eintrags mit den Aktionen als Knöpfe;
  dieselben Aktionen im Kontextmenü. Vollständig per Tastatur bedienbar.
- Klare Sprache statt Fachbegriffe; jeder Zustand mit Farbe, Symbol und Text; riskante Aktionen
  mit verständlicher Bestätigung; Fehlermeldungen sagen, was passiert ist und was zu tun ist.
- Hinweise oben: Kopplungscode bzw. Code-Eingabe, Programme, die ein Gerät blockieren
  („beenden und freigeben“), Audio-Regel noch nicht aktiv.
- Einführung beim ersten Start („So geht's“), Sprachwahl, Version in der Titelleiste.

---

## 7. Sicherheit (umgesetzt in v0.1.0)

- Jede Installation erzeugt beim ersten Start ein **X25519-Schlüsselpaar** (Windows:
  `%ProgramData%\USB-IP-Bridge\host-key`, nur SYSTEM/Administratoren; Linux:
  `/var/lib/usb-ip-bridge/host-key`, 0600).
- **Steuerkanal:** Noise-Protokoll `Noise_XX_25519_ChaChaPoly_BLAKE2s` – beide Seiten lernen den
  öffentlichen Schlüssel der anderen, alles danach ist verschlüsselt.
- **Kopplung:** Rechner A zeigt einen zufälligen 6-stelligen Code, der Benutzer tippt ihn auf
  Rechner B ein. Beide führen **SPAKE2** mit dem Code aus, gebunden an den Handshake-Hash der
  Noise-Verbindung, und beweisen sich gegenseitig den abgeleiteten Schlüssel. Ein Angreifer
  dazwischen hat genau einen Rateversuch pro Kopplungsversuch. Danach merkt sich jede Seite den
  öffentlichen Schlüssel der anderen (`paired.json`).
- **Berechtigung:** Nur gekoppelte Rechner bekommen Geräteliste, Attach/Release und dürfen per
  USB/IP einbinden. Der Export lässt eine Adresse nur zu, wenn sich der Rechner dort in den
  letzten 60 s auf dem Steuerkanal als gekoppelt ausgewiesen hat. Ein Rechner, der mit der Host-ID
  eines gekoppelten Rechners, aber anderem Schlüssel auftritt, wird abgelehnt.
- **Grenzen:** Der USB/IP-Datenstrom selbst ist nicht verschlüsselt (keine Zusatz-Latenz); die
  Zulassung nach Adresse ist im eigenen LAN ausreichend, nicht gegen einen Angreifer, der
  Adressen fälschen kann. Die lokale API (127.0.0.1) prüft keine Benutzerrechte.

**Später:** Verschlüsselung des Datenpfads (mit der Internet-Phase), Berechtigungen pro
Rechner/Gerät, lokale Benutzerrechte, Security-Review, Fuzzing.

---

## 8. Phasen

### Erledigt
- **Phase 0 – Machbarkeit (2026-10-21):** Spikes zu usbipd-win, WinUSB, VBoxUSB (siehe 4.1).
- **Phase 1 – v0.0.1 (2026-10-21):** Windows-Dienst (Export), Linux-Dienst (Import, Einbinden
  auf Wunsch, automatisches Wiederverbinden), CLI, App, „beenden und freigeben“.
- **v0.1.0 (2026-10-24):** Geräteklassen und -profile, `usbipb run`, Speichergeräte sicher
  aushängen, Linux-Audio-Regel, Discovery und Steuerkanal, neue App (Baum, Geben an,
  Zurückholen, Rechner hinzufügen), Sprachwahl, **Kopplung und Verschlüsselung**, **Installer**
  (Windows-Setup mit Dienst, Firewall und usbipd-win; Linux-.deb), Einführung beim ersten Start.
- **v0.1.1 (2026-10-24):** Rechner-Details (Betriebssystem, CPU, Grafik, RAM), automatische
  Sperren für wichtige Geräte plus Sperren/Entsperren per Rechtsklick.
- **v0.2.0 (2026-10-31) – alle Richtungen:** Jeder Rechner gibt eigene Geräte frei und nutzt
  die anderer (Details unten). Getestet Windows→Windows, Linux→Linux, Linux→Windows und
  Windows→Linux mit Audio, MIDI, Video, Eingabe und Speicher; dazu `usbipb panel`,
  Fensterposition merken, Namen nicht eingesteckter Geräte. Offen: High-Speed-Audio mit
  vielen Kanälen in Vollduplex nach Windows (z. B. XR18); Webcam-Befund siehe unten.

### v0.2.0 – alle Richtungen (erledigt)

Ziel: Jeder Rechner kann eigene Geräte freigeben **und** Geräte anderer nutzen –
Windows ↔ Windows, Linux ↔ Linux, Linux → Windows genauso zuverlässig wie heute Windows → Linux.

- **Linux gibt frei:** Kernel-Treiber `usbip-host` (Mainline). Der Dienst bindet ein freigegebenes
  Gerät erst bei Anfrage an `usbip-host` (wie VBoxUSB unter Windows: lokal nutzbar, solange es
  niemand holt), nimmt den USB/IP-Handshake an und übergibt den Socket an den Kernel
  (`usbip_sockfd`); bei Rückgabe wieder an den Originaltreiber. Vorher Speichergeräte aushängen,
  Sperren und Blockierer (Prozesse mit offenem Gerät) wie unter Windows.
- **Windows nutzt Geräte anderer:** **usbip-win2** (BSD-2, Treiber WHLK-zertifiziert bzw.
  Attestation-signiert, Windows 10 1903+). Der Treiber verbindet sich selbst mit dem
  Exportserver; der Dienst stößt das Einbinden an, überwacht es und gibt zurück. Installer:
  usbip-win2 bei Bedarf mitinstallieren (wie usbipd-win), sauber deinstallieren.
- Ausschlüsse: Ein Rechner gibt nie Geräte weiter, die er selbst nur eingebunden hat
  (vhci unter Linux, usbip-win2-Geräte unter Windows).
- App und CLI: „Freigeben“, „Geben an“ und „Hierher holen“ auf allen Systemen; `can_export`/
  `can_import` aus den tatsächlich vorhandenen Treibern statt aus dem Betriebssystem.
- Testmatrix je Richtung (Windows→Windows, Linux→Linux, Linux→Windows, Windows→Linux) mit
  Audio (isochron, Vollduplex), MIDI, Video (USB 3), Eingabe, Speicher.
- Offen zu prüfen: isochrones Audio über usbip-win2 (Vollduplex, Latenz), Zusammenspiel von
  usbip-win2 und den VBoxUSB-Treibern auf demselben Windows-Rechner.
- Befund (2026-10-31): Eine High-Speed-Webcam an einem älteren Linux-Laptop (EHCI, 16 GB RAM)
  geht an Linux (720p/10 fps, 480p/30 fps), aber nicht an Windows: Windows fordert
  isochrone Blöcke von ~380 KB an, der Kernel muss sie über swiotlb umkopieren (höchstens
  256 KB am Stück) – „swiotlb buffer is full“, usbip-host beendet die Verbindung. Mit einem
  neueren Linux-Rechner (64-Bit-DMA, xHCI) gegenprüfen.

### v0.3.0 – macOS (vorgemerkt)

Ziel: Ein Mac nutzt Geräte anderer und gibt eigene frei. Erste Tests auf einem gemieteten
Cloud-Mac (AWS EC2 Mac, tageweise): dort steckt kein USB-Gerät, deshalb holt sich der Mac Geräte
aus dem eigenen Netz (über VPN) und gibt sie – **nur in einem Testmodus** – weiter, um auch das
Freigeben zu prüfen; in Release-Versionen ist das Weitergeben eingebundener Geräte gesperrt
(siehe v0.2.0, Ausschlüsse). Erst wenn das in der Cloud verlässlich läuft: Tests auf echten
Macs mit eigenen Geräten.

- Vorher zu klären: Nutzen erfordert einen virtuellen USB-Hostcontroller
  (IOUSBHostControllerInterface, DriverKit) mit einem von Apple freizuschaltenden Entitlement;
  isochrone Übertragung (Audio) dort laut Apple bisher nicht unterstützt. Ob und wie sich das
  auf einem EC2-Mac zum Entwickeln laden lässt (SIP, Developer-Modus für Systemerweiterungen).
- Freigeben: Gerät per IOKit vom macOS-Treiber übernehmen (Dienst als root).
- Geräte auflisten (IOKit), Pfade für Konfiguration und Zustand, Systeminfo, Installer (.pkg,
  launchd-Dienst).

### Als Nächstes (danach)
1. Windows-Dienst: beim Stoppen warten, bis verliehene Geräte zurück sind.
2. Blockierer unter Windows automatisch finden (Prozesse mit offenen Handles auf das Gerät).
3. Tray-Symbol und Benachrichtigungen (z. B. „XR18 wird von … benutzt“, Kopplungsanfrage).
4. Netzwerk-Gesundheit: Verbindungsgeschwindigkeit, RTT und Neuübertragungen anzeigen; warnen,
   wenn es für ein Gerät eng wird.
5. Events statt Abfrage; „Freigabe anfragen“, wenn ein Gerät belegt ist.
6. JACK-Verdrahtung allgemein (viele Kanäle 1:1), Strg+C in `usbipb run`.
7. Update-Hinweis in der App; CI-Builds der Installer.

### Später
- **Internet**: QUIC, NAT-Traversal, Verschlüsselung des Datenpfads.
- **Android** (zunächst nur Export), weitere Sprachen.

---

## 9. Versionierung

- Semantic Versioning `vMAJOR.MINOR.PATCH`; eine Quelle (Workspace-`Cargo.toml`) für Dienst,
  App, CLI und Installer.
- Git-Tags, CHANGELOG.md im Format „Keep a Changelog“.
- Steuerprotokoll mit eigener Version (aktuell 2); ältere Versionen werden verständlich abgewiesen.

## 10. Qualitätssicherung

- Unit-Tests (Protokoll, Konfiguration, Klassen, Profile, Kryptografie-Kanal und Kopplung).
- Hardware-Tests mit typischen Geräten (siehe 4.5), Dauertests für Audio.
- Später: Integrationstests mit virtuellen USB-Geräten (`dummy_hcd` + Gadget), Netzwerk-Simulation
  (`tc netem`), Fuzzing des Steuerprotokolls.

## 11. Entscheidungen

| Frage | Entscheidung |
|-------|--------------|
| Technologie | Rust + Tauri 2 |
| Lizenz | GPL-3.0-or-later (passt zu usbipd-win, usbip-win2, Linux-USB/IP) |
| Interoperabilität mit Standard-`usbip` | Ja, für den Datenpfad (optional) |
| Zielgruppe | Privatanwender, Musiker |
| Windows-Treiber | signiertes usbipd-win (dessen VBoxUSB-Treiber); der Installer installiert es mit |
| Sicherheit v0.1.0 | Kopplung mit Code (SPAKE2), Noise-Steuerkanal, Datenpfad nur für gekoppelte Rechner, unverschlüsselt |
| Discovery | eigene UDP-Beacons statt mDNS |
| Mindestversionen | Windows 10/11 x64; Linux x64 mit systemd (getestet: Ubuntu 24.04) |
