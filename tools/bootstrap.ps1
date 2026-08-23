[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$JustVersion = '1.58.0'
$PnpmVersion = '10.28.0'
$Root = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))

function Assert-Command {
  param([Parameter(Mandatory)][string]$Name)

  if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
    throw "缺少必需命令：$Name"
  }
}

function Invoke-CheckedCommand {
  param(
    [Parameter(Mandatory)][string]$Command,
    [Parameter(Mandatory)][string[]]$Arguments
  )

  & $Command @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "命令执行失败：$Command $($Arguments -join ' ')"
  }
}

Assert-Command -Name git
Assert-Command -Name cargo
Assert-Command -Name node
Assert-Command -Name corepack
Assert-Command -Name docker

Push-Location $Root
try {
  if (-not (Get-Command just -ErrorAction SilentlyContinue)) {
    Invoke-CheckedCommand -Command 'cargo' -Arguments @('install', 'just', '--locked', '--version', $JustVersion)
  }

  Invoke-CheckedCommand -Command 'corepack' -Arguments @("pnpm@$PnpmVersion", 'install', '--frozen-lockfile=false')
  Invoke-CheckedCommand -Command 'node' -Arguments @('tools/bootstrap.mjs')
  Invoke-CheckedCommand -Command 'corepack' -Arguments @("pnpm@$PnpmVersion", 'protocol:generate')
  Invoke-CheckedCommand -Command 'cargo' -Arguments @('fetch', '--locked')
} finally {
  Pop-Location
}

Write-Host 'Agent Room 开发环境已准备完成。'
