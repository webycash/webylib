$ErrorActionPreference = "Stop"

$Repo = "webycash/webylib"
$Prefix = if ($env:WEBYLIB_PREFIX) { $env:WEBYLIB_PREFIX } else { "$env:LOCALAPPDATA\webylib" }
$Version = $env:WEBYLIB_VERSION

if (-not $Version) {
    $Latest = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest"
    $Version = $Latest.tag_name -replace '^v', ''
}

$Platform = "Windows-x86_64"
$Tarball = "webylib-$Version-$Platform.tar.gz"
$Url = "https://github.com/$Repo/releases/download/v$Version/$Tarball"

Write-Host "Downloading webylib v$Version for $Platform..."
Invoke-WebRequest -Uri $Url -OutFile $Tarball
Invoke-WebRequest -Uri "$Url.sha256" -OutFile "$Tarball.sha256"

Write-Host "Installing to $Prefix\bin..."
New-Item -ItemType Directory -Force -Path "$Prefix\bin" | Out-Null
tar xzf $Tarball
Copy-Item "webylib-$Version-$Platform\webyc.exe" "$Prefix\bin\"

Remove-Item $Tarball, "$Tarball.sha256", "webylib-$Version-$Platform" -Recurse -Force

Write-Host "Installed webyc.exe to $Prefix\bin\webyc.exe"

$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$Prefix\bin*") {
    Write-Host "Add $Prefix\bin to your PATH"
}
