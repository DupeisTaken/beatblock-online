[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = 'Medium')]
param(
    [Parameter(Mandatory = $true)]
    [string] $GameDir,

    [ValidateSet('standalone', 'beatblock-plus')]
    [string] $Distribution = 'standalone',

    [string] $ModsDir,
    [string] $LovelyArchive,
    [switch] $AllowUnknownBuild,
    [switch] $Force,
    [switch] $Uninstall
)

$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$fixturePath = Join-Path $repoRoot 'mod\fixtures\patch-signatures.json'

function Get-FullPath([string] $Path) {
    return [System.IO.Path]::GetFullPath($Path).TrimEnd('\', '/')
}

function Assert-ChildPath([string] $Parent, [string] $Child) {
    $parentFull = Get-FullPath $Parent
    $childFull = Get-FullPath $Child
    $prefix = $parentFull + [System.IO.Path]::DirectorySeparatorChar
    if (-not $childFull.StartsWith($prefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to modify '$childFull' because it is not inside '$parentFull'."
    }
}

function Get-BeatblockPlusFolder([string] $Root, [string] $IgnoredFolder) {
    if (-not (Test-Path -LiteralPath $Root -PathType Container)) { return $null }
    foreach ($manifest in Get-ChildItem -LiteralPath $Root -Filter mod.json -File -Recurse -Depth 1) {
        if ($manifest.Directory.FullName -eq $IgnoredFolder) { continue }
        try {
            $data = Get-Content -LiteralPath $manifest.FullName -Raw | ConvertFrom-Json
            if ($data.id -eq 'beatblock-plus') { return $manifest.Directory.FullName }
        } catch {
            continue
        }
    }
    return $null
}

$gameRoot = Get-FullPath $GameDir
if ([string]::IsNullOrWhiteSpace($ModsDir)) {
    if ([string]::IsNullOrWhiteSpace($env:APPDATA)) {
        throw 'APPDATA is unavailable. Pass -ModsDir explicitly.'
    }
    $ModsDir = Join-Path $env:APPDATA 'Beatblock\Mods'
}
$modsRoot = Get-FullPath $ModsDir
$target = Join-Path $modsRoot 'BeatblockOnline'
Assert-ChildPath $modsRoot $target

if ($Uninstall) {
    if (-not (Test-Path -LiteralPath $target -PathType Container)) {
        Write-Host "Beatblock Online is not installed at $target"
        exit 0
    }
    $standaloneMarker = Test-Path -LiteralPath (Join-Path $target 'lovely\bootstrap.toml') -PathType Leaf
    $plusMarker = Test-Path -LiteralPath (Join-Path $target 'mod.json') -PathType Leaf
    $hooksMarker = Test-Path -LiteralPath (Join-Path $target 'lovely\hooks.toml') -PathType Leaf
    if (-not $hooksMarker -or (-not $standaloneMarker -and -not $plusMarker)) {
        throw "Refusing to remove '$target': it does not look like a Beatblock Online installation."
    }
    if ($PSCmdlet.ShouldProcess($target, 'Remove Beatblock Online mod folder')) {
        Remove-Item -LiteralPath $target -Recurse -Force
        Write-Host 'Beatblock Online was removed. Lovely version.dll was kept because other mods may use it.'
    }
    exit 0
}

$exe = Join-Path $gameRoot 'Beatblock.exe'
if (-not (Test-Path -LiteralPath $exe -PathType Leaf)) {
    throw "Beatblock.exe was not found in '$gameRoot'. In Steam use Manage > Browse local files and pass that folder."
}

$fixture = Get-Content -LiteralPath $fixturePath -Raw | ConvertFrom-Json
$expectedHash = $fixture.reference.beatblockExeSha256
$savedWhatIfPreference = $WhatIfPreference
try {
    $WhatIfPreference = $false
    $actualHash = (Get-FileHash -LiteralPath $exe -Algorithm SHA256).Hash.ToLowerInvariant()
} finally {
    $WhatIfPreference = $savedWhatIfPreference
}
if ($actualHash -ne $expectedHash -and -not $AllowUnknownBuild) {
    throw "Unsupported Beatblock.exe ($actualHash). Expected $expectedHash. Use -AllowUnknownBuild only for non-competitive development."
}
if ($actualHash -ne $expectedHash) {
    Write-Warning 'Installing onto an unknown Beatblock build. Competitive races will remain blocked.'
}

$versionDll = Join-Path $gameRoot 'version.dll'
if (-not (Test-Path -LiteralPath $versionDll -PathType Leaf)) {
    if ([string]::IsNullOrWhiteSpace($LovelyArchive)) {
        $lovelyMessage = "Lovely is not installed. Download lovely-x86_64-pc-windows-msvc.zip from https://github.com/ethangreen-dev/lovely-injector/releases/latest and pass it with -LovelyArchive."
        if ($WhatIfPreference) {
            Write-Warning $lovelyMessage
        } else {
            throw $lovelyMessage
        }
    }
    if (-not [string]::IsNullOrWhiteSpace($LovelyArchive)) {
        $archive = Get-FullPath $LovelyArchive
        if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) {
            throw "Lovely archive was not found at '$archive'."
        }
        $tempRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('bbt-lovely-' + [guid]::NewGuid().ToString('N'))
        try {
            New-Item -ItemType Directory -Path $tempRoot -WhatIf:$false | Out-Null
            Expand-Archive -LiteralPath $archive -DestinationPath $tempRoot -WhatIf:$false
            $candidate = Get-ChildItem -LiteralPath $tempRoot -Filter version.dll -File -Recurse | Select-Object -First 1
            if (-not $candidate) { throw "The archive '$archive' does not contain version.dll." }
            if ($PSCmdlet.ShouldProcess($versionDll, 'Install Lovely runtime injector')) {
                Copy-Item -LiteralPath $candidate.FullName -Destination $versionDll
            }
        } finally {
            $safeTemp = Get-FullPath ([System.IO.Path]::GetTempPath())
            Assert-ChildPath $safeTemp $tempRoot
            if (Test-Path -LiteralPath $tempRoot) { Remove-Item -LiteralPath $tempRoot -Recurse -Force -WhatIf:$false }
        }
    }
}

$source = Join-Path $repoRoot ("mod\" + $Distribution)
if (-not (Test-Path -LiteralPath $source -PathType Container)) {
    throw "Distribution source was not found at '$source'. Run this script from a complete repository checkout."
}

$beatblockPlus = Get-BeatblockPlusFolder $modsRoot $target
if ($Distribution -eq 'beatblock-plus' -and -not $beatblockPlus) {
    throw "BeatblockPlus 2.x was not found in '$modsRoot'. Install BeatblockPlus before this distribution."
}
if ($Distribution -eq 'standalone' -and $beatblockPlus) {
    throw "BeatblockPlus is installed at '$beatblockPlus'. Use -Distribution beatblock-plus instead."
}

New-Item -ItemType Directory -Path $modsRoot -Force | Out-Null
if (Test-Path -LiteralPath $target) {
    if (-not $Force) {
        throw "'$target' already exists. Uninstall it first or pass -Force to replace it with a backed-up copy."
    }
    $backupRoot = Join-Path (Split-Path -Parent $modsRoot) 'BeatblockOnline-backups'
    $backup = Join-Path $backupRoot (Get-Date -Format 'yyyyMMdd-HHmmss')
    if ($PSCmdlet.ShouldProcess($target, "Move existing installation to $backup")) {
        New-Item -ItemType Directory -Path $backupRoot -Force | Out-Null
        Move-Item -LiteralPath $target -Destination $backup
        Write-Host "Previous installation backed up to $backup"
    }
}

if ($PSCmdlet.ShouldProcess($target, "Install Beatblock Online $Distribution distribution")) {
    Copy-Item -LiteralPath $source -Destination $target -Recurse
    Write-Host "Installed Beatblock Online ($Distribution) to $target"
    Write-Host "Lovely runtime: $versionDll"
    Write-Host 'Launch Beatblock through Steam and select Online; the hidden runtime starts on demand.'
}
