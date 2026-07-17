$ErrorActionPreference = 'Stop'
$installer = Join-Path $PSScriptRoot 'install-mod.ps1'
$root = Join-Path ([System.IO.Path]::GetTempPath()) ('bbt-installer-test-' + [guid]::NewGuid().ToString('N'))
$game = Join-Path $root 'game'
$mods = Join-Path $root 'data\Beatblock\Mods'
$lovelySource = Join-Path $root 'lovely-source'
$lovelyArchive = Join-Path $root 'lovely-x86_64-pc-windows-msvc.zip'

try {
    New-Item -ItemType Directory -Path $game, $mods, $lovelySource -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $game 'Beatblock.exe') -Value 'test executable'
    Set-Content -LiteralPath (Join-Path $lovelySource 'version.dll') -Value 'test Lovely proxy'
    Compress-Archive -Path (Join-Path $lovelySource '*') -DestinationPath $lovelyArchive

    & $installer -GameDir $game -ModsDir $mods -Distribution standalone -LovelyArchive $lovelyArchive -AllowUnknownBuild
    if (-not (Test-Path -LiteralPath (Join-Path $game 'version.dll'))) {
        throw 'Lovely archive installation did not copy version.dll beside Beatblock.exe.'
    }
    if (-not (Test-Path -LiteralPath (Join-Path $mods 'BeatblockOnline\lovely\bootstrap.toml'))) {
        throw 'Standalone installation did not create the expected bootstrap path.'
    }
    & $installer -GameDir $game -ModsDir $mods -Uninstall
    if (Test-Path -LiteralPath (Join-Path $mods 'BeatblockOnline')) {
        throw 'Standalone uninstall left the target directory behind.'
    }

    $bbp = Join-Path $mods 'BeatblockPlus'
    New-Item -ItemType Directory -Path $bbp -Force | Out-Null
    Set-Content -LiteralPath (Join-Path $bbp 'mod.json') -Value '{"id":"beatblock-plus","version":"2.1.0"}'
    & $installer -GameDir $game -ModsDir $mods -Distribution beatblock-plus -AllowUnknownBuild
    if (-not (Test-Path -LiteralPath (Join-Path $mods 'BeatblockOnline\mod.json'))) {
        throw 'BeatblockPlus installation did not create the expected manifest path.'
    }

    $duplicateRejected = $false
    try {
        & $installer -GameDir $game -ModsDir $mods -Distribution beatblock-plus -AllowUnknownBuild
    } catch {
        $duplicateRejected = $_.Exception.Message -like '*already exists*'
    }
    if (-not $duplicateRejected) { throw 'Installer did not reject a duplicate installation.' }

    & $installer -GameDir $game -ModsDir $mods -Uninstall
    Write-Host 'Validated standalone install/uninstall, BeatblockPlus install/uninstall, and duplicate rejection.'
} finally {
    $temp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\', '/')
    $resolved = [System.IO.Path]::GetFullPath($root).TrimEnd('\', '/')
    if ($resolved.StartsWith($temp + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
        if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
    }
}
