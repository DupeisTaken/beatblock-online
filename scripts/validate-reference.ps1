param([string]$ReferenceRoot = (Join-Path $PSScriptRoot '..\.reference\Beatblock'))
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.IO.Compression.FileSystem
$fixture = Get-Content (Join-Path $PSScriptRoot '..\mod\fixtures\patch-signatures.json') -Raw | ConvertFrom-Json

function Assert-Hash([string]$Path, [string]$Expected) {
  $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $Expected) { throw "Reference hash mismatch for $Path. Expected $Expected, got $actual" }
}
function Read-ZipEntry([string]$Path, [string]$Suffix) {
  $archive = [IO.Compression.ZipFile]::OpenRead($Path)
  try {
    $entry = $archive.Entries | Where-Object { $_.FullName.EndsWith($Suffix) } | Select-Object -First 1
    if (-not $entry) { throw "Missing $Suffix in $Path" }
    $reader = [IO.StreamReader]::new($entry.Open())
    try { $reader.ReadToEnd() } finally { $reader.Dispose() }
  } finally { $archive.Dispose() }
}

Assert-Hash (Join-Path $ReferenceRoot 'Beatblock.exe') $fixture.reference.beatblockExeSha256
Assert-Hash (Join-Path $ReferenceRoot 'packed\obj.zip') $fixture.reference.objectArchiveSha256
Assert-Hash (Join-Path $ReferenceRoot 'packed\states.zip') $fixture.reference.stateArchiveSha256
$game = Read-ZipEntry (Join-Path $ReferenceRoot 'packed\states.zip') 'Game.lua'
$manager = Read-ZipEntry (Join-Path $ReferenceRoot 'packed\obj.zip') 'GameManager.lua'
foreach ($pattern in $fixture.patchPatterns) { if (-not $game.Contains($pattern) -and -not $pattern.Contains('mainMenu')) { throw "Game.lua no longer contains patch signature: $pattern" } }
foreach ($pattern in $fixture.gameManagerHooks) { if (-not $manager.Contains($pattern)) { throw "GameManager.lua no longer contains hook signature: $pattern" } }
Write-Host 'Pinned Beatblock reference and gameplay hook signatures validated.'
