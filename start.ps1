$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot
cargo build --release
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
Start-Process 'http://127.0.0.1:3030'
& "$PSScriptRoot/target/release/taskdesk.exe"
