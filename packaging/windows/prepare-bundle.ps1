# Prepares the Windows installer build (run by `cargo tauri build`, see
# gui/src-tauri/tauri.windows.conf.json): builds the service and the command line tool, and
# fetches usbipd-win, whose signed VBoxUSB drivers USB-IP-Bridge uses, checked against its
# known SHA-256.
$ErrorActionPreference = 'Stop'
$repo = Resolve-Path (Join-Path $PSScriptRoot '..\..')
cargo build --release -p usbipb-service -p usbipb --manifest-path (Join-Path $repo 'Cargo.toml')
if ($LASTEXITCODE) { exit $LASTEXITCODE }

$extra = Join-Path $repo 'target\bundle-extra'
New-Item -ItemType Directory -Force $extra | Out-Null
$version = '5.3.0'
$sha256 = '1c984914aec944de19b64eff232421439629699f8138e3ddc29301175bc6d938'
$msi = Join-Path $extra 'usbipd-win.msi'
if (-not (Test-Path $msi) -or (Get-FileHash $msi -Algorithm SHA256).Hash -ne $sha256) {
    "downloading usbipd-win $version"
    Invoke-WebRequest "https://github.com/dorssel/usbipd-win/releases/download/v$version/usbipd-win_${version}_x64.msi" -OutFile $msi -UseBasicParsing
}
if ((Get-FileHash $msi -Algorithm SHA256).Hash -ne $sha256) {
    Remove-Item $msi
    throw "usbipd-win.msi: checksum mismatch"
}
Copy-Item (Join-Path $repo 'THIRD-PARTY.md') (Join-Path $extra 'THIRD-PARTY.txt')
