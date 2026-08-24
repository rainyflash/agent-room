[CmdletBinding()]
param(
  [Parameter(Position = 0)]
  [ValidateSet('prepare', 'config', 'up', 'down', 'reset', 'health', 'seed')]
  [string]$Action = 'health'
)

$ErrorActionPreference = 'Stop'
$ProjectName = 'agent-room-dev'
$Root = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$ComposeFile = Join-Path $Root 'infra/compose/compose.yaml'
$EnvFile = Join-Path $Root '.env.local'
$LocalDirectory = Join-Path $Root '.local'
$GeneratedDirectory = Join-Path $LocalDirectory 'generated'

function Write-Utf8File {
  param(
    [Parameter(Mandatory)][string]$Path,
    [Parameter(Mandatory)][string]$Content
  )

  $encoding = New-Object System.Text.UTF8Encoding($false)
  [System.IO.File]::WriteAllText($Path, $Content, $encoding)
}

function New-RandomSecret {
  param([int]$ByteCount = 32)

  $bytes = New-Object byte[] $ByteCount
  $generator = [System.Security.Cryptography.RandomNumberGenerator]::Create()
  try {
    $generator.GetBytes($bytes)
  } finally {
    $generator.Dispose()
  }
  return ([System.BitConverter]::ToString($bytes)).Replace('-', '').ToLowerInvariant()
}

function Convert-ToComposePath {
  param([Parameter(Mandatory)][string]$Path)

  return ([System.IO.Path]::GetFullPath($Path)).Replace('\', '/')
}

function Read-Environment {
  if (-not (Test-Path -LiteralPath $EnvFile)) {
    throw "缺少本地环境文件：$EnvFile"
  }

  $values = @{}
  foreach ($line in Get-Content -LiteralPath $EnvFile -Encoding UTF8) {
    if ([string]::IsNullOrWhiteSpace($line) -or $line.StartsWith('#')) {
      continue
    }
    $parts = $line.Split('=', 2)
    if ($parts.Count -eq 2) {
      $values[$parts[0]] = $parts[1]
    }
  }
  return $values
}

function Write-Environment {
  if (Test-Path -LiteralPath $EnvFile) {
    $current = Read-Environment
    if (-not $current.ContainsKey('AGENT_ROOM_DB_RUNTIME_PASSWORD')) {
      $existing = [System.IO.File]::ReadAllText($EnvFile).TrimEnd()
      $runtimePassword = New-RandomSecret
      Write-Utf8File -Path $EnvFile -Content ("$existing`nAGENT_ROOM_DB_RUNTIME_PASSWORD=$runtimePassword`n")
    }
    return
  }

  $values = [ordered]@{
    AGENT_ROOM_ROOT = Convert-ToComposePath -Path $Root
    AGENT_ROOM_LOCAL_DIR = Convert-ToComposePath -Path $LocalDirectory
    POSTGRES_BOOTSTRAP_PASSWORD = New-RandomSecret
    AGENT_ROOM_DB_PASSWORD = New-RandomSecret
    AGENT_ROOM_DB_RUNTIME_PASSWORD = New-RandomSecret
    SYNAPSE_DB_PASSWORD = New-RandomSecret
    KEYCLOAK_DB_PASSWORD = New-RandomSecret
    KEYCLOAK_ADMIN = 'local-admin'
    KEYCLOAK_ADMIN_PASSWORD = New-RandomSecret
    KEYCLOAK_CLIENT_SECRET = New-RandomSecret
    SYNAPSE_REGISTRATION_SECRET = New-RandomSecret
    SYNAPSE_MACAROON_SECRET = New-RandomSecret
    SYNAPSE_FORM_SECRET = New-RandomSecret
    S3_ACCESS_KEY = 'agentroomlocal'
    S3_SECRET_KEY = New-RandomSecret
    SEED_ADMIN_PASSWORD = New-RandomSecret
    SEED_AGENT_PASSWORD = New-RandomSecret
    SEED_AGENT_ID = '01945c1e-7b5a-7c7f-8a28-2de53f56a9a3'
  }

  $content = ($values.GetEnumerator() | ForEach-Object { "$($_.Key)=$($_.Value)" }) -join "`n"
  Write-Utf8File -Path $EnvFile -Content ($content + "`n")
}

function Write-KeycloakRealm {
  param([Parameter(Mandatory)][hashtable]$Environment)

  $directory = Join-Path $GeneratedDirectory 'keycloak'
  New-Item -ItemType Directory -Path $directory -Force | Out-Null
  $path = Join-Path $directory 'realm-agent-room.json'

  $realm = [ordered]@{
    realm = 'agent-room'
    enabled = $true
    displayName = 'Agent Room Local'
    registrationAllowed = $false
    loginWithEmailAllowed = $true
    sslRequired = 'none'
    clients = @(
      [ordered]@{
        clientId = 'agent-room-web'
        name = 'Agent Room Web'
        enabled = $true
        publicClient = $false
        secret = $Environment.KEYCLOAK_CLIENT_SECRET
        standardFlowEnabled = $true
        directAccessGrantsEnabled = $false
        redirectUris = @(
          'https://api.agent-room.localhost/auth/oidc/callback',
          'http://127.0.0.1:8090/auth/oidc/callback'
        )
        webOrigins = @('https://app.agent-room.localhost', 'http://localhost:5173')
        attributes = @{ 'pkce.code.challenge.method' = 'S256' }
      }
    )
    users = @(
      [ordered]@{
        username = 'developer'
        enabled = $true
        emailVerified = $true
        email = 'developer@agent-room.test'
        firstName = 'Local'
        lastName = 'Developer'
        credentials = @(
          [ordered]@{
            type = 'password'
            value = $Environment.SEED_ADMIN_PASSWORD
            temporary = $false
          }
        )
      }
    )
  }

  Write-Utf8File -Path $path -Content (($realm | ConvertTo-Json -Depth 12) + "`n")
}

function Write-SeaweedConfig {
  param([Parameter(Mandatory)][hashtable]$Environment)

  $directory = Join-Path $GeneratedDirectory 'seaweedfs'
  New-Item -ItemType Directory -Path $directory -Force | Out-Null
  $path = Join-Path $directory 's3.json'
  $config = [ordered]@{
    identities = @(
      [ordered]@{
        name = 'agent-room-local'
        credentials = @(
          [ordered]@{
            accessKey = $Environment.S3_ACCESS_KEY
            secretKey = $Environment.S3_SECRET_KEY
          }
        )
        actions = @('Admin', 'Read', 'Write', 'List', 'Tagging')
      }
    )
  }
  Write-Utf8File -Path $path -Content (($config | ConvertTo-Json -Depth 8) + "`n")
}

function Write-SynapseConfig {
  param([Parameter(Mandatory)][hashtable]$Environment)

  $directory = Join-Path $GeneratedDirectory 'synapse'
  New-Item -ItemType Directory -Path $directory -Force | Out-Null
  $signingKey = Join-Path $directory 'matrix.agent-room.localhost.signing.key'
  if (-not (Test-Path -LiteralPath $signingKey)) {
    $mount = (Convert-ToComposePath -Path $directory) + ':/data'
    $dockerArguments = @(
      'run', '--rm',
      '--env', 'SYNAPSE_SERVER_NAME=matrix.agent-room.localhost',
      '--env', 'SYNAPSE_REPORT_STATS=no',
      '--volume', $mount,
      'matrixdotorg/synapse:v1.159.0', 'generate'
    )
    & docker @dockerArguments
    if ($LASTEXITCODE -ne 0) {
      throw 'Synapse 初始密钥生成失败。'
    }
  }

  $homeserver = @"
server_name: "matrix.agent-room.localhost"
public_baseurl: "http://localhost:18008/"
pid_file: /data/homeserver.pid
listeners:
  - port: 8008
    tls: false
    type: http
    x_forwarded: true
    resources:
      - names: [client, federation]
        compress: false
database:
  name: psycopg2
  args:
    user: synapse
    password: "$($Environment.SYNAPSE_DB_PASSWORD)"
    database: synapse
    host: postgres
    port: 5432
    cp_min: 2
    cp_max: 10
log_config: /data/matrix.agent-room.localhost.log.config
media_store_path: /data/media_store
registration_shared_secret: "$($Environment.SYNAPSE_REGISTRATION_SECRET)"
report_stats: false
macaroon_secret_key: "$($Environment.SYNAPSE_MACAROON_SECRET)"
form_secret: "$($Environment.SYNAPSE_FORM_SECRET)"
signing_key_path: /data/matrix.agent-room.localhost.signing.key
trusted_key_servers:
  - server_name: "matrix.org"
suppress_key_server_warning: true
enable_registration: false
rc_login:
  address:
    per_second: 100
    burst_count: 100
  account:
    per_second: 100
    burst_count: 100
  failed_attempts:
    per_second: 100
    burst_count: 100
"@
  Write-Utf8File -Path (Join-Path $directory 'homeserver.yaml') -Content ($homeserver + "`n")
}

function Prepare-Environment {
  if (-not (Get-Command docker -ErrorAction SilentlyContinue)) {
    throw '缺少 Docker。'
  }

  New-Item -ItemType Directory -Path $GeneratedDirectory -Force | Out-Null
  Write-Environment
  $environment = Read-Environment
  Write-KeycloakRealm -Environment $environment
  Write-SeaweedConfig -Environment $environment
  Write-SynapseConfig -Environment $environment
}

function Invoke-Compose {
  param([Parameter(Mandatory)][string[]]$Arguments)

  & docker compose --project-name $ProjectName --env-file $EnvFile --file $ComposeFile @Arguments
  if ($LASTEXITCODE -ne 0) {
    throw "Docker Compose 命令失败：$($Arguments -join ' ')"
  }
}

function Sync-KeycloakClient {
  $environment = Read-Environment
  $compose = @(
    'compose', '--project-name', $ProjectName, '--env-file', $EnvFile,
    '--file', $ComposeFile, 'exec', '-T', 'identity'
  )
  $admin = '/opt/keycloak/bin/kcadm.sh'
  $adminConfig = '/tmp/agent-room-kcadm.config'

  & docker @compose $admin config credentials `
    --config $adminConfig `
    --server 'http://127.0.0.1:8080' `
    --realm 'master' `
    --user $environment.KEYCLOAK_ADMIN `
    --password $environment.KEYCLOAK_ADMIN_PASSWORD | Out-Null
  if ($LASTEXITCODE -ne 0) {
    throw '无法登录本地 Keycloak 管理接口。'
  }

  $clientJson = & docker @compose $admin get clients `
    --config $adminConfig `
    --realm 'agent-room' `
    --query 'clientId=agent-room-web'
  if ($LASTEXITCODE -ne 0) {
    & docker @compose rm -f $adminConfig | Out-Null
    Write-Warning '现有 Keycloak 数据卷的管理凭据与 .env.local 不一致，无法自动迁移回调地址；全新环境不受影响。'
    return
  }
  $clients = ($clientJson -join "`n") | ConvertFrom-Json
  if ($clients.Count -ne 1) {
    throw '本地 Keycloak 必须且只能存在一个 agent-room-web 客户端。'
  }

  $redirectUris = 'redirectUris=["https://api.agent-room.localhost/auth/oidc/callback","http://127.0.0.1:8090/auth/oidc/callback"]'
  $webOrigins = 'webOrigins=["https://app.agent-room.localhost","http://localhost:5173"]'
  & docker @compose $admin update "clients/$($clients[0].id)" `
    --config $adminConfig `
    --realm 'agent-room' `
    --set $redirectUris `
    --set $webOrigins | Out-Null
  if ($LASTEXITCODE -ne 0) {
    throw '无法同步本地 Keycloak 回调地址。'
  }

  & docker @compose rm -f $adminConfig | Out-Null
}

function Test-HttpEndpoint {
  param(
    [Parameter(Mandatory)][string]$Name,
    [Parameter(Mandatory)][string]$Url,
    [switch]$AllowInvalidCertificate
  )

  $arguments = @('--fail', '--silent', '--show-error', '--max-time', '5')
  if ($AllowInvalidCertificate) {
    $arguments += '--insecure'
  }
  $arguments += $Url
  $curl = if ($IsWindows -or $env:OS -eq 'Windows_NT') { 'curl.exe' } else { 'curl' }
  & $curl @arguments | Out-Null
  if ($LASTEXITCODE -eq 0) {
    Write-Host "✓ $Name"
    return $true
  }
  Write-Host "✗ $Name"
  return $false
}

function Test-Services {
  $results = @()

  & docker compose --project-name $ProjectName --env-file $EnvFile --file $ComposeFile exec -T postgres pg_isready -U agent_room_bootstrap -d postgres | Out-Null
  $postgresHealthy = $LASTEXITCODE -eq 0
  Write-Host ($(if ($postgresHealthy) { '✓ PostgreSQL' } else { '✗ PostgreSQL' }))
  $results += $postgresHealthy
  $results += Test-HttpEndpoint -Name 'Synapse' -Url 'http://127.0.0.1:18008/_matrix/client/versions'
  $results += Test-HttpEndpoint -Name 'Keycloak' -Url 'http://127.0.0.1:18080/realms/agent-room'
  $results += Test-HttpEndpoint -Name 'SeaweedFS S3' -Url 'http://127.0.0.1:19333/cluster/status'
  $results += Test-HttpEndpoint -Name 'Mailpit' -Url 'http://127.0.0.1:18025/api/v1/info'
  $results += Test-HttpEndpoint -Name 'OpenTelemetry Collector' -Url 'http://127.0.0.1:13134/'
  $results += Test-HttpEndpoint -Name 'Caddy 内部 TLS' -Url 'https://gateway.agent-room.localhost:18443/healthz' -AllowInvalidCertificate

  if ($results -contains $false) {
    throw '一个或多个依赖未达到可用状态。'
  }
}

function Reset-Environment {
  $resolvedLocal = [System.IO.Path]::GetFullPath($LocalDirectory)
  if (-not $resolvedLocal.StartsWith($Root, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "拒绝删除项目目录之外的路径：$resolvedLocal"
  }
  if ((Split-Path -Leaf $resolvedLocal) -ne '.local') {
    throw "拒绝删除非 .local 目录：$resolvedLocal"
  }
  if ($ProjectName -ne 'agent-room-dev') {
    throw "拒绝清理未知 Compose 项目：$ProjectName"
  }

  if (Test-Path -LiteralPath $EnvFile) {
    Invoke-Compose -Arguments @('down', '--volumes', '--remove-orphans')
  }
  if (Test-Path -LiteralPath $resolvedLocal) {
    Remove-Item -LiteralPath $resolvedLocal -Recurse -Force
  }
  if (Test-Path -LiteralPath $EnvFile) {
    Remove-Item -LiteralPath $EnvFile -Force
  }
}

switch ($Action) {
  'prepare' {
    Prepare-Environment
  }
  'config' {
    Prepare-Environment
    Invoke-Compose -Arguments @('config', '--quiet')
    Write-Host 'Compose 配置有效。'
  }
  'up' {
    Prepare-Environment
    Invoke-Compose -Arguments @('config', '--quiet')
    Invoke-Compose -Arguments @('up', '--detach', '--wait', '--wait-timeout', '240')
    Sync-KeycloakClient
    Test-Services
  }
  'down' {
    if (Test-Path -LiteralPath $EnvFile) {
      Invoke-Compose -Arguments @('down', '--remove-orphans')
    }
  }
  'reset' {
    Reset-Environment
  }
  'health' {
    if (-not (Test-Path -LiteralPath $EnvFile)) {
      throw '环境尚未准备，请先运行 just dev-up。'
    }
    Test-Services
  }
  'seed' {
    Test-Services
    & (Join-Path $PSScriptRoot 'dev-seed.ps1') -EnvFile $EnvFile -ProjectName $ProjectName
    if ($LASTEXITCODE -ne 0) {
      throw '本地种子数据创建失败。'
    }
  }
}
