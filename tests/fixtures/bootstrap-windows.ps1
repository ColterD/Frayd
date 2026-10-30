# Qualification fixture for client/install/bootstrap.ps1. It runs the unmodified installer inside this process and
# shadows only the commands the installer must not run for real. Functions precede cmdlets in command precedence
# (about_Command_Precedence). These are bounded interceptions for that one inspected script, not a security sandbox.
# Keep this file ASCII: Windows PowerShell 5.1 reads a script without a byte order mark as the ANSI code page.
param(
  [Parameter(Mandatory = $true)][string]$InstallerPath,
  [Parameter(Mandatory = $true)][string]$Events,
  [Parameter(Mandatory = $true)][string]$StartupRedirect,
  [Parameter(Mandatory = $true)][string]$Lib,
  [string]$Tunnel = '',
  [string]$RustupStub = ''
)
$ErrorActionPreference = 'Stop'

$global:FrayFixtureEvents = $Events
$global:FrayFixtureRustupStub = $RustupStub
$global:FrayFixtureStartupRedirect = $StartupRedirect
$global:FrayFixtureStartupBat = Join-Path ([Environment]::GetFolderPath('Startup')) 'frayd.bat'
$global:FrayFixtureUtf8 = New-Object System.Text.UTF8Encoding($false)

function global:Write-FixtureEvent([string]$Text) {
  [IO.File]::AppendAllText($global:FrayFixtureEvents, $Text + "`n", $global:FrayFixtureUtf8)
}

function global:Start-Process {
  [CmdletBinding()]
  param(
    [Parameter(Position = 0)][string]$FilePath,
    [Parameter(Position = 1)][string[]]$ArgumentList,
    [string]$WindowStyle
  )
  $isServer = $FilePath.EndsWith('target\release\frayd-server.exe', [StringComparison]::OrdinalIgnoreCase)
  if ($FilePath -ne 'tailscale' -and -not $isServer) { throw "Start-Process of an unexpected program: $FilePath" }
  Write-FixtureEvent ('process|{0}|{1}|{2}|{3}' -f $FilePath, ($ArgumentList -join ' '), $env:FRAYD_LIBRARY, $WindowStyle)
}

function global:Invoke-WebRequest {
  [CmdletBinding()]
  param(
    [Parameter(Position = 0)][string]$Uri,
    [string]$OutFile
  )
  Write-FixtureEvent ('web|{0}|{1}' -f $Uri, $OutFile)
  if ($Uri -ne 'https://win.rustup.rs/x86_64' -or -not $global:FrayFixtureRustupStub) { throw "Network access is prohibited: $Uri" }
  if (-not $OutFile.StartsWith($env:TEMP, [StringComparison]::OrdinalIgnoreCase)) { throw "Download outside TEMP: $OutFile" }
  Copy-Item -LiteralPath $global:FrayFixtureRustupStub -Destination $OutFile
}

function global:Out-File {
  [CmdletBinding()]
  param(
    [Parameter(Mandatory = $true, Position = 0)][string]$FilePath,
    [string]$Encoding,
    [Parameter(ValueFromPipeline = $true)][object]$InputObject
  )
  begin {
    $destination = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($FilePath)
    if ($destination -ne $global:FrayFixtureStartupBat) { throw "Out-File to an unexpected destination: $destination" }
    $forwarded = @{ FilePath = $global:FrayFixtureStartupRedirect }
    if ($PSBoundParameters.ContainsKey('Encoding')) { $forwarded['Encoding'] = $Encoding }
    Write-FixtureEvent ('outfile|{0}|{1}' -f $destination, $Encoding)
    $wrapped = $ExecutionContext.InvokeCommand.GetCommand('Microsoft.PowerShell.Utility\Out-File', [System.Management.Automation.CommandTypes]::Cmdlet)
    $scriptCommand = { & $wrapped @forwarded }
    $steppablePipeline = $scriptCommand.GetSteppablePipeline($MyInvocation.CommandOrigin)
    $steppablePipeline.Begin($PSCmdlet)
  }
  process { $steppablePipeline.Process($InputObject) }
  end { $steppablePipeline.End() }
}

Write-FixtureEvent ('engine|{0}|{1}' -f $PSVersionTable.PSVersion, $PSVersionTable.PSEdition)
$installerArguments = @{ Lib = $Lib }
if ($Tunnel) { $installerArguments['Tunnel'] = $Tunnel }
try {
  & $InstallerPath @installerArguments
  Write-FixtureEvent ('result|returned|{0}' -f $LASTEXITCODE)
} catch {
  Write-FixtureEvent ('result|threw|{0}' -f $_.Exception.Message)
  exit 3
}
