# Puts everything Nuclear Thunder Live needs into one folder you choose:
# the newest portable version, your settings and recordings from the
# installed version, a desktop shortcut and (if Git is installed) the
# source code. Nothing is deleted; uninstall the old version afterwards.
#
#   powershell -ExecutionPolicy Bypass -File move-to-one-folder.ps1 -Target "D:\Games\Nuclear Thunder Live"

param(
    [string]$Target = (Join-Path ([Environment]::GetFolderPath('Desktop')) 'Nuclear Thunder Live')
)
$ErrorActionPreference = 'Stop'
$App  = Join-Path $Target 'Nuclear Thunder Live'
$Repo = 'https://github.com/n0-0one/Nuclear-Escalation-map-with-localhost-app-V2'
$Temp = Join-Path $env:TEMP 'ntl-move'
New-Item -ItemType Directory -Force $Target, $Temp | Out-Null

Write-Host 'Downloading the newest portable version...'
$zip  = Join-Path $Temp 'portable.zip'
$sums = Join-Path $Temp 'SHA256SUMS.txt'
Invoke-WebRequest "$Repo/releases/latest/download/Nuclear-Thunder-Live-Portable.zip" -OutFile $zip -UseBasicParsing
Invoke-WebRequest "$Repo/releases/latest/download/SHA256SUMS.txt" -OutFile $sums -UseBasicParsing
$expected = ((Select-String -Path $sums -Pattern 'Portable\.zip').Line -split '\s+')[0].ToLower()
$actual   = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLower()
if ($expected -ne $actual) { throw 'The download did not match its checksum. Nothing was changed.' }

Write-Host 'Unpacking...'
$unpacked = Join-Path $Temp 'unpacked'
if (Test-Path $unpacked) { Remove-Item $unpacked -Recurse -Force }
Expand-Archive $zip $unpacked
New-Item -ItemType Directory -Force $App | Out-Null
Copy-Item (Join-Path $unpacked 'Nuclear-Thunder-Live\*') $App -Recurse -Force

Write-Host 'Copying your settings and recordings...'
$old      = Join-Path $env:APPDATA 'io.github.n0-0one.nuclear-thunder-live'
$userData = Join-Path $App 'user-data'
New-Item -ItemType Directory -Force $userData | Out-Null
foreach ($item in 'settings.json', 'recordings') {
    $from = Join-Path $old $item
    if (Test-Path $from) { Copy-Item $from $userData -Recurse -Force }
}

Write-Host 'Creating a desktop shortcut...'
$shell = New-Object -ComObject WScript.Shell
$link  = $shell.CreateShortcut((Join-Path ([Environment]::GetFolderPath('Desktop')) 'Nuclear Thunder Live.lnk'))
$link.TargetPath       = Join-Path $App 'Nuclear-Thunder-Live.exe'
$link.WorkingDirectory = $App
$link.Save()

if (Get-Command git -ErrorAction SilentlyContinue) {
    $src = Join-Path $Target 'source'
    if (-not (Test-Path $src)) {
        Write-Host 'Downloading the source code...'
        git clone --quiet $Repo $src
    }
} else {
    Write-Host 'Git is not installed, so the source code was skipped.'
}

Remove-Item $Temp -Recurse -Force
Write-Host ''
Write-Host "Done. Everything is in: $Target"
Write-Host 'Last step: uninstall the old version (Windows Settings > Apps > Nuclear Thunder Live > Uninstall).'
