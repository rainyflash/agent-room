[CmdletBinding()]
param(
  [string]$BaseUrl = 'http://127.0.0.1:28080',
  [string]$Realm = 'agent-room'
)

$ErrorActionPreference = 'Stop'

$userCode = $env:AGENT_ROOM_TEST_DEVICE_USER_CODE
$username = $env:AGENT_ROOM_TEST_OIDC_USERNAME
$password = $env:AGENT_ROOM_TEST_OIDC_PASSWORD
if ([string]::IsNullOrWhiteSpace($userCode) -or
    [string]::IsNullOrWhiteSpace($username) -or
    [string]::IsNullOrWhiteSpace($password)) {
  throw '缺少隔离测试所需的设备码、用户名或密码环境变量。'
}

$origin = [uri]$BaseUrl
$parsedAddress = $null
$isIpAddress = [System.Net.IPAddress]::TryParse($origin.Host, [ref]$parsedAddress)
$isLoopback = ($isIpAddress -and [System.Net.IPAddress]::IsLoopback($parsedAddress)) -or
  $origin.Host.Equals('localhost', [System.StringComparison]::OrdinalIgnoreCase)
if (-not $origin.IsAbsoluteUri -or
    $origin.Scheme -notin @('http', 'https') -or
    -not $isLoopback -or
    $origin.AbsolutePath -notin @('', '/')) {
  throw '设备授权测试助手只允许访问无路径的回环 HTTP(S) 地址。'
}

Add-Type -AssemblyName System.Net.Http
$handler = [System.Net.Http.HttpClientHandler]::new()
$handler.AllowAutoRedirect = $false
$handler.UseCookies = $false
$client = [System.Net.Http.HttpClient]::new($handler)
$cookies = @{}

function New-FormContent {
  param([Parameter(Mandatory)][hashtable]$Fields)

  $pairs = [System.Collections.Generic.List[System.Collections.Generic.KeyValuePair[string,string]]]::new()
  foreach ($field in $Fields.GetEnumerator()) {
    $pairs.Add([System.Collections.Generic.KeyValuePair[string,string]]::new(
      [string]$field.Key,
      [string]$field.Value
    ))
  }
  return [System.Net.Http.FormUrlEncodedContent]::new($pairs)
}

function Invoke-LocalRequest {
  param(
    [Parameter(Mandatory)][ValidateSet('GET', 'POST')][string]$Method,
    [Parameter(Mandatory)][uri]$Uri,
    [hashtable]$Fields
  )

  if ($Uri.Host -ne $origin.Host -or $Uri.Port -ne $origin.Port -or $Uri.Scheme -ne $origin.Scheme) {
    throw '授权流程试图离开固定回环 Origin。'
  }
  $request = [System.Net.Http.HttpRequestMessage]::new(
    [System.Net.Http.HttpMethod]::new($Method),
    $Uri
  )
  try {
    if ($cookies.Count -gt 0) {
      $cookieHeader = ($cookies.GetEnumerator() | ForEach-Object {
        "$($_.Key)=$($_.Value)"
      }) -join '; '
      [void]$request.Headers.TryAddWithoutValidation('Cookie', $cookieHeader)
    }
    if ($null -ne $Fields) {
      $request.Content = New-FormContent -Fields $Fields
    }

    $response = $client.SendAsync($request).GetAwaiter().GetResult()
    try {
      if ($response.Headers.Contains('Set-Cookie')) {
        foreach ($headerValue in $response.Headers.GetValues('Set-Cookie')) {
          $pair = $headerValue.Split(';', 2)[0].Split('=', 2)
          if ($pair.Count -eq 2 -and -not [string]::IsNullOrWhiteSpace($pair[0])) {
            $cookies[$pair[0]] = $pair[1]
          }
        }
      }
      return [pscustomobject]@{
        Status = [int]$response.StatusCode
        Location = $response.Headers.Location
        Body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
        Uri = $Uri
      }
    } finally {
      $response.Dispose()
    }
  } finally {
    $request.Dispose()
  }
}

function Resolve-Location {
  param(
    [Parameter(Mandatory)][uri]$Current,
    [Parameter(Mandatory)][uri]$Location
  )

  if ($Location.IsAbsoluteUri) {
    return $Location
  }
  return [uri]::new($Current, $Location)
}

function Follow-Redirects {
  param([Parameter(Mandatory)]$Response)

  $current = $Response
  for ($attempt = 0; $attempt -lt 10; $attempt++) {
    if ($current.Status -notin @(301, 302, 303, 307, 308)) {
      return $current
    }
    if ($null -eq $current.Location) {
      throw '授权响应缺少重定向地址。'
    }
    $next = Resolve-Location -Current $current.Uri -Location $current.Location
    $current = Invoke-LocalRequest -Method GET -Uri $next
  }
  throw '授权流程重定向次数超出上限。'
}

function Find-FormAction {
  param(
    [Parameter(Mandatory)][string]$Html,
    [Parameter(Mandatory)][string]$FormId,
    [Parameter(Mandatory)][uri]$Current
  )

  $pattern = '<form[^>]+id="' + [regex]::Escape($FormId) + '"[^>]+action="([^"]+)"'
  $match = [regex]::Match($Html, $pattern, 'IgnoreCase')
  if (-not $match.Success) {
    $pattern = '<form[^>]+action="([^"]+)"[^>]+id="' + [regex]::Escape($FormId) + '"'
    $match = [regex]::Match($Html, $pattern, 'IgnoreCase')
  }
  if (-not $match.Success) {
    $available = [regex]::Matches($Html, '<form[^>]+id="([^"]+)"', 'IgnoreCase') |
      ForEach-Object { $_.Groups[1].Value }
    $fieldNames = [regex]::Matches($Html, '<(?:input|button)[^>]+name="([^"]+)"', 'IgnoreCase') |
      ForEach-Object { $_.Groups[1].Value } |
      Sort-Object -Unique
    $summary = if ($available.Count -eq 0) { '无具名表单' } else { $available -join ', ' }
    $fields = if ($fieldNames.Count -eq 0) { '无字段' } else { $fieldNames -join ', ' }
    $title = [regex]::Match($Html, '<title>(.*?)</title>', 'IgnoreCase, Singleline').Groups[1].Value.Trim()
    throw "授权页面缺少预期表单：$FormId；当前路径：$($Current.AbsolutePath)；标题：$title；当前页面：$summary；字段：$fields。"
  }
  $decoded = [System.Net.WebUtility]::HtmlDecode($match.Groups[1].Value)
  return [uri]::new($Current, $decoded)
}

function Find-FormActionByField {
  param(
    [Parameter(Mandatory)][string]$Html,
    [Parameter(Mandatory)][string]$FieldName,
    [Parameter(Mandatory)][uri]$Current
  )

  $pattern = '(?s)<form[^>]+action="([^"]+)"[^>]*>.*?name="' +
    [regex]::Escape($FieldName) + '".*?</form>'
  $match = [regex]::Match($Html, $pattern, 'IgnoreCase')
  if (-not $match.Success) {
    throw "授权页面缺少包含字段 $FieldName 的表单。"
  }
  $decoded = [System.Net.WebUtility]::HtmlDecode($match.Groups[1].Value)
  return [uri]::new($Current, $decoded)
}

function Find-InputValue {
  param(
    [Parameter(Mandatory)][string]$Html,
    [Parameter(Mandatory)][string]$FieldName,
    [string]$DefaultValue
  )

  $escapedName = [regex]::Escape($FieldName)
  $patterns = @(
    ('<(?:input|button)[^>]+name="' + $escapedName + '"[^>]+value="([^"]*)"'),
    ('<(?:input|button)[^>]+value="([^"]*)"[^>]+name="' + $escapedName + '"')
  )
  foreach ($pattern in $patterns) {
    $match = [regex]::Match($Html, $pattern, 'IgnoreCase')
    if ($match.Success) {
      return [System.Net.WebUtility]::HtmlDecode($match.Groups[1].Value)
    }
  }
  if ($PSBoundParameters.ContainsKey('DefaultValue')) {
    return $DefaultValue
  }
  $tag = [regex]::Match(
    $Html,
    '<(?:input|button)[^>]+name="' + $escapedName + '"[^>]*>',
    'IgnoreCase'
  ).Value -replace 'value="[^"]*"', 'value="[已脱敏]"'
  throw "授权页面缺少字段值：$FieldName；字段标签：$tag。"
}

try {
  $escapedRealm = [uri]::EscapeDataString($Realm)
  $verificationUri = [uri]::new(
    $origin,
    "/realms/$escapedRealm/device?user_code=$([uri]::EscapeDataString($userCode))"
  )
  $loginPage = Follow-Redirects -Response (
    Invoke-LocalRequest -Method GET -Uri $verificationUri
  )
  if ($loginPage.Status -ne 200) {
    throw "设备码验证未进入登录页，状态码：$($loginPage.Status)。"
  }

  $loginAction = Find-FormAction -Html $loginPage.Body -FormId 'kc-form-login' -Current $loginPage.Uri
  $consentPage = Follow-Redirects -Response (
    Invoke-LocalRequest -Method POST -Uri $loginAction -Fields @{
      username = $username
      password = $password
      credentialId = ''
    }
  )
  if ($consentPage.Status -ne 200) {
    throw "测试账户登录失败，状态码：$($consentPage.Status)。"
  }
  if ($consentPage.Uri.AbsolutePath -eq "/realms/$escapedRealm/device/status" -and
      [string]::IsNullOrEmpty($consentPage.Uri.Query)) {
    Write-Host '✓ 隔离 Keycloak 设备授权已批准。'
    return
  }

  $consentAction = Find-FormActionByField `
    -Html $consentPage.Body `
    -FieldName 'accept' `
    -Current $consentPage.Uri
  $acceptValue = Find-InputValue -Html $consentPage.Body -FieldName 'accept' -DefaultValue 'Yes'
  $authorizationCode = Find-InputValue -Html $consentPage.Body -FieldName 'code'
  $completed = Follow-Redirects -Response (
    Invoke-LocalRequest -Method POST -Uri $consentAction -Fields @{
      accept = $acceptValue
      code = $authorizationCode
    }
  )
  if ($completed.Status -ne 200 -or
      $completed.Uri.AbsolutePath -ne "/realms/$escapedRealm/device/status" -or
      -not [string]::IsNullOrEmpty($completed.Uri.Query)) {
    throw '设备授权没有到达成功状态页。'
  }

  Write-Host '✓ 隔离 Keycloak 设备授权已批准。'
} finally {
  $client.Dispose()
  $handler.Dispose()
  $env:AGENT_ROOM_TEST_DEVICE_USER_CODE = $null
  $env:AGENT_ROOM_TEST_OIDC_USERNAME = $null
  $env:AGENT_ROOM_TEST_OIDC_PASSWORD = $null
}
