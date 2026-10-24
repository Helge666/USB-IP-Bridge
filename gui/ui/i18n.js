// UI texts. Add a language by adding an object with the same keys.
const MESSAGES = {
  en: {
    thisComputer: "This computer",
    thisComputerTag: "this computer",
    treeLabel: "Computers and their USB devices",
    onlyShared: "Only shared and in use",
    secureNote: "Only paired computers can see and use your devices.",
    helpButton: "How it works",
    welcomeTitle: "Welcome to USB-IP-Bridge",
    welcomeIntro: "Your computers can use each other's USB devices over the network – as if they were plugged in directly. For example, the audio interface on one computer can be used for recording on another.",
    welcomeStep1: "Install USB-IP-Bridge on every computer that should share or use devices. They find each other in the local network and appear on the left.",
    welcomeStep2: "Pair the computers once: select the other computer, choose “Pair…” and type the code shown here on the other computer.",
    welcomeShare: "Share a device of this computer: select it and choose “Share” or “Give to …”. It comes back when the other computer gives it back.",
    welcomeUse: "Use a device of another computer: select it and choose “Use here”.",
    welcomeSafety: "Only pair computers that belong to you. Do not give away the keyboard or mouse you need to operate this computer.",
    welcomeOk: "Let's go",
    selectHint: "Select a computer or a device on the left.",
    noActions: "Nothing to do here right now",

    addComputer: "+ Add computer",
    addTitle: "Add a computer",
    addHelp: "For computers that are not found automatically, for example in another network or over Tailscale. Enter its name or address – USB-IP-Bridge has to run there too.",
    addPlaceholder: "e.g. 100.101.102.103 or workshop-pc",
    add: "Add", cancel: "Cancel",
    removeComputer: "Remove from the list",
    confirmRemove: "Remove {host} from the list?",

    serviceOk: "Service running",
    serviceDown: "Service not running",
    serviceDownText: "The USB-IP-Bridge service is not running on this computer, so nothing is shared or attached.",
    peerOnline: "online",
    peerOffline: "not reachable",

    chipShared: "shared", chipHere: "here", chipConnecting: "connecting…",
    stateLocal: "Only used on this computer – not shared.",
    stateShared: "Shared – used here until another computer takes it.",
    stateLent: "In use by {host}.",
    stateBlocked: "Locked: never shared (set in the configuration file).",
    stateLockedAuto: "Locked automatically: it looks essential for operating this computer (keyboard, mouse, network, Bluetooth). Never shared unless you unlock it.",
    stateLockedUser: "Locked: never shared.",
    lock: "Lock", unlock: "Unlock",
    sectionSystem: "Computer", sysOs: "Operating system", sysCpu: "Processor", sysGpu: "Graphics", sysRam: "Memory", threads: "{n} threads",
    statePeerShared: "Shared by {host} – available.",
    statePeerNotShared: "Not shared by {host}.",
    statePeerNever: "{host} never shares it.",
    statePeerUsed: "In use by {user}.",
    offerNever: "never shared",
    attached: "In use on this computer",
    connecting: "Connecting to {host}…",
    waiting: "Waiting – {error}",
    disabled: "Stopped – {error}",
    hintNoShare: "Sharing devices is not possible on this system yet.",
    hintAskShare: "Share it on {host}, then it can be used here.",

    share: "Share", unshare: "Stop sharing",
    giveTo: "Give to {host}", giveToMany: "Give to", takeBack: "Take back",
    useHere: "Use here", giveBack: "Give back",
    confirmShare: "Share {device}?\n\n{warnings}\n\nAnother computer can then take it away from this one.",

    blockedTitle: "{client} wants to use {device}, but a program on this computer is holding it.",
    blockedKnown: "Known to hold it: {programs}. It will be started again when {device} comes back.",
    blockedUnknown: "Close programs that use {device} (for example audio software), then the other computer will try again.",
    closeAndShare: "Close {programs} and share",
    closing: "Closing…",
    restarted: "Started again: {programs}",
    audioRulesTitle: "Audio devices of other computers are not taken yet",
    audioRulesText: "The rules that keep them from becoming the default device (system and game audio stay where they are) are read when audio restarts. Reload it while no game stream or audio program runs.",
    reloadAudio: "Reload audio",
    audioBusy: "Audio is in use right now ({programs}). Reload it later, when no game stream or audio program runs.",
    factAudioRules: "Audio rules", audioActive: "active", audioInactive: "not active yet",

    sectionDetails: "Details", sectionWarnings: "Good to know", sectionConnection: "Connection", sectionProfile: "Device profile",
    factComputer: "Computer", factKind: "Kind", factIds: "Vendor : product ID", factSerial: "Serial number",
    factMaker: "Manufacturer", factProduct: "Product name", factPort: "Port", factDriver: "Driver", factPlatformId: "Windows device ID",
    connPort: "Port here", connSince: "Connected", connCount: "Attached", connError: "Last problem", times: "{n}×",
    secsAgo: "{s} s ago", minsAgo: "{m} min ago",
    hostAddress: "Address", hostVersion: "Version", hostCan: "Can", hostHow: "Known through", hostSeen: "Last answer",
    hostDevices: "USB devices", hostId: "Host ID",
    hostPaired: "Paired", hostKey: "Key fingerprint", yes: "yes", no: "no",

    pair: "Pair…", unpair: "Remove pairing", notPaired: "not paired",
    confirmUnpair: "Remove the pairing with {host}? The two computers can then no longer use each other's devices until they are paired again.",
    stateNotPaired: "Not paired – its devices appear once the two computers are paired.",
    stateUnpairedThere: "The pairing was removed on {host} – pair again.",
    pairOutTitle: "Pairing with {host}",
    pairOutText: "Type this code on {host}:",
    pairInTitle: "{host} wants to pair with this computer",
    pairInText: "Only pair computers that belong to you.",
    pairInLabel: "Code from {host}:",
    pairOk: "Pair", pairDecline: "Decline", pairDismiss: "OK",
    pairDone: "Paired with {host}.",
    pairFailed: "Pairing with {host} did not work",
    canExport: "share its devices", canImport: "use devices of others",
    how_added: "added in the app", how_config: "configuration", how_discovered: "found in the network",
    profName: "Profile", profBlocking: "Known to hold it", profReady: "Before starting a program, waits for",
    profReconnect: "After a broken connection", profLinuxAudio: "Linux audio",
    ready_audio: "sound card", ready_midi: "MIDI port", ready_video: "video device", ready_block: "drive",
    ready_tty: "serial port", ready_input: "input device", ready_driver: "driver",
    reconnect_auto: "reconnect automatically", reconnect_ask: "ask first (data may have been lost)",
    laNeverDefault: "never the default device", laProAudio: "pro-audio profile", laBuffer: "buffer {h}/{p}",

    class_audio: "Audio", class_midi: "MIDI", class_video: "Video", class_storage: "Storage",
    class_keyboard: "Keyboard", class_mouse: "Mouse", class_hid: "Input device", class_serial: "Serial port",
    class_network: "Network", class_bluetooth: "Bluetooth", class_printer: "Printer", class_image: "Camera/phone",
    class_smart_card: "Smart card", class_hub: "Hub", class_firmware: "Firmware", class_vendor_specific: "Special",
    warnText_data_loss: "Data can be lost if the connection breaks while it is in use on another computer.",
    warnText_lockout: "Without it, this computer may no longer be operable.",
    warnText_network_loss: "This computer may lose its own network connection without it.",
    warnText_high_bandwidth: "Needs a lot of network bandwidth – best over a wired connection.",
    warnText_latency_sensitive: "Sensitive to network delays – best over a wired connection.",

    language: "Language",
    langAuto: "System language",
  },
  de: {
    thisComputer: "Dieser Computer",
    thisComputerTag: "dieser Computer",
    treeLabel: "Computer und ihre USB-Geräte",
    onlyShared: "Nur freigegebene und benutzte",
    secureNote: "Nur gekoppelte Computer sehen und nutzen deine Geräte.",
    helpButton: "So geht's",
    welcomeTitle: "Willkommen bei USB-IP-Bridge",
    welcomeIntro: "Deine Computer können gegenseitig ihre USB-Geräte über das Netzwerk nutzen – als wären sie direkt eingesteckt. Zum Beispiel lässt sich das Audio-Interface des einen Computers zum Aufnehmen am anderen verwenden.",
    welcomeStep1: "Installiere USB-IP-Bridge auf jedem Computer, der Geräte teilen oder nutzen soll. Die Computer finden sich im lokalen Netzwerk und erscheinen links.",
    welcomeStep2: "Kopple die Computer einmalig: Wähle den anderen Computer, dann „Koppeln…“, und gib den Code, der hier erscheint, auf dem anderen Computer ein.",
    welcomeShare: "Ein Gerät dieses Computers teilen: Gerät wählen, dann „Freigeben“ oder „Geben an …“. Es kommt zurück, sobald der andere Computer es zurückgibt.",
    welcomeUse: "Ein Gerät eines anderen Computers nutzen: Gerät wählen, dann „Hierher holen“.",
    welcomeSafety: "Kopple nur Computer, die dir gehören. Gib keine Tastatur oder Maus weg, die du zum Bedienen dieses Computers brauchst.",
    welcomeOk: "Los geht's",
    selectHint: "Wähle links einen Computer oder ein Gerät.",
    noActions: "Hier gibt es gerade nichts zu tun",

    addComputer: "+ Computer hinzufügen",
    addTitle: "Computer hinzufügen",
    addHelp: "Für Computer, die nicht automatisch gefunden werden, z. B. in einem anderen Netz oder über Tailscale. Gib den Namen oder die Adresse ein – dort muss USB-IP-Bridge ebenfalls laufen.",
    addPlaceholder: "z. B. 100.101.102.103 oder werkstatt-pc",
    add: "Hinzufügen", cancel: "Abbrechen",
    removeComputer: "Aus der Liste entfernen",
    confirmRemove: "{host} aus der Liste entfernen?",

    serviceOk: "Dienst läuft",
    serviceDown: "Dienst läuft nicht",
    serviceDownText: "Der USB-IP-Bridge-Dienst läuft auf diesem Computer nicht – es wird nichts freigegeben oder eingebunden.",
    peerOnline: "erreichbar",
    peerOffline: "nicht erreichbar",

    chipShared: "freigegeben", chipHere: "hier", chipConnecting: "verbinde…",
    stateLocal: "Nur auf diesem Computer – nicht freigegeben.",
    stateShared: "Freigegeben – wird hier benutzt, bis ein anderer Computer es holt.",
    stateLent: "Wird gerade von {host} benutzt.",
    stateBlocked: "Gesperrt: wird nie freigegeben (so in der Konfigurationsdatei festgelegt).",
    stateLockedAuto: "Automatisch gesperrt: sieht für die Bedienung dieses Computers wichtig aus (Tastatur, Maus, Netzwerk, Bluetooth). Wird nie freigegeben, außer du entsperrst es.",
    stateLockedUser: "Gesperrt: wird nie freigegeben.",
    lock: "Sperren", unlock: "Entsperren",
    sectionSystem: "Computer", sysOs: "Betriebssystem", sysCpu: "Prozessor", sysGpu: "Grafik", sysRam: "Arbeitsspeicher", threads: "{n} Threads",
    statePeerShared: "Von {host} freigegeben – verfügbar.",
    statePeerNotShared: "Von {host} nicht freigegeben.",
    statePeerNever: "{host} gibt es nie frei.",
    statePeerUsed: "Wird gerade von {user} benutzt.",
    offerNever: "nie freigegeben",
    attached: "Wird auf diesem Computer benutzt",
    connecting: "Verbinde mit {host}…",
    waiting: "Wartet – {error}",
    disabled: "Angehalten – {error}",
    hintNoShare: "Freigeben ist auf diesem System noch nicht möglich.",
    hintAskShare: "Auf {host} freigeben, dann kann es hier benutzt werden.",

    share: "Freigeben", unshare: "Freigabe beenden",
    giveTo: "An {host} geben", giveToMany: "Geben an", takeBack: "Zurückholen",
    useHere: "Hierher holen", giveBack: "Zurückgeben",
    confirmShare: "{device} freigeben?\n\n{warnings}\n\nEin anderer Computer kann es diesem dann wegnehmen.",

    blockedTitle: "{client} möchte {device} nutzen, aber ein Programm auf diesem Computer hält es fest.",
    blockedKnown: "Bekannt dafür: {programs}. Es wird wieder gestartet, sobald {device} zurück ist.",
    blockedUnknown: "Schließe Programme, die {device} nutzen (z. B. Audio-Software) – der andere Computer versucht es dann erneut.",
    closeAndShare: "{programs} beenden und freigeben",
    closing: "Wird beendet…",
    restarted: "Wieder gestartet: {programs}",
    audioRulesTitle: "Audiogeräte anderer Computer werden noch nicht angenommen",
    audioRulesText: "Die Regeln, die verhindern, dass sie zum Standardgerät werden (System- und Spielton bleiben, wo sie sind), werden beim Neustart von Audio gelesen. Lade Audio neu, während kein Spiel-Stream und kein Audioprogramm läuft.",
    reloadAudio: "Audio neu laden",
    audioBusy: "Audio wird gerade benutzt ({programs}). Bitte später neu laden, wenn kein Spiel-Stream und kein Audioprogramm läuft.",
    factAudioRules: "Audio-Regeln", audioActive: "aktiv", audioInactive: "noch nicht aktiv",

    sectionDetails: "Details", sectionWarnings: "Gut zu wissen", sectionConnection: "Verbindung", sectionProfile: "Geräteprofil",
    factComputer: "Computer", factKind: "Art", factIds: "Hersteller- : Produkt-ID", factSerial: "Seriennummer",
    factMaker: "Hersteller", factProduct: "Produktbezeichnung", factPort: "Anschluss", factDriver: "Treiber", factPlatformId: "Windows-Geräte-ID",
    connPort: "Anschluss hier", connSince: "Verbunden", connCount: "Eingebunden", connError: "Letztes Problem", times: "{n}×",
    secsAgo: "vor {s} s", minsAgo: "vor {m} min",
    hostAddress: "Adresse", hostVersion: "Version", hostCan: "Kann", hostHow: "Bekannt durch", hostSeen: "Letzte Antwort",
    hostDevices: "USB-Geräte", hostId: "Host-ID",
    hostPaired: "Gekoppelt", hostKey: "Schlüssel-Fingerabdruck", yes: "ja", no: "nein",

    pair: "Koppeln…", unpair: "Kopplung aufheben", notPaired: "nicht gekoppelt",
    confirmUnpair: "Kopplung mit {host} aufheben? Die beiden Computer können dann keine Geräte mehr miteinander nutzen, bis sie neu gekoppelt werden.",
    stateNotPaired: "Nicht gekoppelt – die Geräte erscheinen, sobald die beiden Computer gekoppelt sind.",
    stateUnpairedThere: "Die Kopplung wurde auf {host} aufgehoben – bitte neu koppeln.",
    pairOutTitle: "Kopplung mit {host}",
    pairOutText: "Gib diesen Code auf {host} ein:",
    pairInTitle: "{host} möchte sich mit diesem Computer koppeln",
    pairInText: "Kopple nur Computer, die dir gehören.",
    pairInLabel: "Code von {host}:",
    pairOk: "Koppeln", pairDecline: "Ablehnen", pairDismiss: "OK",
    pairDone: "Mit {host} gekoppelt.",
    pairFailed: "Die Kopplung mit {host} hat nicht geklappt",
    canExport: "Geräte freigeben", canImport: "Geräte anderer nutzen",
    how_added: "in der App hinzugefügt", how_config: "Konfiguration", how_discovered: "im Netzwerk gefunden",
    profName: "Profil", profBlocking: "Hält es bekanntermaßen fest", profReady: "Vor dem Programmstart wird gewartet auf",
    profReconnect: "Nach Verbindungsabbruch", profLinuxAudio: "Linux-Audio",
    ready_audio: "Soundkarte", ready_midi: "MIDI-Anschluss", ready_video: "Videogerät", ready_block: "Laufwerk",
    ready_tty: "serielle Schnittstelle", ready_input: "Eingabegerät", ready_driver: "Treiber",
    reconnect_auto: "automatisch neu verbinden", reconnect_ask: "erst nachfragen (Daten könnten verloren sein)",
    laNeverDefault: "nie Standardgerät", laProAudio: "Pro-Audio-Profil", laBuffer: "Puffer {h}/{p}",

    class_audio: "Audio", class_midi: "MIDI", class_video: "Video", class_storage: "Speicher",
    class_keyboard: "Tastatur", class_mouse: "Maus", class_hid: "Eingabegerät", class_serial: "Serielle Schnittstelle",
    class_network: "Netzwerk", class_bluetooth: "Bluetooth", class_printer: "Drucker", class_image: "Kamera/Handy",
    class_smart_card: "Chipkarte", class_hub: "Hub", class_firmware: "Firmware", class_vendor_specific: "Speziell",
    warnText_data_loss: "Daten können verloren gehen, wenn die Verbindung abbricht, während es an einem anderen Computer benutzt wird.",
    warnText_lockout: "Ohne dieses Gerät ist dieser Computer eventuell nicht mehr bedienbar.",
    warnText_network_loss: "Ohne dieses Gerät verliert dieser Computer eventuell seine eigene Netzwerkverbindung.",
    warnText_high_bandwidth: "Braucht viel Netzwerk-Bandbreite – am besten über Kabel.",
    warnText_latency_sensitive: "Empfindlich gegenüber Netzwerk-Verzögerungen – am besten über Kabel.",

    language: "Sprache",
    langAuto: "Systemsprache",
  },
};

// Each language's own name, shown in the language menu.
const LANGUAGE_NAMES = { en: "English", de: "Deutsch" };

// Language setting: "auto" (follow the system) or a key of MESSAGES; kept per user.
const LANG_KEY = "usbipb.language";

function systemLanguage() {
  const sys = (navigator.language || "en").toLowerCase().split("-")[0];
  return MESSAGES[sys] ? sys : "en";
}

function storedLanguage() {
  try {
    const v = localStorage.getItem(LANG_KEY);
    return v === "auto" || MESSAGES[v] ? v : "auto";
  } catch {
    return "auto";
  }
}

let LANG_SETTING = storedLanguage();
let LANG = LANG_SETTING === "auto" ? systemLanguage() : LANG_SETTING;

function setLanguage(setting) {
  LANG_SETTING = setting;
  LANG = setting === "auto" ? systemLanguage() : setting;
  try {
    localStorage.setItem(LANG_KEY, setting);
  } catch {
    // storage unavailable: the choice lasts until the app closes
  }
  applyStaticTexts();
}

function t(key, vars = {}) {
  const text = (MESSAGES[LANG] && MESSAGES[LANG][key]) || MESSAGES.en[key] || key;
  return text.replace(/\{(\w+)\}/g, (_, k) => (vars[k] !== undefined ? vars[k] : `{${k}}`));
}

function applyStaticTexts() {
  document.documentElement.lang = LANG;
  document.querySelectorAll("[data-i18n]").forEach((el) => { el.textContent = t(el.dataset.i18n); });
  const select = document.getElementById("language");
  if (select) {
    select.setAttribute("aria-label", t("language"));
    select.title = t("language");
    select.replaceChildren(
      new Option(`${t("langAuto")} (${LANGUAGE_NAMES[systemLanguage()]})`, "auto"),
      ...Object.keys(MESSAGES).map((k) => new Option(LANGUAGE_NAMES[k] || k, k)),
    );
    select.value = LANG_SETTING;
  }
}
