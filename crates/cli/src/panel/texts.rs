//! Panel texts, English and German (mostly the app's wording). Umlauts are spelled out when
//! drawn (see `term::ascii`).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    De,
}

impl Lang {
    /// From `--lang`, else the system language.
    pub fn detect(arg: Option<&str>) -> Self {
        let pick = |s: &str| if s.to_ascii_lowercase().starts_with("de") { Lang::De } else { Lang::En };
        if let Some(a) = arg {
            return pick(a);
        }
        for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
            if let Ok(v) = std::env::var(var) {
                if !v.is_empty() && v != "C" && v != "POSIX" {
                    return pick(&v);
                }
            }
        }
        #[cfg(windows)]
        {
            // primary language 0x07: German
            if unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() } & 0x3ff == 0x07 {
                return Lang::De;
            }
        }
        Lang::En
    }

    pub fn other(self) -> Self {
        match self {
            Lang::En => Lang::De,
            Lang::De => Lang::En,
        }
    }
}

/// The text for `key` ("?" if there is none).
pub fn t(lang: Lang, key: &str) -> &'static str {
    match TEXTS.iter().find(|(k, _, _)| *k == key) {
        Some((_, en, de)) => {
            if lang == Lang::De {
                de
            } else {
                en
            }
        }
        None => "?",
    }
}

/// The text for `key` with `{name}` placeholders filled in.
pub fn tv(lang: Lang, key: &str, vars: &[(&str, &str)]) -> String {
    let mut s = t(lang, key).to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

const TEXTS: &[(&str, &str, &str)] = &[
    // panel
    ("treeTitle", "Computers", "Computer"),
    ("detailsTitle", "Details", "Details"),
    ("keysMain", "Up/Down: select  Enter: actions  Tab: details  a: add computer  f: filter  l: language  ?: help  q: quit", "Auf/Ab: wählen  Enter: Aktionen  Tab: Details  a: Computer hinzufügen  f: Filter  l: Sprache  ?: Hilfe  q: Ende"),
    ("keysDetails", "Up/Down: scroll  Tab/Esc: back to the list", "Auf/Ab: blättern  Tab/Esc: zurück zur Liste"),
    ("keysMenu", "Up/Down or 1-9: choose  Enter: do it  Esc: close", "Auf/Ab oder 1-9: wählen  Enter: ausführen  Esc: schließen"),
    ("keysConfirm", "y/Enter: yes  n/Esc: no", "j/Enter: ja  n/Esc: nein"),
    ("keysInput", "Enter: OK  Esc: cancel", "Enter: OK  Esc: abbrechen"),
    ("keysPairIn", "Enter: pair  Esc: decline", "Enter: koppeln  Esc: ablehnen"),
    ("keysClose", "Esc/Enter: close", "Esc/Enter: schließen"),
    ("filterOn", "Filter: only shared and used devices", "Filter: nur freigegebene und benutzte Geräte"),
    ("filterOff", "Filter off: all devices", "Filter aus: alle Geräte"),
    ("tooSmall", "Window too small", "Fenster zu klein"),
    ("actionsTitle", "Actions", "Aktionen"),
    ("actionsHint", "Enter: actions", "Enter: Aktionen"),
    ("done", "{action}: done", "{action}: erledigt"),
    ("failed", "{action} failed: {error}", "{action} hat nicht geklappt: {error}"),
    ("noActions", "Nothing to do here right now", "Hier gibt es gerade nichts zu tun"),
    ("helpTitle", "Help", "Hilfe"),
    ("help", "USB-IP-Bridge lets your computers use each other's USB devices over the network.\n\nLeft: this computer and the other computers with their USB devices. Right: details of the selected item. Enter shows what can be done with it: share a device, give it to another computer, take it back, use a device of another computer here, pair computers, lock devices.\n\nKeys:\n  Up/Down, PgUp/PgDn, Home/End   select\n  Left/Right                     close/open a computer\n  Enter or Space                 actions\n  Tab                            scroll the details\n  a                              add a computer (name or address)\n  f                              only shared and used devices\n  l                              language (English/Deutsch)\n  r                              refresh now\n  q or Ctrl+C                    quit\n\nThe panel shows only plain ASCII characters, so it works in any terminal and over SSH.", "Mit USB-IP-Bridge nutzen deine Computer gegenseitig ihre USB-Geräte über das Netzwerk.\n\nLinks: dieser Computer und die anderen Computer mit ihren USB-Geräten. Rechts: Details zum gewählten Eintrag. Enter zeigt, was sich damit tun lässt: Gerät freigeben, an einen anderen Computer geben, zurückholen, ein Gerät eines anderen Computers hierher holen, Computer koppeln, Geräte sperren.\n\nTasten:\n  Auf/Ab, Bild auf/ab, Pos1/Ende   wählen\n  Links/Rechts                     Computer zu-/aufklappen\n  Enter oder Leertaste             Aktionen\n  Tab                              in den Details blättern\n  a                                Computer hinzufügen (Name oder Adresse)\n  f                                nur freigegebene und benutzte Geräte\n  l                                Sprache (English/Deutsch)\n  r                                sofort aktualisieren\n  q oder Strg+C                    beenden\n\nDas Panel zeigt nur einfache ASCII-Zeichen und funktioniert so in jedem Terminal und über SSH."),
    // like the app
    ("thisComputer", "This computer", "Dieser Computer"),
    ("thisComputerTag", "this computer", "dieser Computer"),
    ("serviceOk", "service running", "Dienst läuft"),
    ("serviceDown", "Service not running", "Dienst läuft nicht"),
    ("serviceDownText", "The USB-IP-Bridge service is not running on this computer – nothing is shared or used.", "Der USB-IP-Bridge-Dienst läuft auf diesem Computer nicht – es wird nichts freigegeben oder eingebunden."),
    ("peerOnline", "reachable", "erreichbar"),
    ("peerOffline", "not reachable", "nicht erreichbar"),
    ("notPaired", "not paired", "nicht gekoppelt"),
    ("chipShared", "shared", "freigegeben"),
    ("chipHere", "here", "hier"),
    ("chipConnecting", "connecting...", "verbinde..."),
    ("chipWaiting", "waiting", "wartet"),
    ("chipAbsent", "not there", "nicht da"),
    ("stateAbsentLocal", "Shared, but not plugged in right now.", "Freigegeben, aber gerade nicht eingesteckt."),
    ("stateAbsent", "{host} does not list it right now (not plugged in, or {host} is not reachable).", "{host} meldet es gerade nicht (nicht eingesteckt, oder {host} ist nicht erreichbar)."),
    ("chipLocked", "locked", "gesperrt"),
    ("chipProblem", "! problem", "! Problem"),
    ("stateLocal", "Only on this computer – not shared.", "Nur auf diesem Computer – nicht freigegeben."),
    ("stateShared", "Shared – used here until another computer takes it.", "Freigegeben – wird hier benutzt, bis ein anderer Computer es holt."),
    ("stateLent", "In use by {host}.", "Wird gerade von {host} benutzt."),
    ("stateBlocked", "Locked: never shared (set in the configuration file).", "Gesperrt: wird nie freigegeben (so in der Konfigurationsdatei festgelegt)."),
    ("stateLockedAuto", "Locked automatically: looks important for using this computer (keyboard, mouse, network, Bluetooth). Never shared unless you unlock it.", "Automatisch gesperrt: sieht für die Bedienung dieses Computers wichtig aus (Tastatur, Maus, Netzwerk, Bluetooth). Wird nie freigegeben, außer du entsperrst es."),
    ("stateLockedUser", "Locked: never shared.", "Gesperrt: wird nie freigegeben."),
    ("statePeerShared", "Shared by {host} – available.", "Von {host} freigegeben – verfügbar."),
    ("statePeerNotShared", "Not shared by {host}.", "Von {host} nicht freigegeben."),
    ("statePeerNever", "{host} never shares it.", "{host} gibt es nie frei."),
    ("statePeerUsed", "In use by {user}.", "Wird gerade von {user} benutzt."),
    ("attached", "In use on this computer", "Wird auf diesem Computer benutzt"),
    ("connecting", "Connecting to {host}...", "Verbinde mit {host}…"),
    ("waiting", "Waiting – {error}", "Wartet – {error}"),
    ("disabled", "Stopped – {error}", "Angehalten – {error}"),
    ("hintNoShare", "Sharing devices is not possible on this system yet.", "Freigeben ist auf diesem System noch nicht möglich."),
    ("hintAskShare", "Share it on {host}, then it can be used here.", "Auf {host} freigeben, dann kann es hier benutzt werden."),
    ("share", "Share", "Freigeben"),
    ("unshare", "Stop sharing", "Freigabe beenden"),
    ("giveTo", "Give to {host}", "An {host} geben"),
    ("takeBack", "Take back", "Zurückholen"),
    ("useHere", "Use here", "Hierher holen"),
    ("giveBack", "Give back", "Zurückgeben"),
    ("lock", "Lock", "Sperren"),
    ("unlock", "Unlock", "Entsperren"),
    ("confirmShare", "Share {device}?\n\n{warnings}\n\nAnother computer can then take it away from this one.", "{device} freigeben?\n\n{warnings}\n\nEin anderer Computer kann es diesem dann wegnehmen."),
    ("pair", "Pair...", "Koppeln…"),
    ("unpair", "Unpair", "Kopplung aufheben"),
    ("removeComputer", "Remove from the list", "Aus der Liste entfernen"),
    ("confirmUnpair", "Unpair {host}? The two computers can no longer use each other's devices until they are paired again.", "Kopplung mit {host} aufheben? Die beiden Computer können dann keine Geräte mehr miteinander nutzen, bis sie neu gekoppelt werden."),
    ("confirmRemove", "Remove {host} from the list?", "{host} aus der Liste entfernen?"),
    ("stateNotPaired", "Not paired – its devices appear once the two computers are paired.", "Nicht gekoppelt – die Geräte erscheinen, sobald die beiden Computer gekoppelt sind."),
    ("stateUnpairedThere", "{host} removed the pairing – please pair again.", "Die Kopplung wurde auf {host} aufgehoben – bitte neu koppeln."),
    ("pairOutTitle", "Pairing with {host}", "Kopplung mit {host}"),
    ("pairOutText", "Type this code on {host}:", "Gib diesen Code auf {host} ein:"),
    ("pairInTitle", "{host} wants to pair with this computer", "{host} möchte sich mit diesem Computer koppeln"),
    ("pairInText", "Only pair computers that belong to you.", "Kopple nur Computer, die dir gehören."),
    ("pairInLabel", "Code from {host}:", "Code von {host}:"),
    ("pairDone", "Paired with {host}.", "Mit {host} gekoppelt."),
    ("pairFailed", "Pairing with {host} did not work", "Die Kopplung mit {host} hat nicht geklappt"),
    ("addComputer", "Add computer", "Computer hinzufügen"),
    ("addHelp", "For computers that are not found automatically, e.g. in another network or over Tailscale. USB-IP-Bridge must run there, too.", "Für Computer, die nicht automatisch gefunden werden, z. B. in einem anderen Netz oder über Tailscale. Dort muss USB-IP-Bridge ebenfalls laufen."),
    ("addLabel", "Name or address:", "Name oder Adresse:"),
    ("blockedTitle", "{client} wants to use {device}, but a program on this computer holds it.", "{client} möchte {device} nutzen, aber ein Programm auf diesem Computer hält es fest."),
    ("blockedKnownPanel", "Known to do this: {programs}. Close it – the other computer then tries again.", "Bekannt dafür: {programs}. Beende es – der andere Computer versucht es dann erneut."),
    ("blockedUnknown", "Close programs that use {device} (e.g. audio software) – the other computer then tries again.", "Schließe Programme, die {device} nutzen (z. B. Audio-Software) – der andere Computer versucht es dann erneut."),
    ("audioRulesTitle", "Audio devices of other computers are not accepted yet: reload audio (local computer, Enter) while no game stream and no audio program runs.", "Audiogeräte anderer Computer werden noch nicht angenommen: Audio neu laden (dieser Computer, Enter), während kein Spiel-Stream und kein Audioprogramm läuft."),
    ("reloadAudio", "Reload audio", "Audio neu laden"),
    ("factAudioRules", "Audio rules", "Audio-Regeln"),
    ("audioActive", "active", "aktiv"),
    ("audioInactive", "not active yet", "noch nicht aktiv"),
    ("sectionSystem", "Computer", "Computer"),
    ("sysOs", "Operating system", "Betriebssystem"),
    ("sysCpu", "Processor", "Prozessor"),
    ("sysGpu", "Graphics", "Grafik"),
    ("sysRam", "Memory", "Arbeitsspeicher"),
    ("threads", "{n} threads", "{n} Threads"),
    ("sectionDetails", "Details", "Details"),
    ("sectionWarnings", "Good to know", "Gut zu wissen"),
    ("sectionConnection", "Connection", "Verbindung"),
    ("sectionProfile", "Device profile", "Geräteprofil"),
    ("factComputer", "Computer", "Computer"),
    ("factKind", "Kind", "Art"),
    ("factIds", "Vendor : product ID", "Hersteller- : Produkt-ID"),
    ("factSerial", "Serial number", "Seriennummer"),
    ("factMaker", "Manufacturer", "Hersteller"),
    ("factProduct", "Product name", "Produktbezeichnung"),
    ("factPort", "Port", "Anschluss"),
    ("factDriver", "Driver", "Treiber"),
    ("factPlatformId", "System device ID", "Geräte-ID im System"),
    ("connPort", "Port here", "Anschluss hier"),
    ("connSince", "Connected", "Verbunden"),
    ("connCount", "Attached", "Eingebunden"),
    ("connError", "Last problem", "Letztes Problem"),
    ("times", "{n}x", "{n}x"),
    ("secsAgo", "{s} s ago", "vor {s} s"),
    ("minsAgo", "{m} min ago", "vor {m} min"),
    ("hostAddress", "Address", "Adresse"),
    ("hostVersion", "Version", "Version"),
    ("hostCan", "Can", "Kann"),
    ("hostHow", "Known from", "Bekannt durch"),
    ("hostSeen", "Last answer", "Letzte Antwort"),
    ("hostDevices", "USB devices", "USB-Geräte"),
    ("hostId", "Host ID", "Host-ID"),
    ("hostPaired", "Paired", "Gekoppelt"),
    ("hostKey", "Key fingerprint", "Schlüssel-Fingerabdruck"),
    ("yes", "yes", "ja"),
    ("no", "no", "nein"),
    ("canExport", "share its devices", "Geräte freigeben"),
    ("canImport", "use devices of others", "Geräte anderer nutzen"),
    ("how_added", "added in the app", "in der App hinzugefügt"),
    ("how_config", "configuration", "Konfiguration"),
    ("how_discovered", "found in the network", "im Netzwerk gefunden"),
    ("profName", "Profile", "Profil"),
    ("profBlocking", "Known to hold it", "Hält es bekanntermaßen fest"),
    ("profReady", "Before a program starts, wait for", "Vor dem Programmstart wird gewartet auf"),
    ("profReconnect", "After a lost connection", "Nach Verbindungsabbruch"),
    ("profLinuxAudio", "Linux audio", "Linux-Audio"),
    ("ready_audio", "sound card", "Soundkarte"),
    ("ready_midi", "MIDI port", "MIDI-Anschluss"),
    ("ready_video", "video device", "Videogerät"),
    ("ready_block", "drive", "Laufwerk"),
    ("ready_tty", "serial port", "serielle Schnittstelle"),
    ("ready_input", "input device", "Eingabegerät"),
    ("ready_driver", "driver", "Treiber"),
    ("reconnect_auto", "reconnect automatically", "automatisch neu verbinden"),
    ("reconnect_ask", "ask first (data may be lost)", "erst nachfragen (Daten könnten verloren sein)"),
    ("laNeverDefault", "never the default device", "nie Standardgerät"),
    ("laProAudio", "pro audio profile", "Pro-Audio-Profil"),
    ("laBuffer", "buffer {h}/{p}", "Puffer {h}/{p}"),
    ("class_audio", "Audio", "Audio"),
    ("class_midi", "MIDI", "MIDI"),
    ("class_video", "Video", "Video"),
    ("class_storage", "Storage", "Speicher"),
    ("class_keyboard", "Keyboard", "Tastatur"),
    ("class_mouse", "Mouse", "Maus"),
    ("class_hid", "Input device", "Eingabegerät"),
    ("class_serial", "Serial port", "Serielle Schnittstelle"),
    ("class_network", "Network", "Netzwerk"),
    ("class_bluetooth", "Bluetooth", "Bluetooth"),
    ("class_printer", "Printer", "Drucker"),
    ("class_image", "Camera/phone", "Kamera/Handy"),
    ("class_smart_card", "Smart card", "Chipkarte"),
    ("class_hub", "Hub", "Hub"),
    ("class_firmware", "Firmware", "Firmware"),
    ("class_vendor_specific", "Special", "Speziell"),
    ("warnText_data_loss", "Data can be lost if the connection breaks while another computer uses it.", "Daten können verloren gehen, wenn die Verbindung abbricht, während es an einem anderen Computer benutzt wird."),
    ("warnText_lockout", "Without this device you may no longer be able to operate this computer.", "Ohne dieses Gerät ist dieser Computer eventuell nicht mehr bedienbar."),
    ("warnText_network_loss", "Without this device this computer may lose its own network connection.", "Ohne dieses Gerät verliert dieser Computer eventuell seine eigene Netzwerkverbindung."),
    ("warnText_high_bandwidth", "Needs a lot of network bandwidth – best over cable.", "Braucht viel Netzwerk-Bandbreite – am besten über Kabel."),
    ("warnText_latency_sensitive", "Sensitive to network delays – best over cable.", "Empfindlich gegenüber Netzwerk-Verzögerungen – am besten über Kabel."),
];
