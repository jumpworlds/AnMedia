$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
Push-Location $Root
try {
    cargo fmt --all --check
    cargo clippy --target x86_64-pc-windows-msvc --all-targets --all-features -- -D warnings
    cargo check --target x86_64-pc-windows-msvc
} finally {
    Pop-Location
}
