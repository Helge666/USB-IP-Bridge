# Prepares the Windows installer build (run by `cargo tauri build`, see
# gui/src-tauri/tauri.windows.conf.json): builds the service and the command line tool, and
# fetches the USB drivers USB-IP-Bridge uses, each checked against its known SHA-256:
# usbipd-win (signed VBoxUSB drivers, to share devices) and usbip-win2 (to use devices of
# other computers).
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..')
cargo build --release -p usbipb-service -p usbipb --manifest-path (Join-Path $repo 'Cargo.toml')
if ($LASTEXITCODE) { exit $LASTEXITCODE }

$extra = Join-Path $repo 'target\bundle-extra'
New-Item -ItemType Directory -Force $extra | Out-Null

function Fetch($name, $url, $sha256) {
    $file = Join-Path $extra $name
    if (-not (Test-Path $file) -or (Get-FileHash $file -Algorithm SHA256).Hash -ne $sha256) {
        "downloading $url"
        Invoke-WebRequest $url -OutFile $file -UseBasicParsing
    }
    if ((Get-FileHash $file -Algorithm SHA256).Hash -ne $sha256) {
        Remove-Item $file
        throw "${name}: checksum mismatch"
    }
}

Fetch 'usbipd-win.msi' 'https://github.com/dorssel/usbipd-win/releases/download/v5.3.0/usbipd-win_5.3.0_x64.msi' `
    '1c984914aec944de19b64eff232421439629699f8138e3ddc29301175bc6d938'
Fetch 'usbip-win2-setup.exe' 'https://github.com/vadimgrn/usbip-win2/releases/download/v.0.9.8.1/USBip-0.9.8.1-x64.exe' `
    '38cad6d4432b52d5bb9409d9ad03b72fdffc4ada4cd3a48fbeca1a2752a8518a'
Copy-Item (Join-Path $repo 'THIRD-PARTY.md') (Join-Path $extra 'THIRD-PARTY.txt')
