[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$Root = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$Artifacts = Join-Path $Root 'artifacts'
New-Item -ItemType Directory -Path $Artifacts -Force | Out-Null

Get-ChildItem -LiteralPath $Artifacts -Filter '*.cdx.json' -File |
  Remove-Item -Force

Push-Location $Root
try {
  $removedEnvironment = @{}
  Get-ChildItem Env: |
    Where-Object { $_.Name -eq 'NODE_PATH' -or $_.Name -match '(?i)(API_KEY|TOKEN|SECRET|PASSWORD|PRIVATE_KEY)$' } |
    ForEach-Object {
      $removedEnvironment[$_.Name] = $_.Value
      [System.Environment]::SetEnvironmentVariable($_.Name, $null, 'Process')
    }

  try {
    corepack 'pnpm@10.28.0' sbom:node
    if ($LASTEXITCODE -ne 0) {
      throw 'Node SBOM 生成失败。'
    }
  } finally {
    foreach ($entry in $removedEnvironment.GetEnumerator()) {
      [System.Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, 'Process')
    }
  }

  if (-not (Get-Command cargo-cyclonedx -ErrorAction SilentlyContinue)) {
    cargo install cargo-cyclonedx --locked --version 0.5.9
    if ($LASTEXITCODE -ne 0) {
      throw 'cargo-cyclonedx 安装失败。'
    }
  }

  cargo cyclonedx --all --format json --spec-version 1.5
  if ($LASTEXITCODE -ne 0) {
    throw 'Rust SBOM 生成失败。'
  }

  @('apps', 'crates') |
    ForEach-Object { Get-ChildItem -LiteralPath (Join-Path $Root $_) -Directory } |
    ForEach-Object {
      Get-ChildItem -LiteralPath $_.FullName -Filter '*.cdx.json' -File
    } |
    ForEach-Object {
      Move-Item -LiteralPath $_.FullName -Destination (Join-Path $Artifacts "rust-$($_.Name)") -Force
    }

  Get-ChildItem -LiteralPath $Artifacts -Filter '*.cdx.json' -File |
    ForEach-Object {
      $document = Get-Content -LiteralPath $_.FullName -Raw | ConvertFrom-Json
      if ($document.bomFormat -ne 'CycloneDX') {
        throw "无效的 CycloneDX 文档：$($_.Name)"
      }
    }
} finally {
  Pop-Location
}

Write-Host "SBOM 已写入 $Artifacts"
