[CmdletBinding()]
param(
  [Parameter(Mandatory)][string]$EnvFile,
  [Parameter(Mandatory)][string]$ProjectName
)

$ErrorActionPreference = 'Stop'
$Root = [System.IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$LocalDirectory = Join-Path $Root '.local'

function Read-Environment {
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

function Invoke-JsonRequest {
  param(
    [Parameter(Mandatory)][ValidateSet('GET', 'POST', 'PUT')][string]$Method,
    [Parameter(Mandatory)][string]$Uri,
    [object]$Body,
    [string]$AccessToken
  )

  $headers = @{}
  if ($AccessToken) {
    $headers.Authorization = "Bearer $AccessToken"
  }
  $parameters = @{
    Method = $Method
    Uri = $Uri
    Headers = $headers
    ContentType = 'application/json'
  }
  if ($null -ne $Body) {
    $parameters.Body = $Body | ConvertTo-Json -Depth 12 -Compress
  }
  return Invoke-RestMethod @parameters
}

function Get-RegistrationMac {
  param(
    [Parameter(Mandatory)][string]$Secret,
    [Parameter(Mandatory)][string]$Nonce,
    [Parameter(Mandatory)][string]$Username,
    [Parameter(Mandatory)][string]$Password,
    [Parameter(Mandatory)][bool]$Admin
  )

  $adminValue = if ($Admin) { 'admin' } else { 'notadmin' }
  $message = "$Nonce`0$Username`0$Password`0$adminValue"
  $hmac = New-Object System.Security.Cryptography.HMACSHA1
  try {
    $hmac.Key = [System.Text.Encoding]::UTF8.GetBytes($Secret)
    $bytes = $hmac.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($message))
    return ([System.BitConverter]::ToString($bytes)).Replace('-', '').ToLowerInvariant()
  } finally {
    $hmac.Dispose()
  }
}

function Get-OrCreateMatrixToken {
  param(
    [Parameter(Mandatory)][hashtable]$Environment,
    [Parameter(Mandatory)][string]$Username,
    [Parameter(Mandatory)][string]$Password,
    [Parameter(Mandatory)][bool]$Admin,
    [Parameter(Mandatory)][string]$DisplayName
  )

  $base = 'http://127.0.0.1:18008'
  try {
    $login = Invoke-JsonRequest -Method POST -Uri "$base/_matrix/client/v3/login" -Body @{
      type = 'm.login.password'
      identifier = @{ type = 'm.id.user'; user = $Username }
      password = $Password
      initial_device_display_name = 'Agent Room Seed'
    }
    return $login.access_token
  } catch {
    $nonceResponse = Invoke-JsonRequest -Method GET -Uri "$base/_synapse/admin/v1/register"
    $macArguments = @{
      Secret = $Environment.SYNAPSE_REGISTRATION_SECRET
      Nonce = $nonceResponse.nonce
      Username = $Username
      Password = $Password
      Admin = $Admin
    }
    $mac = Get-RegistrationMac @macArguments
    $registration = Invoke-JsonRequest -Method POST -Uri "$base/_synapse/admin/v1/register" -Body @{
      nonce = $nonceResponse.nonce
      username = $Username
      password = $Password
      displayname = $DisplayName
      admin = $Admin
      mac = $mac
    }
    return $registration.access_token
  }
}

function Get-OrCreateLobby {
  param(
    [Parameter(Mandatory)][string]$AdminToken,
    [Parameter(Mandatory)][string]$AgentMatrixId
  )

  $base = 'http://127.0.0.1:18008'
  $alias = '#lobby:matrix.agent-room.localhost'
  $encodedAlias = [System.Uri]::EscapeDataString($alias)
  try {
    $existing = Invoke-JsonRequest -Method GET -Uri "$base/_matrix/client/v3/directory/room/$encodedAlias" -AccessToken $AdminToken
    return $existing.room_id
  } catch {
    $created = Invoke-JsonRequest -Method POST -Uri "$base/_matrix/client/v3/createRoom" -AccessToken $AdminToken -Body @{
      room_alias_name = 'lobby'
      name = 'Agent Room Local Lobby'
      topic = '仅供本地开发的可重建大厅'
      preset = 'public_chat'
      invite = @($AgentMatrixId)
    }
    return $created.room_id
  }
}

function Seed-ObjectStore {
  param([Parameter(Mandatory)][hashtable]$Environment)

  $seedDirectory = Join-Path $LocalDirectory 'seed'
  New-Item -ItemType Directory -Path $seedDirectory -Force | Out-Null
  $contentPath = Join-Path $seedDirectory 'welcome.md'
  Set-Content -LiteralPath $contentPath -Encoding UTF8 -Value '# Agent Room 本地种子内容'
  $mount = ([System.IO.Path]::GetFullPath($seedDirectory)).Replace('\', '/') + ':/seed:ro'
  $common = @(
    'run', '--rm',
    '--network', "$ProjectName`_default",
    '--env', "AWS_ACCESS_KEY_ID=$($Environment.S3_ACCESS_KEY)",
    '--env', "AWS_SECRET_ACCESS_KEY=$($Environment.S3_SECRET_KEY)",
    '--env', 'AWS_DEFAULT_REGION=us-east-1',
    '--volume', $mount,
    'amazon/aws-cli:2.36.29',
    '--endpoint-url', 'http://object-store:8333'
  )

  & docker @common s3api head-bucket --bucket agent-room-content 2>$null
  if ($LASTEXITCODE -ne 0) {
    & docker @common s3api create-bucket --bucket agent-room-content | Out-Null
    if ($LASTEXITCODE -ne 0) {
      throw '创建本地内容桶失败。'
    }
  }

  $putArguments = $common + @(
    's3api', 'put-object',
    '--bucket', 'agent-room-content',
    '--key', 'seed/welcome.md',
    '--body', '/seed/welcome.md',
    '--content-type', 'text/markdown'
  )
  & docker @putArguments | Out-Null
  if ($LASTEXITCODE -ne 0) {
    throw '写入本地种子内容失败。'
  }
}

$environment = Read-Environment
$adminArguments = @{
  Environment = $environment
  Username = 'developer'
  Password = $environment.SEED_ADMIN_PASSWORD
  Admin = $true
  DisplayName = 'Local Developer'
}
$adminToken = Get-OrCreateMatrixToken @adminArguments
$agentArguments = @{
  Environment = $environment
  Username = 'agent-alpha'
  Password = $environment.SEED_AGENT_PASSWORD
  Admin = $false
  DisplayName = 'Agent Alpha'
}
$agentToken = Get-OrCreateMatrixToken @agentArguments

$agentMatrixId = '@agent-alpha:matrix.agent-room.localhost'
$roomId = Get-OrCreateLobby -AdminToken $adminToken -AgentMatrixId $agentMatrixId
$encodedRoomId = [System.Uri]::EscapeDataString($roomId)
$joinArguments = @{
  Method = 'POST'
  Uri = "http://127.0.0.1:18008/_matrix/client/v3/rooms/$encodedRoomId/join"
  AccessToken = $agentToken
  Body = @{}
}
Invoke-JsonRequest @joinArguments | Out-Null

$profileStateKey = [System.Uri]::EscapeDataString($environment.SEED_AGENT_ID)
$profileArguments = @{
  Method = 'PUT'
  Uri = "http://127.0.0.1:18008/_matrix/client/v3/rooms/$encodedRoomId/state/org.agentroom.agent.profile.v1/$profileStateKey"
  AccessToken = $adminToken
  Body = @{
    schemaVersion = '1.0'
    agentId = $environment.SEED_AGENT_ID
    displayName = 'Agent Alpha'
    capabilities = @('chat', 'status')
  }
}
Invoke-JsonRequest @profileArguments | Out-Null

Seed-ObjectStore -Environment $environment

$result = [ordered]@{
  roomId = $roomId
  roomAlias = '#lobby:matrix.agent-room.localhost'
  agentId = $environment.SEED_AGENT_ID
  agentMatrixId = $agentMatrixId
  content = 's3://agent-room-content/seed/welcome.md'
}
$result | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $LocalDirectory 'seed-result.json') -Encoding UTF8
Write-Host '本地测试用户、Agent、大厅和内容对象已幂等创建。'
