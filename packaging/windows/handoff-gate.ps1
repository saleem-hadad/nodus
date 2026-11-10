# The Windows handoff gate (decision D12): installs a real Electron Nodus,
# then updates it to the new installer exactly as its updater does
# (src/main/updater.ts, unchanged since 8839972 in every Windows copy from
# 0.9.0 to 0.9.4), and checks what HANDOFF CONTRACT E promises: the Rust app
# in the same folder, the Electron files gone, the same registry values and
# shortcuts, the data folder kept, and the app started again with --updated.
#
#   pwsh packaging/windows/handoff-gate.ps1 -Old <Nodus-Setup-x64.exe of 0.9.4> `
#        -New <dist\Nodus-Setup-x64.exe> -Version <the new version>
#
# For a CI runner only: it installs into this user's %LOCALAPPDATA%\Programs\nodus
# and writes %APPDATA%\Nodus, as a user's machine has them, and stops the
# Nodus it started at the end. The seeded state.json is synthetic.
param(
  [Parameter(Mandatory)] [string] $Old,
  [Parameter(Mandatory)] [string] $New,
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

function Stop-Nodus {
  Get-CimInstance Win32_Process -Filter "Name = 'Nodus.exe'" | ForEach-Object {
    Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue
  }
}

# The Electron app, installed the way a user's first install was.
Write-Host "▸ Installing the Electron Nodus from $Old"
Stop-Nodus
$p = Start-Process -FilePath $Old -ArgumentList '/S' -PassThru -Wait
if ($p.ExitCode -ne 0) { Fail "The old installer exited with $($p.ExitCode)" }
if (-not (Test-Path (Join-Path $dir 'resources\app.asar'))) { Fail "No Electron install in $dir" }
$oldVersion = (Get-ItemProperty $uninstallKey).DisplayVersion
Pass "Electron Nodus $oldVersion in $dir"
# 0.9.4's arm64 installer leaves out every executable on Windows on Arm (its
# Nsis7z can't decode 7-Zip's ARM64 filter), so there was never a working
# arm64 0.9.4 there: its shortcuts point at a Nodus.exe that isn't there, and
# the update may make them again. Everything else is checked as usual.
$oldHadExe = Test-Path (Join-Path $dir 'Nodus.exe')
if (-not $oldHadExe) { Write-Host "  · the old install has no Nodus.exe (0.9.4's arm64 installer on Arm), so its shortcuts may be made again" }
foreach ($link in $startLink, $desktopLink) {
  if (-not (Test-Path $link)) { Fail "The old installer made no $link" }
}
$linkTimes = @{ start = (Get-Item $startLink).LastWriteTimeUtc; desktop = (Get-Item $desktopLink).LastWriteTimeUtc }

# Its data, as a user's would be: a synthetic board.
New-Item -ItemType Directory -Force -Path (Join-Path $userData 'updates') | Out-Null
$repo = Join-Path $env:RUNNER_TEMP 'handoff-demo'
New-Item -ItemType Directory -Force -Path $repo | Out-Null
$projectId = '0f1e2d3c-4b5a-4968-8778-695a4b3c2d1e'
$state = [ordered]@{
  projects = @([ordered]@{ id = $projectId; name = 'handoff-demo'; repoPath = $repo; color = 'blue'; createdAt = 1790000000000 })
  groups = @()
  boards = [ordered]@{}
  settings = [ordered]@{ theme = 'light' }
}
$stateFile = Join-Path $userData 'state.json'
[IO.File]::WriteAllText($stateFile, ($state | ConvertTo-Json -Depth 8))

# What 0.9.4 does on Restart to Update: the installer it kept in
# <userData>\updates, started detached with exactly these arguments, not
# waited for (it quits). Here the gate waits, to check the result.
$staged = Join-Path $userData "updates\Nodus-Setup-$Version.exe"
Copy-Item $New $staged -Force
Write-Host "▸ Updating it as 0.9.4 does: $staged --updated /S --force-run"
$p = Start-Process -FilePath $staged -ArgumentList '--updated', '/S', '--force-run' -PassThru
if (-not $p.WaitForExit(300000)) { Fail "The installer didn't finish within 5 minutes" }
if ($p.ExitCode -ne 0) { Fail "The installer exited with $($p.ExitCode)" }
Pass "the installer exited with 0"

# The Rust app in the same folder, and nothing of Electron's left.
$exe = Join-Path $dir 'Nodus.exe'
if (-not (Test-Path $exe)) { Fail "No $exe" }
$info = (Get-Item $exe).VersionInfo
if ($info.ProductName -ne 'Nodus' -or $info.FileVersion -ne $Version) {
  Fail "Nodus.exe is $($info.ProductName) $($info.FileVersion), not Nodus $Version"
}
if ($info.CompanyName -ne 'Saleem Hadad' -or $info.FileDescription -ne 'Nodus' -or $info.ProductVersion -ne "$Version.0") {
  Fail "Nodus.exe's version strings aren't 0.9.4's kind (REL-028)"
}
if (-not (Test-Path (Join-Path $dir 'Uninstall Nodus.exe'))) { Fail "No Uninstall Nodus.exe" }
$leftovers = @('resources', 'locales', 'ffmpeg.dll', 'icudtl.dat', 'resources.pak', 'v8_context_snapshot.bin') |
  Where-Object { Test-Path (Join-Path $dir $_) }
if ($leftovers) { Fail "Electron files left in ${dir}: $($leftovers -join ', ')" }
Pass "Nodus.exe $Version and Uninstall Nodus.exe in $dir, no Electron files"

# The registry values the next update reads (REL-021).
$install = Get-ItemProperty $installKey
if ($install.InstallLocation -ne $dir -or $install.KeepShortcuts -ne 'true' -or $install.ShortcutName -ne 'Nodus') {
  Fail "$installKey is $($install | Out-String)"
}
$uninstall = Get-ItemProperty $uninstallKey
$expected = @{
  DisplayName = "Nodus $Version"
  DisplayVersion = $Version
  UninstallString = "`"$dir\Uninstall Nodus.exe`" /currentuser"
  QuietUninstallString = "`"$dir\Uninstall Nodus.exe`" /currentuser /S"
  DisplayIcon = "$dir\Nodus.exe,0"
  Publisher = 'Saleem Hadad'
  Comments = 'A spatial board for managing Claude Code sessions per project'
  NoModify = 1
  NoRepair = 1
}
foreach ($name in $expected.Keys) {
  if ($uninstall.$name -ne $expected[$name]) { Fail "$name is '$($uninstall.$name)', not '$($expected[$name])'" }
}
if (-not ($uninstall.EstimatedSize -gt 0)) { Fail "No EstimatedSize" }
Pass "HKCU registry values as electron-builder writes them"

# The shortcuts kept as they were (--updated, KeepShortcuts), still Nodus's.
$shell = New-Object -ComObject WScript.Shell
foreach ($link in $startLink, $desktopLink) {
  if (-not (Test-Path $link)) { Fail "$link is gone" }
  if ($shell.CreateShortcut($link).TargetPath -ne $exe) { Fail "$link doesn't open $exe" }
}
if ((Get-Item $startLink).LastWriteTimeUtc -ne $linkTimes.start -or (Get-Item $desktopLink).LastWriteTimeUtc -ne $linkTimes.desktop) {
  if ($oldHadExe) { Fail "The shortcuts were made again instead of kept" }
  Write-Host "  · the shortcuts were made again (the old ones pointed at a missing Nodus.exe)"
}
$folder = (New-Object -ComObject Shell.Application).NameSpace((Split-Path $startLink))
$aumid = $folder.ParseName('Nodus.lnk').ExtendedProperty('System.AppUserModel.ID')
if ($aumid -ne 'dev.saleem.nodus') { Fail "The Start menu shortcut's AppUserModelID is '$aumid'" }
Pass "Start menu and Desktop shortcuts kept, AppUserModelID dev.saleem.nodus"

if (-not (Test-Path (Join-Path $env:LOCALAPPDATA 'nodus-updater\installer.exe'))) { Fail "No nodus-updater\installer.exe (REL-025)" }
Pass "installer kept in %LOCALAPPDATA%\nodus-updater"

# --force-run: the app started again, through the shortcut, with --updated,
# and it keeps running on the user's data.
$started = $null
$deadline = (Get-Date).AddSeconds(60)
while (-not $started -and (Get-Date) -lt $deadline) {
  $started = Get-CimInstance Win32_Process -Filter "Name = 'Nodus.exe'" |
    Where-Object { $_.ExecutablePath -eq $exe -and $_.CommandLine -match '--updated' } | Select-Object -First 1
  if (-not $started) { Start-Sleep -Milliseconds 500 }
}
if (-not $started) { Fail "Nodus.exe wasn't started with --updated" }
Start-Sleep -Seconds 10
if (-not (Get-Process -Id $started.ProcessId -ErrorAction SilentlyContinue)) { Fail "Nodus.exe quit by itself after the update" }
Pass "Nodus.exe started with --updated and is still running"

$after = Get-Content $stateFile -Raw | ConvertFrom-Json
if (-not ($after.projects | Where-Object { $_.id -eq $projectId })) { Fail "state.json lost the board" }
if (Get-ChildItem $userData -Filter 'state.json.unreadable-*') { Fail "The Rust app couldn't read 0.9.4's state.json" }
Pass "%APPDATA%\Nodus kept, and the Rust app read its state.json"

Stop-Nodus
Write-Host "✓ Electron Nodus $oldVersion updates itself to $Version"
