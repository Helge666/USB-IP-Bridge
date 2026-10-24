#!/usr/bin/env python3
"""Writes THIRD-PARTY.md: the components USB-IP-Bridge is built from, with their licenses.

Run from the repository root after dependencies change:  python3 packaging/third-party.py
"""
import json
import pathlib
import subprocess

TARGETS = ["x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu"]

HEADER = """# Third-party components

USB-IP-Bridge is free software under the GNU General Public License, version 3 or later
(see LICENSE). It is built from, and comes with, the following components of others. Their
licenses permit this; the full license texts are in each project's repository.

## Included in the Windows installer

| Component | Version | License | Source |
|---|---|---|---|
| usbipd-win (its signed VBoxUSB drivers are used to share devices) | 5.3.0 | GPL-3.0-only | https://github.com/dorssel/usbipd-win |

## Rust libraries compiled into the programs

| Crate | Version | License | Source |
|---|---|---|---|
"""


def main():
    root = pathlib.Path(__file__).resolve().parent.parent
    args = ["cargo", "metadata", "--format-version", "1", "--manifest-path", str(root / "Cargo.toml")]
    for t in TARGETS:
        args += ["--filter-platform", t]
    meta = json.loads(subprocess.run(args, check=True, capture_output=True, text=True, encoding="utf-8").stdout)
    members = set(meta["workspace_members"])
    packages = {p["id"]: p for p in meta["packages"]}
    # everything the programs depend on (not only dev-dependencies of tests)
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    seen, todo = set(), [m for m in members if not packages[m]["name"].startswith("usbipb-spike")]
    while todo:
        pid = todo.pop()
        for dep in nodes[pid]["deps"]:
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]) and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                todo.append(dep["pkg"])
    rows = sorted((packages[p] for p in seen if p not in members), key=lambda p: (p["name"], p["version"]))
    lines = [HEADER]
    for p in rows:
        source = p.get("repository") or f"https://crates.io/crates/{p['name']}"
        lines.append(f"| {p['name']} | {p['version']} | {p.get('license') or 'see source'} | {source} |\n")
    (root / "THIRD-PARTY.md").write_text("".join(lines), encoding="utf-8", newline="\n")
    print(f"THIRD-PARTY.md: {len(rows)} crates")


if __name__ == "__main__":
    main()
