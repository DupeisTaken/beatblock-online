$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-utils.ps1')

$root = Join-Path ([System.IO.Path]::GetTempPath()) ('bbt-release-utils-' + [guid]::NewGuid().ToString('N'))
$fixture = Join-Path $root 'fixture.bin'
try {
    New-Item -ItemType Directory -Path $root -Force | Out-Null
    [System.IO.File]::WriteAllText($fixture, 'release', [System.Text.UTF8Encoding]::new($false))
    $actual = Get-Sha256Hex -LiteralPath $fixture
    $expected = 'A4D451EC23463726F72C43D64C710968F6B602CD653B4DE8ADEE1B556240A829'
    if ($actual -ne $expected) {
        throw "SHA-256 helper returned $actual, expected $expected."
    }
    Write-Host 'Validated the release SHA-256 helper.'
} finally {
    $temp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\', '/')
    $resolved = [System.IO.Path]::GetFullPath($root).TrimEnd('\', '/')
    if ($resolved.StartsWith($temp + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
        if (Test-Path -LiteralPath $resolved) {
            Remove-Item -LiteralPath $resolved -Recurse -Force
        }
    }
}
