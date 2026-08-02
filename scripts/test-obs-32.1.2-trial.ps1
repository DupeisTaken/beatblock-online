$ErrorActionPreference = 'Stop'
$scriptPath = Join-Path $PSScriptRoot 'run-obs-32.1.2-trial.ps1'
. $scriptPath -LibraryOnly

function Assert-Equal {
    param($Actual, $Expected, [string]$Message)
    if ($Actual -ne $Expected) { throw "$Message Expected '$Expected', got '$Actual'." }
}

function Set-UInt32 {
    param([byte[]]$Buffer, [int]$Offset, [uint32]$Value)
    [BitConverter]::GetBytes($Value).CopyTo($Buffer, $Offset)
}

function Set-UInt64 {
    param([byte[]]$Buffer, [int]$Offset, [uint64]$Value)
    [BitConverter]::GetBytes($Value).CopyTo($Buffer, $Offset)
}

$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('bbt-obs-trial-test-' + [guid]::NewGuid().ToString('N'))
try {
    New-Item -ItemType Directory -Path $temporaryRoot -Force | Out-Null

    $accepted = Assert-ExactObsVersion -VersionEvidence ([pscustomobject]@{ major = 32; minor = 1; patch = 2 }) -ExpectedVersion '32.1.2'
    Assert-Equal $accepted '32.1.2' 'Exact OBS version was not accepted.'
    $wrongVersionRejected = $false
    try {
        Assert-ExactObsVersion -VersionEvidence ([pscustomobject]@{ major = 32; minor = 0; patch = 4 }) -ExpectedVersion '32.1.2' | Out-Null
    } catch {
        $wrongVersionRejected = $_.Exception.Message -match 'No samples were taken'
    }
    Assert-Equal $wrongVersionRejected $true 'Wrong OBS version did not fail closed.'

    $portable = Join-Path $temporaryRoot 'portable obs'
    $obsExecutable = Join-Path $portable 'bin\64bit\obs64.exe'
    New-Item -ItemType Directory -Path (Split-Path -Parent $obsExecutable) -Force | Out-Null
    [System.IO.File]::WriteAllBytes($obsExecutable, [byte[]](1, 2, 3))
    Assert-Equal (Normalize-ObsDirectory -Candidate $portable) ([System.IO.Path]::GetFullPath($portable)) 'OBS root normalization failed.'
    Assert-Equal (Normalize-ObsDirectory -Candidate $obsExecutable) ([System.IO.Path]::GetFullPath($portable)) 'OBS executable normalization failed.'
    $versionSource = Get-Content -LiteralPath $scriptPath -Raw
    if ($versionSource -notmatch '\$version\.FileMajorPart' -or $versionSource -match 'major = \$version\.ProductMajorPart') {
        throw 'OBS exact-version preflight must use the executable numeric FileVersion tuple.'
    }

    $framePath = Join-Path $temporaryRoot 'stream-A.bbtframe'
    $header = New-Object byte[] 80
    [System.Text.Encoding]::ASCII.GetBytes('BBTFRAME').CopyTo($header, 0)
    Set-UInt32 $header 8 4
    Set-UInt32 $header 12 1280
    Set-UInt32 $header 16 720
    Set-UInt32 $header 20 5120
    Set-UInt32 $header 24 3
    Set-UInt32 $header 28 1
    Set-UInt64 $header 32 120
    Set-UInt64 $header 40 ([uint64](1280 * 720 * 4))
    Set-UInt64 $header 48 2
    Set-UInt64 $header 56 120
    [System.IO.File]::WriteAllBytes($framePath, $header)
    $frame = Get-RendererFrameHeader -Slot A -Path $framePath
    Assert-Equal $frame.status 'ok' 'Valid frame header was rejected.'
    Assert-Equal $frame.sequence ([uint64]120) 'Frame sequence was parsed incorrectly.'
    Assert-Equal $frame.droppedFrames ([uint64]2) 'Dropped-frame count was parsed incorrectly.'
    Set-UInt64 $header 56 0
    [System.IO.File]::WriteAllBytes($framePath, $header)
    Assert-Equal (Get-RendererFrameHeader -Slot A -Path $framePath).status 'changing' 'An invalidated frame slot was accepted.'
    Set-UInt64 $header 56 123
    [System.IO.File]::WriteAllBytes($framePath, $header)
    Assert-Equal (Get-RendererFrameHeader -Slot A -Path $framePath).status 'changing' 'A reused modulo slot was accepted for the old generation.'
    Assert-Equal (Get-RendererFrameHeader -Slot B -Path (Join-Path $temporaryRoot 'missing.bbtframe')).status 'missing' 'Missing frame ring was not reported.'

    $installer = Join-Path $temporaryRoot 'BeatblockOnlineInstaller.exe'
    $artifact = Join-Path $temporaryRoot 'beatblock-online-obs.dll'
    $installed = Join-Path $temporaryRoot 'installed\beatblock-online-obs.dll'
    $sourceRoot = Join-Path $temporaryRoot 'source-root'
    $source = Join-Path $sourceRoot 'obs-plugin\src\plugin.c'
    New-Item -ItemType Directory -Path (Split-Path -Parent $installed) -Force | Out-Null
    New-Item -ItemType Directory -Path (Split-Path -Parent $source) -Force | Out-Null
    [System.IO.File]::WriteAllText($installer, 'installer')
    [System.IO.File]::WriteAllText($artifact, 'plugin')
    [System.IO.File]::WriteAllText($source, 'source')
    Copy-Item -LiteralPath $artifact -Destination $installed
    $installerHash = Get-Sha256Lower -LiteralPath $installer
    $artifactHash = Get-Sha256Lower -LiteralPath $artifact
    $sourceHash = Get-Sha256Lower -LiteralPath $source
    $checksums = Join-Path $temporaryRoot 'SHA256SUMS.txt'
    [System.IO.File]::WriteAllText($checksums, "$installerHash  BeatblockOnlineInstaller.exe`n$artifactHash  beatblock-online-obs.dll`n")
    $manifest = [ordered]@{
        schemaVersion = 1
        obsVersion = '32.0.4'
        sourcePath = 'obs-plugin/src/plugin.c'
        sourceSha256 = $sourceHash
        artifactSha256 = $artifactHash
    } | ConvertTo-Json
    [System.IO.File]::WriteAllText([System.IO.Path]::ChangeExtension($artifact, '.build.json'), $manifest)
    $artifacts = Get-ArtifactPreflight -Installer $installer -PluginArtifact $artifact -InstalledPlugin $installed -Checksums $checksums -SourceRoot $sourceRoot
    Assert-Equal $artifacts.pluginArtifact.sha256 $artifactHash 'Artifact SHA was not captured.'
    Assert-Equal $artifacts.installedPlugin.sha256 $artifactHash 'Installed plugin SHA was not captured.'
    Assert-Equal $artifacts.pluginSource.sha256 $sourceHash 'Plugin source SHA was not captured.'

    [System.IO.File]::WriteAllText($source, 'stale source')
    $staleSourceRejected = $false
    try {
        Get-ArtifactPreflight -Installer $installer -PluginArtifact $artifact -InstalledPlugin $installed -Checksums $checksums -SourceRoot $sourceRoot | Out-Null
    } catch {
        $staleSourceRejected = $_.Exception.Message -match 'stale'
    }
    Assert-Equal $staleSourceRejected $true 'A plugin built from stale source was accepted.'
    [System.IO.File]::WriteAllText($source, 'source')

    [System.IO.File]::WriteAllText($installed, 'different')
    $mismatchRejected = $false
    try {
        Get-ArtifactPreflight -Installer $installer -PluginArtifact $artifact -InstalledPlugin $installed -Checksums $checksums | Out-Null
    } catch {
        $mismatchRejected = $_.Exception.Message -match 'does not match'
    }
    Assert-Equal $mismatchRejected $true 'A stale installed plugin was accepted.'

    $sample1 = [pscustomobject]@{
        timestampUtc = '2026-08-01T00:00:00.0000000Z'
        frames = @([pscustomobject]@{ slot = 'A'; status = 'ok'; sequence = [uint64]100; droppedFrames = [uint64]1 })
        processes = @([pscustomobject]@{ identity = '10:1'; role = 'obs'; pid = 10; cpuPercentOfMachine = $null; workingSetBytes = 100MB; privateBytes = 80MB })
    }
    $sample2 = [pscustomobject]@{
        timestampUtc = '2026-08-01T00:00:05.0000000Z'
        frames = @([pscustomobject]@{ slot = 'A'; status = 'ok'; sequence = [uint64]400; droppedFrames = [uint64]2 })
        processes = @([pscustomobject]@{ identity = '10:1'; role = 'obs'; pid = 10; cpuPercentOfMachine = 12.5; workingSetBytes = 120MB; privateBytes = 90MB })
    }
    $samples = @($sample1, $sample2)
    $frameSummary = @(Get-FrameSummaries -Samples $samples | Where-Object slot -eq A)[0]
    Assert-Equal $frameSummary.publishedFrames ([uint64]300) 'Published-frame summary is incorrect.'
    Assert-Equal $frameSummary.droppedFrames ([uint64]1) 'Dropped-frame summary is incorrect.'
    Assert-Equal $frameSummary.observedFps 60 'Observed FPS summary is incorrect.'
    $processSummary = @(Get-ProcessSummaries -Samples $samples)[0]
    Assert-Equal $processSummary.peakWorkingSetBytes ([int64](120MB)) 'Process peak memory summary is incorrect.'

    $reportDirectory = Join-Path $temporaryRoot 'reports'
    $fakeArtifacts = Get-ArtifactPreflight -Installer $installer -PluginArtifact $artifact -InstalledPlugin $artifact -Checksums $checksums -SourceRoot $sourceRoot
    $report = [pscustomobject][ordered]@{
        schemaVersion = 1
        issue = 28
        generatedAt = '2026-08-01T00:00:00Z'
        samplerStatus = 'EVIDENCE_CAPTURED'
        error = $null
        preflight = [pscustomobject]@{
            gitCommit = 'fixture'
            gitWorktreeClean = $true
            obs = [pscustomobject]@{ executable = $obsExecutable; exactVersion = '32.1.2' }
            artifacts = $fakeArtifacts
        }
        frameSummaries = @(Get-FrameSummaries -Samples $samples)
        processSummaries = @(Get-ProcessSummaries -Samples $samples)
        samples = $samples
    }
    $paths = Write-TrialEvidence -Report $report -Directory $reportDirectory
    if (-not (Test-Path -LiteralPath $paths.json) -or -not (Test-Path -LiteralPath $paths.markdown)) {
        throw 'Evidence files were not written.'
    }
    $json = Get-Content -LiteralPath $paths.json -Raw | ConvertFrom-Json
    Assert-Equal $json.issue 28 'JSON evidence lost the issue number.'
    $markdown = Get-Content -LiteralPath $paths.markdown -Raw
    if ($markdown -notmatch 'MANUAL REVIEW REQUIRED' -or $markdown -notmatch 'NOT RECORDED') {
        throw 'Markdown evidence could be mistaken for a completed physical gate.'
    }

    $scriptSource = Get-Content -LiteralPath $scriptPath -Raw
    if ($scriptSource -match '\b(Start-Process|Stop-Process|Set-Process|taskkill|SetMute)\b') {
        throw 'Passive sampler contains a process launch, termination, or mute operation.'
    }
    if ($scriptSource -notmatch 'DurationSeconds = 600\.0' -or $scriptSource -notmatch 'SampleIntervalSeconds = 5\.0') {
        throw 'Passive sampler defaults drifted from ten minutes / five seconds.'
    }

    Write-Host 'Validated OBS 32.1.2 passive sampler preflight, fixtures, summaries, and evidence output.'
} finally {
    $temp = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\', '/')
    $resolved = [System.IO.Path]::GetFullPath($temporaryRoot).TrimEnd('\', '/')
    if ($resolved.StartsWith($temp + [System.IO.Path]::DirectorySeparatorChar, [System.StringComparison]::OrdinalIgnoreCase)) {
        if (Test-Path -LiteralPath $resolved) { Remove-Item -LiteralPath $resolved -Recurse -Force }
    }
}
