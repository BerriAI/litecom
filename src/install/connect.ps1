param(
  [Parameter(Mandatory=$true)][string]$Relay,
  [Parameter(Mandatory=$true)][string]$Code,
  [Parameter(Mandatory=$true)][string]$Connection,
  [string]$Agent,
  [string]$Workspace
)
$ErrorActionPreference = 'Stop'
$litecomBinary = (Get-Command litecom -ErrorAction SilentlyContinue).Source
if (-not $litecomBinary -or (& $litecomBinary --version) -ne 'litecom @VERSION@') {
  $litecomInstallDir = if ($env:LITECOM_INSTALL_DIR) { $env:LITECOM_INSTALL_DIR } else { Join-Path $env:USERPROFILE '.local\bin' }
  $previousInstallDir = $env:LITECOM_INSTALL_DIR
  try {
    $env:LITECOM_INSTALL_DIR = $litecomInstallDir
    & ([scriptblock]::Create((Invoke-RestMethod 'https://github.com/BerriAI/litecom/releases/download/v@VERSION@/litecom-installer.ps1')))
  } finally {
    $env:LITECOM_INSTALL_DIR = $previousInstallDir
  }
  $litecomBinary = Join-Path $litecomInstallDir 'litecom.exe'
}
$setupArguments = @('--connection', $Connection, 'setup', '--relay', $Relay, '--connect-code', $Code)
if ($Agent) { $setupArguments += @('--agent', $Agent) }
if ($Workspace) { $setupArguments += @('--workspace', $Workspace) }
& $litecomBinary @setupArguments
if ($LASTEXITCODE -ne 0) { throw 'Connection did not finish. Follow the message above to retry.' }
