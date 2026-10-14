$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$DistRoot = Join-Path $Root "dist"
$Dist = Join-Path $DistRoot "AnMedia-2.0"
$Archive = Join-Path $DistRoot "AnMedia-2.0-Windows-x64.zip"

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Rust не установлен. Установите Rust через rustup и повторите сборку"
}

Push-Location $Root
try {
    cargo build --release --target x86_64-pc-windows-msvc
} finally {
    Pop-Location
}

if (Test-Path $Dist) {
    Remove-Item -Path $Dist -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
Copy-Item -Path (Join-Path $Root "target\x86_64-pc-windows-msvc\release\anmedia.exe") -Destination (Join-Path $Dist "AnMedia.exe") -Force
$Mpv = Join-Path $Root "mpv\libmpv-2.dll"
if (-not (Test-Path $Mpv)) {
    & (Join-Path $Root "scripts\fetch-libmpv.ps1")
}
if (Test-Path $Mpv) {
    Copy-Item -Path $Mpv -Destination $Dist -Force
} else {
    Write-Warning "mpv\libmpv-2.dll не найдена: сборка будет открывать только форматы с кодеками Windows"
}
Copy-Item -Path (Join-Path $Root "README.md") -Destination $Dist -Force
Copy-Item -Path (Join-Path $Root "LICENSE") -Destination $Dist -Force
if (Test-Path $Archive) {
    Remove-Item -Path $Archive -Force
}
Compress-Archive -Path (Join-Path $Dist "*") -DestinationPath $Archive -Force
Write-Host ("Готовая сборка: " + $Archive)
