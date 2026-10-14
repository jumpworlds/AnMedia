$ErrorActionPreference = "Stop"
# Скачивает libmpv-2.dll (сборка shinchiro/mpv-winbuild-cmake) в папку mpv\.
# DLL весит ~120 МБ, поэтому не хранится в репозитории.
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$Target = Join-Path $Root "mpv"
$Dll = Join-Path $Target "libmpv-2.dll"
if ((Test-Path $Dll) -and -not $args.Contains("-Force")) {
    Write-Host "libmpv уже скачана: $Dll"
    exit 0
}
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$release = Invoke-RestMethod "https://api.github.com/repos/shinchiro/mpv-winbuild-cmake/releases/latest" -Headers @{ "User-Agent" = "AnMedia" }
$asset = $release.assets | Where-Object { $_.name -match "^mpv-dev-x86_64-\d" } | Select-Object -First 1
if (-not $asset) { throw "В последнем релизе нет пакета mpv-dev-x86_64" }
New-Item -ItemType Directory -Force -Path $Target | Out-Null
$Archive = Join-Path $env:TEMP $asset.name
Write-Host "Скачивание $($asset.name)…"
Invoke-WebRequest $asset.browser_download_url -OutFile $Archive -UseBasicParsing
$Extract = Join-Path $env:TEMP "anmedia-libmpv"
if (Test-Path $Extract) { Remove-Item $Extract -Recurse -Force }
New-Item -ItemType Directory -Force -Path $Extract | Out-Null
& "$env:SystemRoot\System32\tar.exe" -xf $Archive -C $Extract
if ($LASTEXITCODE -ne 0) { throw "Не удалось распаковать $Archive (нужен Windows 10 1803+ с tar.exe)" }
Copy-Item (Join-Path $Extract "libmpv-2.dll") $Dll -Force
Remove-Item $Archive -Force
Remove-Item $Extract -Recurse -Force
Write-Host "Готово: $Dll"
