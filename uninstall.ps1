$ErrorActionPreference = "Stop"

$Prefix = if ($env:WEBYLIB_PREFIX) { $env:WEBYLIB_PREFIX } else { "$env:LOCALAPPDATA\webylib" }

if (Test-Path "$Prefix\bin\webyc.exe") {
    Remove-Item "$Prefix\bin\webyc.exe"
    Write-Host "Removed $Prefix\bin\webyc.exe"
} else {
    Write-Host "webyc.exe not found at $Prefix\bin\webyc.exe"
}

Write-Host "Note: Wallet data (*.db files) is not removed."
