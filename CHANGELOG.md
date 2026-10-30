# Changelog

All notable changes to this project are documented in this file.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- `usbipb panel`: an interactive panel for the terminal with what the app offers –
  computers and their devices, details, share, give to, take back, use here, give back,
  lock, pair, add computers. Plain ASCII only, so it works in any terminal and over SSH;
  English and German (`--lang de|en`, else the system language).
- Linux can share its devices (kernel driver `usbip-host`): a shared device stays usable
  locally until another computer takes it and gets its normal driver back afterwards;
  mounted file systems are unmounted first. The Linux package opens TCP 3242 in ufw.
  (Not tested between two Linux computers yet.)
- Shared devices keep their name while they are not plugged in: the sharing computer
  remembers them and lists them as "not there" (also to other computers).
- Windows can use devices of other computers, with the usbip-win2 driver installed
  (https://github.com/vadimgrn/usbip-win2): use here, give back, give to, take back;
  the service keeps a device attached across its own restarts. Storage devices are
  ejected the safe way before they are given back. An audio device of another computer
  does not become the default playback or recording device (Windows would make it one).

### Changed
- A device that is wanted here but not offered right now shows "waiting" instead of
  "connecting…".

### Fixed
- A computer no longer lists devices it only uses from another computer as its own.
- The device details said "Windows device ID" on Linux, too.
- After a restart of the service, an attached device showed "attached (?)" instead of its
  bus id; on Linux it could even fail to be taken over once the computer shared devices.
- Windows installer: an update while the app was open could stop with the service
  stopped; the app is now closed first.
- Windows: a lent device that is unplugged ends the session, so the other computer
  waits and takes the device again once it is back (before, it kept a dead device).
- "In use by" showed the router's name for a computer (e.g. a hex number) instead of the
  name of the paired computer.

## [0.1.1] – 2026-10-24

First public release.

### Added
- Details of every computer in the app: operating system, processor, graphics, memory
  (sent by each computer's service).
- Locks: devices that look essential for operating a computer (keyboard, mouse, network
  adapter, Bluetooth – their device profile warns of lockout or network loss) are locked
  automatically and never shared. Any device can be locked or unlocked in the app (right
  click: "Lock" / "Unlock"); remembered by the service. `never_share` in config.toml still
  locks for good.

### Changed
- Clearer code entry when pairing (labelled field, no example digits).
- Linux: no WirePlumber rules on systems without WirePlumber (e.g. PulseAudio only).
- Windows installer: an update stops the running service reliably first.

## [0.1.0] – 2026-10-24

### Added
- **Pairing and encryption.** Every installation has its own key pair. The services talk to
  each other over an encrypted, authenticated channel (Noise protocol, X25519, ChaCha20-Poly1305).
  Computers are paired once: one shows a 6-digit code, the user types it on the other
  (SPAKE2, bound to the connection – someone in the middle gets one guess per attempt).
  Only paired computers see each other's devices, give and take them, and only they may
  attach shared devices over USB/IP (`require_pairing = false` allows any computer, e.g.
  standard `usbip` clients). App: "Pair…", "Remove pairing", code display and code entry,
  key fingerprints in the details. CLI: `usbipb pair`, `pair-answer`, `unpair`.
- **Installers.** Windows: setup program that installs the app, the service (automatic
  start, restart on failure), the firewall rules (private networks) and, if missing,
  usbipd-win for its USB drivers; uninstalling removes all of it again. Linux: .deb package
  with the service (systemd), the virtual USB host controller module, the app and a menu entry.
- First start: a short introduction (also behind "How it works").
- The Windows service log starts anew at 5 MB (the previous one is kept as `service.log.1`).
- THIRD-PARTY.md lists the components USB-IP-Bridge is built from and their licenses.
- GUI: language menu (system language, English, Deutsch); the choice is remembered per user.
- Device classes detected from the USB descriptors (audio, MIDI, video, storage, keyboard,
  mouse, network, ...) and device profiles (built-in TOML plus user files) with warnings,
  readiness checks, reconnect policy and known blocking programs. The XR18 is the first profile.
- The device list sent to clients includes interfaces and classes; clients show kind and warnings.
- `usbipb run [--device D] [--wait S] -- PROGRAM`: attach, wait until the device is usable,
  run the program, give the device back. The MorningDAW launcher uses it.
- GUI: "Kind" column with warning hints; `usbipb list`/`status` show kind and warnings.

- Linux: before giving a storage device back, its file systems are unmounted; while one is
  in use the device stays, and the reason is shown.
- `usbipb descriptors BUSID` prints a device's configuration descriptor (diagnostics).
- Profile for the Elgato Facecam (blocked by Elgato Camera Hub).

- Computers find each other (UDP broadcast on 3244, or `peers` in the configuration) and
  talk over a control channel (TCP 3244, no authentication yet). The GUI shows them with
  their devices.
- Share / stop sharing a device with one click (remembered in `shares.json`), use a shared
  device of another computer here, give a device to another computer and take it back.
- `usbipb share|unshare VID:PID`.
- New GUI: a tree of all computers and their USB devices with a details pane, context menu,
  keyboard navigation and a "only shared and in use" filter; "Add computer" for computers in
  other networks (e.g. Tailscale).
- Computers found in the network are remembered (known.json): after a restart they are listed
  (greyed) and come back on their own; offline ones can be removed from the list. A computer
  known under several addresses (LAN, Tailscale) is one entry and can be removed as a whole.
- Linux: one PipeWire/WirePlumber rule for all audio devices attached over USB/IP (never the
  default device, pro-audio profile, buffering – from the device profile of the audio class),
  written by the service in the format of the installed WirePlumber (0.4 Lua / 0.5 conf).
  An audio device is only attached once the rule is active; if needed the service reloads
  WirePlumber first – but only while no audio is in use (game stream, DAW, playback).
  GUI: notice with "Reload audio" while the rule is not active yet.
- "Give to" also allows the chosen computer where the configuration limits a share to
  certain clients (remembered in shares.json).
- "Give to" offers a menu when several computers can take a device; new computers and devices
  flash briefly in the tree.

### Fixed
- USB 3 (SuperSpeed) devices were announced as high speed and could not be used on Linux.
- Device names padded with spaces are trimmed.

### Changed
- Storage devices are not reattached silently after a lost connection.
- Control protocol version 2 (encrypted); it does not talk to version 0.0.1.
- The Windows service depends on the VBoxUSB monitor driver, so Windows starts it first;
  installing the service again updates it.

### Removed
- Development helpers (sudo rules, install scripts) and the early experiments ("spikes").

## [0.0.1] – 2026-10-21

### Added
- **Windows service** (`usbipb-service`): shares configured USB devices over USB/IP using the
  VBoxUSB drivers installed by usbipd-win. Devices are captured only while a client uses them
  and handed back to Windows afterwards. High-speed isochronous transfers (audio interfaces)
  work in full duplex: the server queries VBoxUSB's high-speed flag and re-frames isochronous
  packets into whole 1 ms frames.
- **Linux service** (systemd): attaches remote devices through the kernel's `vhci_hcd`
  without the `usbip` tool; attaches on request (`mode = "manual"`, default) or always,
  and reconnects automatically after network or server interruptions while wanted.
- **Command line tool** `usbipb`: `list`, `status`, `attach [N]`, `detach [N]`.
- **Desktop app** (Tauri, Windows and Linux): devices of this computer, shared and attached
  devices, "use here"/"give back" buttons, and a warning with a "close and share" button when
  a known program (e.g. a vendor control panel) holds a device another computer asks for; the
  closed program is started again when the device comes back. English and German.
- Configuration in TOML (`export`, `import`, `never_share`, `blocking_processes`), local JSON
  API on 127.0.0.1:3243, logging, install scripts for both platforms.
- Device and host names across computers: the exporter sends them in the USB/IP device
  record's path (`/usb-ip-bridge/serial=…/name=…/host=…`); clients are shown by reverse-DNS
  name; a device lent to another computer keeps its real name in the device list.
- Linux helper scripts for the XR18 + MorningDAW setup: `morningdaw-xr18` launcher.
- Project plan (`PLAN.md`), README, GPL-3.0-or-later license.

### Known limitations
- No security yet (trusted LAN only, client IP allow list).
- Windows can only share, Linux can only attach; no discovery of other hosts yet.
- The GUI language follows the system language (no setting yet).
