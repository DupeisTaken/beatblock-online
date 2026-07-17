param(
    [string]$ObsDirectory
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$lovely = Join-Path $root 'artifacts\lovely\version.dll'
$obs = Join-Path $root 'artifacts\obs\beatblock-online-obs.dll'

function Invoke-Checked {
    param([scriptblock]$Command, [string]$Failure)
    & $Command
    if ($LASTEXITCODE -ne 0) { throw $Failure }
}

Invoke-Checked { pnpm --filter @bbt/protocol build } 'Protocol build failed.'
& (Join-Path $PSScriptRoot 'build-lovely.ps1') -OutputPath $lovely

$obsArguments = @{ OutputPath = $obs }
if ($ObsDirectory) { $obsArguments.ObsDirectory = $ObsDirectory }
& (Join-Path $PSScriptRoot 'build-obs-plugin.ps1') @obsArguments

$previousLovely = $env:BBT_LOVELY_DLL
$previousObs = $env:BBT_OBS_PLUGIN_DLL
try {
    $env:BBT_LOVELY_DLL = $lovely
    $env:BBT_OBS_PLUGIN_DLL = $obs
    Invoke-Checked { node (Join-Path $PSScriptRoot 'build-windows.mjs') } 'Windows installer build failed.'
} finally {
    $env:BBT_LOVELY_DLL = $previousLovely
    $env:BBT_OBS_PLUGIN_DLL = $previousObs
}

Invoke-Checked { node (Join-Path $PSScriptRoot 'package-mods.mjs') } 'Mod packaging failed.'
Invoke-Checked { node (Join-Path $PSScriptRoot 'verify-release.mjs') } 'Release verification failed.'
Write-Host 'Release outputs are ready under release/, artifacts/, and mod/releases/.'
