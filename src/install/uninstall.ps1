$ErrorActionPreference = 'Stop'
$litecomTemp = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
$previousUnmanaged = $env:LITECOM_UNMANAGED_INSTALL
$previousInstallDir = $env:LITECOM_INSTALL_DIR
try {
  $env:LITECOM_UNMANAGED_INSTALL = Join-Path $litecomTemp 'bin'
  $env:LITECOM_INSTALL_DIR = $env:LITECOM_UNMANAGED_INSTALL
  & ([scriptblock]::Create((Invoke-RestMethod 'https://github.com/BerriAI/litecom/releases/download/v@VERSION@/litecom-installer.ps1')))
  $env:LITECOM_UNMANAGED_INSTALL = $previousUnmanaged
  $env:LITECOM_INSTALL_DIR = $previousInstallDir
  & (Join-Path $litecomTemp 'bin/litecom.exe') uninstall
  if ($LASTEXITCODE -ne 0) { throw 'Uninstall needs another try. Follow the message above, then rerun this command.' }
} finally {
  $env:LITECOM_UNMANAGED_INSTALL = $previousUnmanaged
  $env:LITECOM_INSTALL_DIR = $previousInstallDir
  if (Test-Path $litecomTemp) { Remove-Item -LiteralPath $litecomTemp -Recurse -Force }
}
