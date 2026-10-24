// USB-IP-Bridge UI: a tree of computers and their USB devices (left) and the details of the
// selected item (right). Polls the app backend every 2 s.
const invoke = window.__TAURI__.core.invoke;

let canShare = false; // this computer can share its devices (Windows)
let canImport = false; // this computer can use devices of others (Linux)
let status = null; // service status (null: service not reachable)
let statusError = null;
let localDevices = [];
let closing = false;
// After we closed blocking programs: restart them once the device was used remotely and is back.
let sawRemoteUseAfterClose = false;

// ------------------------------------------------------------------ helpers

const el = (tag, attrs = {}, ...children) => {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === "class") e.className = v;
    else if (k.startsWith("on")) e.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) e.setAttribute(k, v === true ? "" : v);
  }
  for (const c of children.flat()) if (c !== null && c !== undefined && c !== "") e.append(c);
  return e;
};
const hex = (n) => n.toString(16).padStart(4, "0");
const deviceName = (d) => (d ? (d.manufacturer && d.product && !d.product.startsWith(d.manufacturer) ? `${d.manufacturer} ${d.product}` : d.product || `${hex(d.vendor_id)}:${hex(d.product_id)}`) : "");
const ipOf = (address) => address.replace(/:\d+$/, "").replace(/^\[|\]$/g, "");
const sameSerial = (a, b) => !a || !b || a.toLowerCase() === b.toLowerCase();
const matchArgs = (d) => ({ vendorId: d.vendor_id, productId: d.product_id, serial: d.serial || null });

// Small per-user settings (tree state); the UI works without them.
const stored = (key, fallback) => { try { const v = localStorage.getItem(key); return v === null ? fallback : JSON.parse(v); } catch { return fallback; } };
const store = (key, value) => { try { localStorage.setItem(key, JSON.stringify(value)); } catch { /* not kept */ } };

let selectedKey = stored("usbipb.selected", "host:local");
const expanded = new Set(stored("usbipb.expanded", ["host:local"]));
let onlyShared = stored("usbipb.onlyShared", false);

const ICONS = {
  audio: "🎛️", midi: "🎹", video: "📷", storage: "💾", keyboard: "⌨️", mouse: "🖱️", hid: "🕹️",
  serial: "🔌", network: "🌐", bluetooth: "📶", printer: "🖨️", image: "📸", smart_card: "💳",
  hub: "🔀", firmware: "⚙️", vendor_specific: "🔧",
};
const iconOf = (classes) => ICONS[(classes || [])[0]] || "🔌";

const RISKY = ["data_loss", "lockout", "network_loss"];
const kindText = (classes) => (classes || []).map((c) => t("class_" + c)).join(" + ");

// Device profiles of other computers' devices, fetched on demand.
const profiles = new Map();
function profileOf(d) {
  const key = `${d.vendor_id}:${d.product_id}:${(d.classes || []).join(",")}`;
  if (!profiles.has(key)) {
    profiles.set(key, null);
    invoke("profile", { vendorId: d.vendor_id, productId: d.product_id, classes: d.classes || [] })
      .then((p) => { profiles.set(key, p); renderDetails(true); })
      .catch(() => {});
  }
  return profiles.get(key);
}

// ------------------------------------------------------------------ model

// Hosts with their devices, as shown in the tree.
function buildModel() {
  const hosts = [];
  const exports = status ? status.exports : [];
  const lent = exports.filter((e) => e.client && e.device);
  const peers = status ? status.peers || [] : [];
  const importers = peers.filter((p) => p.online && p.hello && p.hello.can_import && p.paired && p.hello.paired);

  // this computer
  const local = { key: "host:local", local: true, name: status ? status.host : t("thisComputer"), online: !!status, devices: [] };
  for (const row of localDevices) {
    // While a device is used by another computer, Windows lists the capture driver
    // (VirtualBox USB, 80ee:cafe) in its place: show the real device.
    const vbox = row.vendor_id === 0x80ee && row.product_id === 0xcafe;
    const lentTo = vbox ? lent.find((e) => e.device.bus_id === row.bus_id) : null;
    if (vbox && !lentTo) continue;
    if (row.state === "attached") continue; // a device of another computer, shown there
    const device = lentTo ? lentTo.device : row;
    const d = {
      kind: "device", local: true, host: local, device,
      key: `dev:local:${hex(device.vendor_id)}:${hex(device.product_id)}:${device.serial || device.bus_id}`,
      name: lentTo ? deviceName(device) : row.name,
      classes: lentTo ? device.classes : row.kind,
      warnings: lentTo ? [] : row.warnings,
      profile: lentTo ? null : row.profile,
      state: lentTo ? "lent" : row.blocked ? "blocked" : row.shared ? "shared" : "local",
      lock: lentTo ? null : row.lock,
      usedBy: lentTo ? lentTo.client_name || lentTo.client : null,
      problem: exports.find((e) => e.problem && e.device && e.device.bus_id === device.bus_id) || null,
      importers,
    };
    if (lentTo) d.warnings = (profileOf(d) || {}).warnings || [];
    local.devices.push(d);
  }
  hosts.push(local);

  // other computers
  for (const p of peers) {
    const name = p.hello ? p.hello.host_name : p.known_name || ipOf(p.address);
    const id = p.hello ? p.hello.host_id : p.known_id || p.address;
    // paired on both sides (only then they talk about devices)
    const paired = !!(p.paired && p.hello && p.hello.paired);
    const host = { key: `host:${id}`, local: false, name, online: p.online, paired, peer: p, devices: [] };
    const ip = ipOf(p.address);
    const importsHere = (status.imports || []).filter((i) => i.device && ipOf(i.remote) === ip);
    const importFor = (dev) => importsHere.find((i) => i.device.vendor_id === dev.vendor_id && i.device.product_id === dev.product_id && sameSerial(i.device.serial, dev.serial));
    for (const pd of p.devices) {
      const imp = importFor(pd.device) || null;
      const here = imp && imp.wanted;
      host.devices.push({
        kind: "device", local: false, host, device: pd.device, imp,
        key: `dev:${host.key}:${hex(pd.device.vendor_id)}:${hex(pd.device.product_id)}:${pd.device.serial || pd.device.bus_id}`,
        name: pd.name, classes: pd.classes, warnings: pd.warnings, offer: pd.offer,
        state: here ? "here" : pd.used_by ? "used" : pd.offer,
        usedBy: pd.used_by,
      });
    }
    // devices used here that the computer does not list (it is offline, or the device is gone)
    for (const imp of importsHere) {
      if (host.devices.some((d) => d.imp === imp)) continue;
      const device = { vendor_id: imp.device.vendor_id, product_id: imp.device.product_id, serial: imp.device.serial };
      host.devices.push({
        kind: "device", local: false, host, device, imp,
        key: `dev:${host.key}:${hex(device.vendor_id)}:${hex(device.product_id)}:${device.serial || "-"}`,
        name: imp.device_name || `${hex(device.vendor_id)}:${hex(device.product_id)}`,
        classes: imp.classes, warnings: imp.warnings, offer: "shared",
        state: imp.wanted ? "here" : "shared",
      });
    }
    hosts.push(host);
  }
  for (const h of hosts) h.devices.sort((a, b) => a.name.localeCompare(b.name));
  return [hosts[0], ...hosts.slice(1).sort((a, b) => a.name.localeCompare(b.name))];
}

const ACTIVE_STATES = ["shared", "lent", "here", "used"];
const visibleDevices = (h) => (onlyShared ? h.devices.filter((d) => ACTIVE_STATES.includes(d.state) || d.problem) : h.devices);

// ------------------------------------------------------------------ actions

// What can be done with an item: [{ label, run, confirm, secondary }].
function actionsOf(item) {
  if (!item) return [];
  if (item.kind === "host") {
    const p = item.peer;
    if (!p) return [];
    const out = [];
    const pairing = status && (status.pairings || []).some((x) => x.outgoing && x.state === "waiting" && x.address === p.address);
    if (p.online && p.hello && !item.paired && !pairing) out.push({ label: t("pair"), run: () => invoke("pair", { address: p.address }) });
    const hostId = p.hello ? p.hello.host_id : p.known_id;
    if (p.paired && hostId) out.push({ label: t("unpair"), secondary: true, confirm: t("confirmUnpair", { host: item.name }), run: () => invoke("unpair", { hostId }) });
    if (p.removable) out.push({ label: t("removeComputer"), secondary: true, confirm: t("confirmRemove", { host: item.name }), run: () => invoke("remove_peer", { address: p.address }) });
    return out;
  }
  const d = item;
  if (d.local) {
    if (d.state === "lent") return canShare ? [{ label: t("takeBack"), run: () => invoke("take_back", matchArgs(d.device)) }] : [];
    const setLock = (lock) => () => invoke("set_locked", { ...matchArgs(d.device), lock });
    // locked in config.toml: only there
    if (d.state === "blocked") return d.lock === "config" ? [] : [{ label: "🔓 " + t("unlock"), secondary: true, run: setLock(false) }];
    const lockAction = { label: "🔒 " + t("lock"), secondary: true, run: setLock(true) };
    if (!canShare) return [lockAction];
    const out = [];
    for (const p of d.importers) {
      out.push({
        group: "give", target: p.hello.host_name,
        label: t("giveTo", { host: p.hello.host_name }),
        confirm: d.state === "shared" ? null : riskyConfirm(d),
        run: () => invoke("give", { peer: p.address, ...matchArgs(d.device) }),
      });
    }
    if (d.state === "shared") out.push({ label: t("unshare"), secondary: true, run: () => invoke("set_shared", { ...matchArgs(d.device), share: false }) });
    else out.push({ label: t("share"), confirm: riskyConfirm(d), run: () => invoke("set_shared", { ...matchArgs(d.device), share: true }) });
    out.push(lockAction);
    return out;
  }
  const p = d.host.peer;
  if (!canImport || !p.hello || !p.hello.can_export) return [];
  const args = { host: ipOf(p.address), port: p.hello.usbip_port, ...matchArgs(d.device) };
  if (d.state === "here") return [{ label: t("giveBack"), secondary: true, run: () => invoke("use_device", { ...args, attach: false }) }];
  if (d.state === "shared" && d.host.online) return [{ label: t("useHere"), run: () => invoke("use_device", { ...args, attach: true }) }];
  return [];
}

// Confirmation for giving away a device with risky warnings, or null.
function riskyConfirm(d) {
  const risky = (d.warnings || []).filter((w) => RISKY.includes(w));
  if (!risky.length) return null;
  return t("confirmShare", { device: d.name, warnings: risky.map((w) => "⚠ " + t("warnText_" + w)).join("\n") });
}

async function runAction(a) {
  if (a.confirm && !confirm(a.confirm)) return;
  try { await a.run(); } catch (e) { alert(String(e)); }
  refresh();
}

// ------------------------------------------------------------------ tree

let model = [];
let flat = []; // visible items in tree order (for keyboard navigation)

function chipOf(d) {
  if (d.problem) return el("span", { class: "chip warn" }, "⚠");
  switch (d.state) {
    case "shared": return el("span", { class: "chip shared" }, t("chipShared"));
    case "lent": return el("span", { class: "chip busy" }, `→ ${d.usedBy}`);
    case "used": return el("span", { class: "chip busy" }, `→ ${d.usedBy}`);
    case "here":
      if (d.imp && d.imp.state !== "attached") return el("span", { class: "chip warn" }, t("chipConnecting"));
      return el("span", { class: "chip busy" }, t("chipHere"));
    case "blocked": case "never": return el("span", { class: "chip lock", title: t("offerNever") }, "🔒");
    default: return null;
  }
}

// Computers and devices that appeared since the last look flash for a moment.
const seenKeys = new Set();
const freshUntil = new Map();
let firstTree = true;
const FRESH_MS = 4000;
const isFresh = (key) => (freshUntil.get(key) || 0) > Date.now();

function noteNewItems() {
  const now = Date.now();
  const all = model.flatMap((h) => [{ kind: "host", ...h }, ...h.devices]);
  for (const i of all) {
    if (!seenKeys.has(i.key)) {
      seenKeys.add(i.key);
      if (!firstTree) freshUntil.set(i.key, now + FRESH_MS);
    }
  }
  // a computer that comes back online counts as new, too
  for (const i of all) {
    if (i.kind === "host" && !i.local) {
      const k = "online:" + i.key;
      if (i.online && !seenKeys.has(k)) { seenKeys.add(k); if (!firstTree) freshUntil.set(i.key, now + FRESH_MS); }
      if (!i.online) seenKeys.delete(k);
    }
  }
  firstTree = false;
}

let treeSignature = "";
function renderTree() {
  model = buildModel();
  flat = [];
  const items = model.map((h) => {
    const item = { kind: "host", ...h };
    flat.push(item);
    const open = expanded.has(h.key);
    const devices = visibleDevices(h);
    const meta = h.local ? t("thisComputerTag") : !h.online ? t("peerOffline") : h.paired ? "" : t("notPaired");
    const children = open ? devices.map((d) => {
      flat.push(d);
      const dim = ["not_shared", "never", "blocked"].includes(d.state);
      return el("li", { role: "treeitem", id: "ti-" + flat.length, "data-key": d.key, "aria-selected": d.key === selectedKey,
        class: `device${d.key === selectedKey ? " selected" : ""}${dim ? " dim" : ""}${isFresh(d.key) ? " fresh" : ""}` },
        el("div", { class: "row" },
          el("span", { class: "twisty" }),
          el("span", { class: "icon" }, iconOf(d.classes)),
          el("span", { class: "label" }, d.name),
          chipOf(d)));
    }) : [];
    return el("li", { role: "treeitem", id: "ti-" + flat.length, "data-key": h.key, "aria-expanded": open, "aria-selected": h.key === selectedKey,
      class: `host${h.key === selectedKey ? " selected" : ""}${h.online ? "" : " offline"}${isFresh(h.key) ? " fresh" : ""}` },
      el("div", { class: "row" },
        el("span", { class: "twisty", "data-toggle": "1" }, open ? "▾" : "▸"),
        el("span", { class: "icon" }, "🖥️"),
        el("span", { class: "label" }, h.name),
        el("span", { class: "meta" }, meta),
        el("span", { class: "chip lock" }, el("span", { class: "dot " + (h.online ? "ok" : "") }))),
      open ? el("ul", { role: "group" }, ...children) : null);
  });
  // Rebuild only when something visible changed (keeps hover and clicks steady).
  noteNewItems();
  const sig = JSON.stringify([LANG, selectedKey, [...expanded], onlyShared, flat.map((i) => [i.key, i.name, i.state, i.online, i.paired, i.usedBy, !!i.problem, i.imp && i.imp.state, isFresh(i.key)])]);
  if (sig === treeSignature) return;
  treeSignature = sig;
  const tree = document.getElementById("tree");
  tree.replaceChildren(...items);
  tree.setAttribute("aria-label", t("treeLabel"));
  const sel = tree.querySelector("li.selected");
  if (sel) tree.setAttribute("aria-activedescendant", sel.id);
}

const itemByKey = (key) => flat.find((i) => i.key === key) || null;

function select(key) {
  if (!key || key === selectedKey) return;
  selectedKey = key;
  store("usbipb.selected", key);
  renderTree();
  renderDetails();
  const row = document.querySelector(`#tree li[data-key="${CSS.escape(key)}"] > .row`);
  if (row) row.scrollIntoView({ block: "nearest" });
}

function toggle(key, open) {
  const isOpen = expanded.has(key);
  if (open === undefined) open = !isOpen;
  if (open === isOpen) return;
  if (open) expanded.add(key); else expanded.delete(key);
  store("usbipb.expanded", [...expanded]);
  renderTree();
}

function setupTree() {
  const tree = document.getElementById("tree");
  tree.addEventListener("click", (ev) => {
    const li = ev.target.closest("li[data-key]");
    if (!li) return;
    tree.focus();
    if (ev.target.closest("[data-toggle]")) toggle(li.dataset.key);
    select(li.dataset.key);
  });
  tree.addEventListener("dblclick", (ev) => {
    const li = ev.target.closest("li.host");
    if (li && !ev.target.closest("[data-toggle]")) toggle(li.dataset.key);
  });
  tree.addEventListener("contextmenu", (ev) => {
    ev.preventDefault();
    const li = ev.target.closest("li[data-key]");
    if (!li) return;
    tree.focus();
    select(li.dataset.key);
    openMenu(itemByKey(li.dataset.key), ev.clientX, ev.clientY);
  });
  tree.addEventListener("keydown", (ev) => {
    const i = flat.findIndex((x) => x.key === selectedKey);
    const item = flat[i];
    const move = (n) => { const next = flat[Math.max(0, Math.min(flat.length - 1, (i < 0 ? 0 : i) + n))]; if (next) select(next.key); };
    switch (ev.key) {
      case "ArrowDown": move(1); break;
      case "ArrowUp": move(-1); break;
      case "Home": if (flat[0]) select(flat[0].key); break;
      case "End": if (flat.length) select(flat[flat.length - 1].key); break;
      case "ArrowRight":
        if (item && item.kind === "host") { if (!expanded.has(item.key)) toggle(item.key, true); else move(1); }
        break;
      case "ArrowLeft":
        if (item && item.kind === "host") toggle(item.key, false);
        else if (item) select(item.host.key);
        break;
      case "Enter": case " ":
        if (item && item.kind === "host") toggle(item.key);
        break;
      case "ContextMenu":
        openMenuAtSelection(); break;
      case "F10":
        if (ev.shiftKey) openMenuAtSelection(); else return;
        break;
      default: return;
    }
    ev.preventDefault();
  });
  const filter = document.getElementById("onlyShared");
  filter.checked = onlyShared;
  filter.addEventListener("change", () => { onlyShared = filter.checked; store("usbipb.onlyShared", onlyShared); renderTree(); });
}

// ------------------------------------------------------------------ context menu

function openMenuAtSelection() {
  const row = document.querySelector("#tree li.selected > .row");
  if (!row) return;
  const r = row.getBoundingClientRect();
  openMenu(itemByKey(selectedKey), r.left + 40, r.bottom);
}

function openMenu(item, x, y) {
  openMenuWith(actionsOf(item), x, y);
}

function openMenuWith(actions, x, y) {
  const menu = document.getElementById("menu");
  menu.replaceChildren(...(actions.length
    ? actions.map((a) => el("button", { role: "menuitem", onclick: () => { closeMenu(); runAction(a); } }, a.label))
    : [el("div", { class: "none" }, t("noActions"))]));
  menu.hidden = false;
  const w = menu.offsetWidth, h = menu.offsetHeight;
  menu.style.left = Math.min(x, window.innerWidth - w - 8) + "px";
  menu.style.top = Math.min(y, window.innerHeight - h - 8) + "px";
  const first = menu.querySelector("button");
  if (first) first.focus();
}

function closeMenu(refocus) {
  const menu = document.getElementById("menu");
  if (menu.hidden) return;
  menu.hidden = true;
  if (refocus) document.getElementById("tree").focus();
}

function setupMenu() {
  const menu = document.getElementById("menu");
  document.addEventListener("mousedown", (ev) => { if (!menu.contains(ev.target)) closeMenu(); });
  window.addEventListener("blur", () => closeMenu());
  menu.addEventListener("keydown", (ev) => {
    const buttons = [...menu.querySelectorAll("button")];
    const i = buttons.indexOf(document.activeElement);
    if (ev.key === "Escape") closeMenu(true);
    else if (ev.key === "ArrowDown" && buttons.length) buttons[(i + 1) % buttons.length].focus();
    else if (ev.key === "ArrowUp" && buttons.length) buttons[(i - 1 + buttons.length) % buttons.length].focus();
    else return;
    ev.preventDefault();
  });
}

// ------------------------------------------------------------------ details

const fact = (label, value, cls) => (value === null || value === undefined || value === "" ? [] : [el("dt", {}, label), el("dd", { class: cls || "" }, value)]);
const secsText = (s) => (s < 120 ? t("secsAgo", { s }) : t("minsAgo", { m: Math.round(s / 60) }));

function statusLine(item) {
  let dot = "", text = "";
  if (item.kind === "host") {
    if (item.local) { dot = status ? "ok" : ""; text = status ? t("serviceOk") : t("serviceDown"); }
    else if (item.online && !item.paired) { dot = "warn"; text = item.peer.paired ? t("stateUnpairedThere", { host: item.name }) : t("stateNotPaired"); }
    else { dot = item.online ? "ok" : ""; text = item.online ? t("peerOnline") : `${t("peerOffline")}${item.peer.error ? " – " + item.peer.error : ""}`; }
  } else if (item.local) {
    if (item.state === "local") { dot = ""; text = t("stateLocal"); }
    if (item.state === "shared") { dot = "ok"; text = t("stateShared"); }
    if (item.state === "lent") { dot = "busy"; text = t("stateLent", { host: item.usedBy }); }
    if (item.state === "blocked") { dot = ""; text = t({ auto: "stateLockedAuto", user: "stateLockedUser" }[item.lock] || "stateBlocked"); }
  } else {
    const host = item.host.name;
    const imp = item.imp;
    if (item.state === "here" && imp) {
      const vars = { host, error: imp.last_error || "" };
      [dot, text] = {
        connecting: ["warn", t("connecting", vars)],
        attached: imp.last_error ? ["warn", `${t("attached")} – ${imp.last_error}`] : ["busy", t("attached")],
        waiting: ["warn", t("waiting", vars)],
        disabled: ["", t("disabled", vars)],
        idle: ["warn", t("connecting", vars)],
      }[imp.state] || ["", imp.state];
    } else if (item.state === "used") { dot = "busy"; text = t("statePeerUsed", { user: item.usedBy }); }
    else if (item.state === "shared") { dot = item.host.online ? "ok" : ""; text = item.host.online ? t("statePeerShared", { host }) : t("peerOffline"); }
    else if (item.state === "never") { text = t("statePeerNever", { host }); }
    else { text = t("statePeerNotShared", { host }); }
    if (imp && !imp.wanted && imp.state === "disabled" && imp.last_error) { dot = "warn"; text = `${t("disabled", { error: imp.last_error })}`; }
  }
  return el("div", { class: "status-line" }, el("span", { class: "dot " + dot }), el("span", {}, text));
}

function problemBox(d) {
  const e = d.problem;
  if (!e) return null;
  const programs = (e.blocking_processes || []).join(", ");
  const vars = { client: e.problem_client_name || e.problem_client || "?", device: d.name, programs };
  return el("div", { class: "alert" }, el("div", { class: "dot warn" }),
    el("div", { class: "main" }, el("div", { class: "title" }, t("blockedTitle", vars)), el("div", {}, programs ? t("blockedKnown", vars) : t("blockedUnknown", vars))),
    programs ? closeProgramsButton(e.blocking_processes, vars) : null);
}

function closeProgramsButton(names, vars) {
  return el("button", { disabled: closing, onclick: async () => {
    closing = true; refresh();
    try { await invoke("close_programs", { names }); sawRemoteUseAfterClose = false; }
    finally { closing = false; }
  } }, closing ? t("closing") : t("closeAndShare", vars));
}

function profileFacts(p) {
  if (!p) return [];
  const ready = (p.ready || []).map((r) => t("ready_" + r)).join(", ");
  let audio = "";
  if (p.linux_audio) {
    const la = p.linux_audio, parts = [];
    if (la.never_default) parts.push(t("laNeverDefault"));
    if (la.pro_audio) parts.push(t("laProAudio"));
    if (la.headroom || la.period_size) parts.push(t("laBuffer", { h: la.headroom || "–", p: la.period_size || "–" }));
    audio = parts.join(" · ");
  }
  return [
    ...fact(t("profName"), p.name),
    ...fact(t("profBlocking"), (p.blocking_processes || []).join(", ")),
    ...fact(t("profReady"), ready),
    ...fact(t("profReconnect"), t("reconnect_" + (p.reconnect || "auto"))),
    ...fact(t("profLinuxAudio"), audio),
  ];
}

const ramText = (mb) => (mb >= 1024 ? `${Math.round(mb / 1024)} GB` : `${mb} MB`);

function systemFacts(sys) {
  if (!sys) return [];
  return [
    ...fact(t("sysOs"), sys.os),
    ...fact(t("sysCpu"), sys.cpu ? (sys.threads ? `${sys.cpu} (${t("threads", { n: sys.threads })})` : sys.cpu) : null),
    ...fact(t("sysGpu"), (sys.gpus || []).join(", ")),
    ...fact(t("sysRam"), sys.ram_mb ? ramText(sys.ram_mb) : null),
  ];
}

function hostDetails(h) {
  const p = h.peer;
  const sys = h.local ? status && status.system : p.hello && p.hello.system;
  const sysFacts = systemFacts(sys);
  const how = h.local ? null : p.added ? t("how_added") : p.manual ? t("how_config") : t("how_discovered");
  const can = h.local ? [canShare && t("canExport"), canImport && t("canImport")] : p.hello ? [p.hello.can_export && t("canExport"), p.hello.can_import && t("canImport")] : [];
  return [
    ...(sysFacts.length ? [el("h3", {}, t("sectionSystem")), el("dl", { class: "facts" }, ...sysFacts)] : []),
    el("h3", {}, t("sectionDetails")),
    el("dl", { class: "facts" },
      ...fact(t("hostAddress"), h.local ? null : p.address, "mono"),
      ...fact(t("hostVersion"), h.local ? (status ? status.version : null) : p.hello && p.hello.version),
      ...fact(t("hostCan"), can.filter(Boolean).join(", ")),
      ...fact(t("hostHow"), how),
      ...fact(t("hostPaired"), h.local ? null : h.paired ? t("yes") : t("no")),
      ...fact(t("hostKey"), h.local ? status && status.fingerprint : p.fingerprint, "mono"),
      ...fact(t("hostSeen"), h.local || p.last_seen_secs === null || p.last_seen_secs === undefined ? null : secsText(p.last_seen_secs)),
      ...fact(t("hostDevices"), String(h.devices.length)),
      ...fact(t("factAudioRules"), h.local && status && status.audio_rules ? (status.audio_rules.active ? t("audioActive") : t("audioInactive")) : null),
      ...fact(t("hostId"), h.local ? status && status.host_id : p.hello && p.hello.host_id, "mono")),
  ];
}

function deviceDetails(d) {
  const dev = d.device;
  const profile = d.local && d.profile ? d.profile : profileOf(d);
  const warnings = (d.warnings && d.warnings.length ? d.warnings : profile ? profile.warnings : []) || [];
  const out = [];
  if (warnings.length) {
    out.push(el("h3", {}, t("sectionWarnings")),
      el("ul", { class: "warnings" }, ...warnings.map((w) => el("li", { class: RISKY.includes(w) ? "risky" : "" }, t("warnText_" + w)))));
  }
  if (d.imp && (d.state === "here" || d.imp.attach_count)) {
    const i = d.imp;
    out.push(el("h3", {}, t("sectionConnection")), el("dl", { class: "facts" },
      ...fact(t("connPort"), i.bus_id, "mono"),
      ...fact(t("connSince"), i.state === "attached" ? secsText(i.since_secs) : null),
      ...fact(t("connCount"), i.attach_count ? t("times", { n: i.attach_count }) : null),
      ...fact(t("connError"), i.last_error)));
  }
  out.push(el("h3", {}, t("sectionDetails")), el("dl", { class: "facts" },
    ...fact(t("factComputer"), d.host.name),
    ...fact(t("factKind"), kindText(d.classes)),
    ...fact(t("factIds"), `${hex(dev.vendor_id)}:${hex(dev.product_id)}`, "mono"),
    ...fact(t("factSerial"), dev.serial, "mono"),
    ...fact(t("factMaker"), dev.manufacturer),
    ...fact(t("factProduct"), dev.product),
    ...fact(t("factPort"), dev.bus_id, "mono"),
    ...fact(t("factDriver"), dev.driver),
    ...fact(t("factPlatformId"), d.local ? dev.platform_id : null, "mono")));
  const pf = profileFacts(profile);
  if (pf.length) out.push(el("h3", {}, t("sectionProfile")), el("dl", { class: "facts" }, ...pf));
  return out;
}

// Buttons for the details pane; several "give to" targets share one button with a menu.
function actionButtons(actions) {
  const give = actions.filter((a) => a.group === "give");
  const out = [];
  let giveDone = false;
  for (const a of actions) {
    if (a.group === "give" && give.length > 1) {
      if (giveDone) continue;
      giveDone = true;
      out.push(el("button", { "aria-haspopup": "menu", onclick: (ev) => {
        const r = ev.currentTarget.getBoundingClientRect();
        openMenuWith(give.map((g) => ({ ...g, label: g.target })), r.left, r.bottom + 4);
      } }, t("giveToMany") + " ▾"));
    } else {
      out.push(el("button", { class: a.secondary ? "secondary" : "", onclick: () => runAction(a) }, a.label));
    }
  }
  return out;
}

let detailsSignature = "";
function renderDetails(force) {
  const box = document.getElementById("details");
  const item = itemByKey(selectedKey) || (flat[0] && itemByKey(flat[0].key));
  // items point to their host and back: describe those links by name only
  const plain = (k, v) => (k === "host" && v && v.key ? `${v.name}/${v.online}` : k === "importers" ? v.map((p) => p.address) : v);
  const sig = JSON.stringify([LANG, closing, item, profiles.size, [...profiles.values()].filter(Boolean).length, status && status.pairings], plain);
  if (!force && sig === detailsSignature) return;
  detailsSignature = sig;
  if (!item) { box.replaceChildren(el("div", { class: "empty" }, t("selectHint"))); return; }
  const isHost = item.kind === "host";
  const actions = actionsOf(item);
  let hint = null;
  if (!actions.length && !isHost) {
    if (item.local && !canShare) hint = t("hintNoShare");
    else if (!item.local && item.state === "not_shared") hint = t("hintAskShare", { host: item.host.name });
  }
  box.replaceChildren(...[
    el("div", { class: "title-row" }, el("span", { class: "icon" }, isHost ? "🖥️" : iconOf(item.classes)), el("h2", {}, item.name)),
    isHost ? el("div", { class: "muted" }, item.local ? t("thisComputerTag") : "") : el("div", { class: "muted" }, kindText(item.classes)),
    statusLine(item),
    isHost ? null : problemBox(item),
    actions.length ? el("div", { class: "actions" }, ...actionButtons(actions)) : null,
    hint ? el("div", { class: "muted small" }, hint) : null,
    ...(isHost ? hostDetails(item) : deviceDetails(item)),
  ].filter(Boolean));
}

// ------------------------------------------------------------------ alerts, service

function renderHeader() {
  const pill = document.getElementById("service");
  pill.className = "pill " + (status ? "ok" : "bad");
  pill.textContent = status ? t("serviceOk") : t("serviceDown");
}

async function reloadAudio(button) {
  button.disabled = true;
  try {
    await invoke("reload_audio");
  } catch (e) {
    const msg = String(e);
    const busy = msg.match(/audio is in use right now \((.*)\)/);
    alert(busy ? t("audioBusy", { programs: busy[1] }) : msg);
  }
  button.disabled = false;
  refresh();
}

// Problems that need attention, whatever is selected.
function renderAlerts() {
  const box = document.getElementById("alerts");
  const alerts = [];
  if (status && status.audio_rules && !status.audio_rules.active) {
    alerts.push(el("div", { class: "alert" }, el("div", { class: "dot warn" }),
      el("div", { class: "main" }, el("div", { class: "title" }, t("audioRulesTitle")), el("div", {}, t("audioRulesText"))),
      el("button", { onclick: (ev) => reloadAudio(ev.currentTarget) }, t("reloadAudio"))));
  }
  if (!status) {
    alerts.push(el("div", { class: "alert" }, el("div", { class: "main" }, el("div", { class: "title" }, t("serviceDown")), el("div", {}, t("serviceDownText")), el("div", { class: "muted" }, statusError || ""))));
  } else {
    for (const e of status.exports) {
      if (!e.problem) continue;
      const device = deviceName(e.device) || e.rule;
      const programs = e.blocking_processes.join(", ");
      const vars = { client: e.problem_client_name || e.problem_client || "?", device, programs };
      alerts.push(el("div", { class: "alert" }, el("div", { class: "dot warn" }),
        el("div", { class: "main" }, el("div", { class: "title" }, t("blockedTitle", vars)), el("div", {}, programs ? t("blockedKnown", vars) : t("blockedUnknown", vars))),
        programs ? closeProgramsButton(e.blocking_processes, vars) : null));
    }
  }
  box.replaceChildren(...alerts);
}

async function maybeRestartPrograms() {
  const closed = await invoke("closed_programs");
  if (!closed.length || !status) return;
  const inUse = status.exports.some((e) => e.client);
  if (inUse) sawRemoteUseAfterClose = true;
  else if (sawRemoteUseAfterClose) {
    const started = await invoke("restart_closed_programs");
    sawRemoteUseAfterClose = false;
    console.log(t("restarted", { programs: started.join(", ") }));
  }
}

// ------------------------------------------------------------------ pairing

const dismissedPairings = new Set();
const askedAttention = new Set();
let pairingSignature = "";
const codeText = (c) => (c || "").replace(/^(\d{3})(\d{3})$/, "$1 $2");

// Pairings in progress: the code to type on the other computer, or a field for the code
// another computer shows. Rebuilt only when they change (keeps what the user is typing).
function renderPairing() {
  const list = ((status && status.pairings) || []).filter((p) => !dismissedPairings.has(p.id));
  const sig = JSON.stringify([LANG, list]);
  if (sig === pairingSignature) return;
  pairingSignature = sig;
  const box = document.getElementById("pairing");
  const dismiss = (p) => el("button", { class: "secondary", onclick: () => { dismissedPairings.add(p.id); renderPairing(); } }, t("pairDismiss"));
  box.replaceChildren(...list.map((p) => {
    const vars = { host: p.host_name };
    if (p.state === "paired") {
      return el("div", { class: "alert" }, el("div", { class: "dot ok" }), el("div", { class: "main" }, el("div", { class: "title" }, t("pairDone", vars))), dismiss(p));
    }
    if (p.state === "failed") {
      return el("div", { class: "alert" }, el("div", { class: "dot warn" }),
        el("div", { class: "main" }, el("div", { class: "title" }, t("pairFailed", vars)), el("div", {}, p.message || "")), dismiss(p));
    }
    if (p.outgoing) {
      return el("div", { class: "alert" }, el("div", { class: "dot busy" }),
        el("div", { class: "main" }, el("div", { class: "title" }, t("pairOutTitle", vars)), el("div", {}, t("pairOutText", vars)),
          el("div", { class: "pair-code" }, codeText(p.code))));
    }
    if (!askedAttention.has(p.id)) { askedAttention.add(p.id); invoke("attention").catch(() => {}); }
    const input = el("input", { class: "pair-input", inputmode: "numeric", autocomplete: "off", maxlength: "7", "aria-label": t("pairInLabel", vars) });
    const error = el("div", { class: "error small" });
    const answer = async (code) => {
      try { await invoke("pair_answer", { id: p.id, code }); } catch (e) { error.textContent = String(e); return; }
      refresh();
    };
    return el("form", { class: "alert", onsubmit: (ev) => { ev.preventDefault(); answer(input.value); } },
      el("div", { class: "dot warn" }),
      el("div", { class: "main" }, el("div", { class: "title" }, t("pairInTitle", vars)), el("div", {}, t("pairInText", vars)),
        el("label", { class: "pair-label" }, t("pairInLabel", vars), " ", input), error),
      el("button", { type: "submit" }, t("pairOk")),
      el("button", { type: "button", class: "secondary", onclick: () => answer(null) }, t("pairDecline")));
  }));
  const first = box.querySelector(".pair-input");
  if (first && document.activeElement !== first) first.focus();
}

// ------------------------------------------------------------------ add computer

function setupAddDialog() {
  const dialog = document.getElementById("addDialog");
  const input = document.getElementById("addAddress");
  const error = document.getElementById("addError");
  const ok = document.getElementById("addOk");
  document.getElementById("addComputer").addEventListener("click", () => {
    input.value = ""; error.textContent = ""; ok.disabled = false;
    input.placeholder = t("addPlaceholder");
    dialog.showModal();
    input.focus();
  });
  document.getElementById("addCancel").addEventListener("click", () => dialog.close());
  document.getElementById("addForm").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    ok.disabled = true; error.textContent = "";
    try {
      await invoke("add_peer", { address: input.value });
      dialog.close();
      refresh();
    } catch (e) {
      error.textContent = String(e);
    } finally {
      ok.disabled = false;
    }
  });
}

// ------------------------------------------------------------------ first start

function showWelcome() {
  const steps = [t("welcomeStep1"), t("welcomeStep2")];
  if (canShare) steps.push(t("welcomeShare"));
  if (canImport) steps.push(t("welcomeUse"));
  document.getElementById("welcomeBody").replaceChildren(
    el("p", {}, t("welcomeIntro")),
    el("ol", {}, ...steps.map((s) => el("li", {}, s))),
    el("p", { class: "muted small" }, t("welcomeSafety")));
  document.getElementById("welcomeDialog").showModal();
}

function setupWelcome() {
  document.getElementById("help").addEventListener("click", showWelcome);
  document.getElementById("welcomeDialog").addEventListener("close", () => store("usbipb.welcomed", true));
  if (!stored("usbipb.welcomed", false)) showWelcome();
}

// ------------------------------------------------------------------ main loop

let refreshing = false;
async function refresh() {
  if (refreshing) return;
  refreshing = true;
  try {
    try { status = await invoke("service_status"); statusError = null; } catch (e) { status = null; statusError = String(e); }
    try { localDevices = await invoke("list_devices"); } catch { localDevices = []; }
    renderHeader();
    renderPairing();
    renderAlerts();
    renderTree();
    if (!itemByKey(selectedKey) && flat[0]) { selectedKey = flat[0].key; renderTree(); }
    renderDetails();
    await maybeRestartPrograms();
  } finally {
    refreshing = false;
  }
}

(async () => {
  applyStaticTexts();
  document.getElementById("language").addEventListener("change", (ev) => {
    setLanguage(ev.target.value);
    treeSignature = ""; detailsSignature = ""; pairingSignature = "";
    refresh();
  });
  setupTree();
  setupMenu();
  setupAddDialog();
  const info = await invoke("app_info");
  canShare = info.can_share;
  canImport = info.can_import;
  document.getElementById("version").textContent = "v" + info.version;
  setupWelcome();
  await refresh();
  setInterval(refresh, 2000);
})();
