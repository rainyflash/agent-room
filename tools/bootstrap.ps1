[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$Root = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))

Push-Location $Root
try {
  & node tools/bootstrap.mjs
  if ($LASTEXITCODE -ne 0) {
    throw "Agent Room bootstrap failed with exit code $LASTEXITCODE."
  }
} finally {
  Pop-Location
}
