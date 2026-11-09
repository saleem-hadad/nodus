# Installs a new Nodus-Setup-<arch>.exe on a machine without Nodus, the way a
# user who downloads it does (`/S`, as CI ran 0.9.4's, REL-031), checks the
# install (REL-016, REL-021…REL-023, REL-025), then uninstalls it silently
# with the uninstaller it wrote and checks the data folder stays (REL-026),
# so the next update can run that uninstaller as 0.9.4's ran its own.
#
#   pwsh packaging/windows/install-check.ps1 -Installer <dist\Nodus-Setup-x64.exe> -Version <version>
#
# For a CI runner only: it uses this user's %LOCALAPPDATA%\Programs\nodus.
param(
  [Parameter(Mandatory)] [string] $Installer,
  [Parameter(Mandatory)] [string] $Version
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

function Fail([string] $message) { Write-Host "✗ $message"; exit 1 }
function Pass([string] $message) { Write-Host "  ✓ $message" }

$guid = '99c2a492-c8ad-53bf-923b-1df868e91b51' # gitleaks:allow (electron-builder's public UUIDv5 of the app id)
$dir = Join-Path $env:LOCALAPPDATA 'Programs\nodus'
$installKey = "HKCU:\Software\$guid"
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\$guid"
$startLink = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Nodus.lnk'
$desktopLink = Join-Path ([Environment]::GetFolderPath('Desktop')) 'Nodus.lnk'
$userData = Join-Path $env:APPDATA 'Nodus'
if (Test-Path $uninstallKey) { Fail "Nodus is installed already; this checks a fresh install" }

Write-Host "▸ Installing $Installer with /S"
$p = Start-Process -FilePath $Installer -ArgumentList '/S' -PassThru -Wait
if ($p.ExitCode -ne 0) { Fail "The installer exited with $($p.ExitCode)" }
$exe = Join-Path $dir 'Nodus.exe'
if (-not (Test-Path $exe) -or -not (Test-Path (Join-Path $dir 'Uninstall Nodus.exe'))) { Fail "No Nodus.exe and Uninstall Nodus.exe in $dir" }
if ((Get-Item $exe).VersionInfo.FileVersion -ne $Version) { Fail "Nodus.exe isn't $Version" }
Pass "Nodus.exe $Version and Uninstall Nodus.exe in $dir"

$install = Get-ItemProperty $installKey
if ($install.InstallLocation -ne $dir -or $install.KeepShortcuts -ne 'true' -or $install.ShortcutName -ne 'Nodus') { Fail "$installKey is wrong" }
$uninstall = Get-ItemProperty $uninstallKey
if ($uninstall.DisplayName -ne "Nodus $Version" -or $uninstall.DisplayVersion -ne $Version -or
    $uninstall.UninstallString -ne "`"$dir\Uninstall Nodus.exe`" /currentuser") { Fail "$uninstallKey is wrong" }
Pass "registry values"

$shell = New-Object -ComObject WScript.Shell
$folder = (New-Object -ComObject Shell.Application).NameSpace((Split-Path $startLink))
foreach ($link in $startLink, $desktopLink) {
  if (-not (Test-Path $link)) { Fail "No $link" }
  $shortcut = $shell.CreateShortcut($link)
  if ($shortcut.TargetPath -ne $exe -or $shortcut.Description -ne 'A spatial board for managing Claude Code sessions per project') { Fail "$link is wrong" }
}
$aumid = $folder.ParseName('Nodus.lnk').ExtendedProperty('System.AppUserModel.ID')
if ($aumid -ne 'dev.saleem.nodus') { Fail "The Start menu shortcut's AppUserModelID is '$aumid'" }
Pass "Start menu and Desktop shortcuts, AppUserModelID dev.saleem.nodus"
if (-not (Test-Path (Join-Path $env:LOCALAPPDATA 'nodus-updater\installer.exe'))) { Fail "No nodus-updater\installer.exe" }
Pass "installer kept in %LOCALAPPDATA%\nodus-updater"

# /S alone doesn't start the app (REL-024).
Start-Sleep -Seconds 3
if (Get-Process -Name Nodus -ErrorAction SilentlyContinue) { Fail "/S without --force-run started Nodus" }
Pass "not started without --force-run"

Write-Host "▸ Uninstalling with its QuietUninstallString"
New-Item -ItemType Directory -Force -Path $userData | Out-Null
$marker = Join-Path $userData 'install-check.txt'
Set-Content $marker 'kept'
$p = Start-Process -FilePath (Join-Path $dir 'Uninstall Nodus.exe') -ArgumentList '/currentuser', '/S' -PassThru -Wait
if ($p.ExitCode -ne 0) { Fail "The uninstaller exited with $($p.ExitCode)" }
# The uninstaller copies itself out and runs from there, so wait for the folder to go.
$deadline = (Get-Date).AddSeconds(60)
while ((Test-Path $exe) -and (Get-Date) -lt $deadline) { Start-Sleep -Milliseconds 500 }
if (Test-Path $exe) { Fail "Nodus.exe is still there" }
if ((Test-Path $uninstallKey) -or (Test-Path $startLink) -or (Test-Path $desktopLink)) { Fail "The uninstall left its registry key or shortcuts" }
if (-not (Test-Path $marker)) { Fail "The uninstall deleted %APPDATA%\Nodus" }
Remove-Item $marker
Pass "uninstalled; %APPDATA%\Nodus kept"
Write-Host "✓ Nodus $Version installs and uninstalls as 0.9.4 did"
