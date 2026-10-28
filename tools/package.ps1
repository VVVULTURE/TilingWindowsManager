# Builds the release binaries and produces, in dist\:
#   TilingWindowsManager-<version>-portable-x64.zip   (unzip and run; settings kept beside the exe)
#   TilingWindowsManager-<version>-setup.exe          (per-user installer, no admin rights)
# Usage: powershell -ExecutionPolicy Bypass -File tools\package.ps1 [-SkipBuild] [-NoInstaller]
param([switch]$SkipBuild, [switch]$NoInstaller)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"' | Select-Object -First 1).Matches[0].Groups[1].Value
Write-Host "TilingWindowsManager $version"

if (-not $SkipBuild) {
    cargo build --release --locked -p twm -p twmctl -p twm-settings
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
}

$dist = Join-Path $root "dist"
$stage = Join-Path $dist "stage"
if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
New-Item -ItemType Directory -Force $stage, "$stage\docs", "$stage\examples\themes" | Out-Null

foreach ($exe in "twm.exe", "twmctl.exe", "twm-settings.exe") { Copy-Item "target\release\$exe" $stage }
foreach ($f in "README.md", "LICENSE") {
    if (-not (Test-Path $f)) { throw "$f is missing" }
    Copy-Item $f $stage
}
Copy-Item docs\*.md "$stage\docs"
Copy-Item config\default.toml "$stage\examples\config.toml"
Copy-Item config\themes\*.toml "$stage\examples\themes"
python tools\third_party.py "$stage\THIRD_PARTY_LICENSES.md"
if ($LASTEXITCODE -ne 0) { throw "third-party licence collection failed" }

# Portable zip: the `portable` marker keeps config and logs next to the exe.
$portable = Join-Path $dist "TilingWindowsManager-$version-portable-x64"
if (Test-Path $portable) { Remove-Item -Recurse -Force $portable }
Copy-Item -Recurse $stage $portable
Set-Content -Path "$portable\portable" -Value "Delete this file to keep settings in %APPDATA% instead of next to the program." -Encoding ascii
$zip = "$portable.zip"
if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path "$portable\*" -DestinationPath $zip
Remove-Item -Recurse -Force $portable
Write-Host "portable: $zip"

if (-not $NoInstaller) {
    $iscc = @("$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe", "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe", "$env:ProgramFiles\Inno Setup 6\ISCC.exe") | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) { throw "Inno Setup 6 (ISCC.exe) not found; install it or pass -NoInstaller" }
    & $iscc /Q "/DAppVersion=$version" "/DStageDir=$stage" "/DOutDir=$dist" installer\TilingWindowsManager.iss
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
    Write-Host "installer: $dist\TilingWindowsManager-$version-setup.exe"
}

Get-ChildItem $dist -File | ForEach-Object { "{0,-55} {1,10:N0} bytes  SHA256 {2}" -f $_.Name, $_.Length, (Get-FileHash $_.FullName).Hash }
