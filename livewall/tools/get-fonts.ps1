# Downloads IBM Plex Sans (SIL Open Font License) into settings\ui\fonts at build time.
# The app itself never touches the network.
$ErrorActionPreference = "Stop"
$dir = Join-Path (Split-Path $PSScriptRoot -Parent) "settings\ui\fonts"
New-Item -ItemType Directory -Force $dir | Out-Null
foreach ($w in "Regular", "Medium", "SemiBold") {
    $url = "https://cdn.jsdelivr.net/npm/@ibm/plex-sans/fonts/complete/woff2/IBMPlexSans-$w.woff2"
    $out = Join-Path $dir "IBMPlexSans-$w.woff2"
    try { Invoke-WebRequest $url -OutFile $out -UseBasicParsing; Write-Host "Saved $out" }
    catch { Write-Warning "Could not download $url. Download IBM Plex Sans from https://github.com/IBM/plex/releases and copy IBMPlexSans-$w.woff2 into $dir" }
}
