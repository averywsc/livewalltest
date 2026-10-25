# Builds the installer and portable exe. Run from a normal PowerShell window: .\tools\build.ps1
$ErrorActionPreference = "Stop"
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root

if (-not (Test-Path "vendor\libmpv-2.dll")) {
    throw "vendor\libmpv-2.dll is missing. See README step 3."
}
if (-not (Test-Path "settings\ui\fonts\IBMPlexSans-Regular.woff2")) {
    Write-Warning "IBM Plex Sans files are missing, so the window will use Segoe UI. Run tools\get-fonts.ps1 to add them."
}

Write-Host "1/5 Building the renderer"
cargo build --release -p livewall
if ($LASTEXITCODE -ne 0) { throw "renderer build failed" }

Write-Host "2/5 Building the settings window"
Push-Location settings
cargo tauri build --no-bundle
$code = $LASTEXITCODE
Pop-Location
if ($code -ne 0) { throw "settings build failed" }

Write-Host "3/5 Packing the portable payload"
New-Item -ItemType Directory -Force dist | Out-Null
$stage = "dist\stage"
if (Test-Path $stage) { Remove-Item $stage -Recurse -Force }
New-Item -ItemType Directory $stage | Out-Null
Copy-Item target\release\livewall.exe, target\release\livewall-settings.exe, vendor\libmpv-2.dll, LICENSE, THIRD_PARTY.md $stage
if (Test-Path dist\payload.zip) { Remove-Item dist\payload.zip }
Compress-Archive -Path "$stage\*" -DestinationPath dist\payload.zip

Write-Host "4/5 Building the portable exe"
cargo build --release -p livewall-portable
if ($LASTEXITCODE -ne 0) { throw "portable build failed" }
Copy-Item target\release\livewall-portable.exe dist\LiveWallpaper-Portable.exe -Force

Write-Host "5/5 Building the installer"
$iscc = @("$env:ProgramFiles(x86)\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe", "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6.3 or newer was not found. Install it from https://jrsoftware.org/isdl.php" }
& $iscc installer\livewall.iss
if ($LASTEXITCODE -ne 0) { throw "installer build failed" }

Write-Host ""
Write-Host "Done. Files in dist\:"
Get-ChildItem dist\LiveWallpaper-Setup.exe, dist\LiveWallpaper-Portable.exe | Format-Table Name, @{n="MB";e={[math]::Round($_.Length/1MB,1)}}
