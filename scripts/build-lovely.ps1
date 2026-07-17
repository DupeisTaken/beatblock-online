param(
    [string]$OutputPath,
    [string]$SourceDirectory
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot 'release-utils.ps1')
$tag = 'v0.9.0'
$commit = '91759da5702618c3b940fbbe8135954414c0ef34'
$toolchain = 'nightly-2026-07-15'
$toolchainCommit = 'da80ed0708a09dc096c184345d6eb42cbcd50a1e'
$repository = 'https://github.com/ethangreen-dev/lovely-injector.git'
$patch = Join-Path $root 'third-party\lovely-no-console.patch'

if (-not $OutputPath) {
    $OutputPath = Join-Path $root 'artifacts\lovely\version.dll'
}
if (-not $SourceDirectory) {
    $SourceDirectory = Join-Path $root ".tools\lovely-injector-$tag"
}

if (-not (Test-Path -LiteralPath $SourceDirectory -PathType Container)) {
    New-Item -ItemType Directory -Force (Split-Path -Parent $SourceDirectory) | Out-Null
    git clone --branch $tag --depth 1 $repository $SourceDirectory
    if ($LASTEXITCODE -ne 0) { throw 'Failed to clone the pinned Lovely source.' }
}

$actualCommit = (git -C $SourceDirectory rev-parse HEAD).Trim()
if ($LASTEXITCODE -ne 0 -or $actualCommit -ne $commit) {
    throw "Lovely source is not the pinned commit. Expected $commit, got $actualCommit."
}

# Keep the build cache reusable while proving that it contains exactly our
# reviewed delta. Apply once, or accept an already-applied patch.
$savedErrorPreference = $ErrorActionPreference
$ErrorActionPreference = 'Continue'
git -C $SourceDirectory apply --check $patch 2>$null
$applyCheck = $LASTEXITCODE
$ErrorActionPreference = $savedErrorPreference
if ($applyCheck -eq 0) {
    git -C $SourceDirectory apply $patch
    if ($LASTEXITCODE -ne 0) { throw 'Failed to apply the Lovely release patch.' }
} else {
    $ErrorActionPreference = 'Continue'
    git -C $SourceDirectory apply --reverse --check $patch 2>$null
    $reverseCheck = $LASTEXITCODE
    $ErrorActionPreference = $savedErrorPreference
    if ($reverseCheck -ne 0) {
        throw 'The cached Lovely source contains changes other than the reviewed patch.'
    }
}

$expectedPatch = (Get-Content -LiteralPath $patch -Raw).Replace("`r`n", "`n").Trim()
$actualPatch = ((git -C $SourceDirectory diff --no-ext-diff) -join "`n").Trim()
if ($LASTEXITCODE -ne 0 -or $actualPatch -ne $expectedPatch) {
    throw 'The cached Lovely source does not exactly match the reviewed release patch.'
}

$vswhere = 'C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe'
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
    throw 'Visual Studio Build Tools were not found.'
}
$visualStudio = & $vswhere -latest -products * `
    -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
    -property installationPath
if (-not $visualStudio) { throw 'The Visual C++ x64 toolchain is not installed.' }
$devCommand = Join-Path $visualStudio 'Common7\Tools\VsDevCmd.bat'
$cargoToolchain = $toolchain
$ErrorActionPreference = 'Continue'
$rustVersion = (& rustc "+$toolchain" --version --verbose 2>$null) -join "`n"
$pinnedToolchainAvailable = $LASTEXITCODE -eq 0
$ErrorActionPreference = $savedErrorPreference
if (-not $pinnedToolchainAvailable) {
    # A rustup `nightly` alias is acceptable only when it resolves to the same
    # compiler commit. This avoids a redundant local install without allowing
    # the release compiler to drift.
    $rustVersion = (& rustc +nightly --version --verbose) -join "`n"
    if ($LASTEXITCODE -ne 0 -or $rustVersion -notmatch [regex]::Escape($toolchainCommit)) {
        throw "Install Rust $toolchain (compiler commit $toolchainCommit) before building Lovely."
    }
    $cargoToolchain = 'nightly'
}
$buildCommand = Join-Path $root '.tools\build-lovely.cmd'
@"
@echo off
call "$devCommand" -arch=x64 >nul
if errorlevel 1 exit /b %errorlevel%
cargo +$cargoToolchain build --manifest-path "$SourceDirectory\Cargo.toml" --release -p lovely-win --locked
"@ | Set-Content -Encoding ascii $buildCommand
cmd.exe /d /c $buildCommand
if ($LASTEXITCODE -ne 0) { throw 'Lovely release build failed.' }

$built = Join-Path $SourceDirectory 'target\release\version.dll'
if (-not (Test-Path -LiteralPath $built -PathType Leaf)) {
    throw "Lovely build did not produce $built."
}
New-Item -ItemType Directory -Force (Split-Path -Parent $OutputPath) | Out-Null
Copy-Item -LiteralPath $built -Destination $OutputPath -Force
Write-Host "Built $OutputPath"
Write-Host "SHA-256 $(Get-Sha256Hex -LiteralPath $OutputPath)"
