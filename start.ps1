$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Start-Process -FilePath "$PSScriptRoot/target/release/67ai.exe" -WorkingDirectory $PSScriptRoot -WindowStyle Hidden
