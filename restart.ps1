$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$executables = @(
    [IO.Path]::GetFullPath("$PSScriptRoot/target/release/67ai.exe")
    [IO.Path]::GetFullPath("$PSScriptRoot/target/debug/67ai.exe")
    # Allow the first restart after upgrading an existing installation.
    [IO.Path]::GetFullPath("$PSScriptRoot/target/release/taskdesk.exe")
    [IO.Path]::GetFullPath("$PSScriptRoot/target/debug/taskdesk.exe")
)
Get-Process -Name 67ai,taskdesk -ErrorAction SilentlyContinue |
    Where-Object { $_.Path -in $executables } |
    ForEach-Object {
        Stop-Process -Id $_.Id -Force
        if (-not $_.WaitForExit(10000)) { throw '67ai did not stop.' }
    }

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'Build failed. 67ai was not restarted.' }

$port = if ($env:67AI_PORT) { $env:67AI_PORT } else { '3030' }
$url = "http://127.0.0.1:$port"
$server = Start-Process -FilePath $executables[0] -WorkingDirectory $PSScriptRoot -WindowStyle Hidden -PassThru
$deadline = [DateTime]::UtcNow.AddSeconds(30)
while ([DateTime]::UtcNow -lt $deadline) {
    if ($server.HasExited) { throw "67ai exited with code $($server.ExitCode)." }
    try {
        $response = Invoke-WebRequest "$url/api/state" -UseBasicParsing -TimeoutSec 1
    } catch {
        Start-Sleep -Milliseconds 200
        continue
    }
    if ($response.StatusCode -eq 200 -and -not $server.HasExited) {
        Start-Process $url
        Write-Host "67ai running at $url"
        return
    }
}
throw "67ai did not respond at $url within 30 seconds."
