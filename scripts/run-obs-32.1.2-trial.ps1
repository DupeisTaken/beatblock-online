[CmdletBinding()]
param(
    [string]$ObsDirectory,
    [ValidateRange(0.05, 3600.0)]
    [double]$DurationSeconds = 600.0,
    [ValidateRange(0.01, 60.0)]
    [double]$SampleIntervalSeconds = 5.0,
    [string]$RendererDirectory,
    [string]$InstallerPath,
    [string]$ObsPluginArtifactPath,
    [string]$InstalledObsPluginPath,
    [string]$ReleaseChecksumsPath,
    [string]$OutputDirectory,
    [switch]$LibraryOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$script:RepositoryRoot = Split-Path -Parent $PSScriptRoot
$script:RequiredObsVersion = '32.1.2'
$script:ExpectedFrameMagic = 'BBTFRAME'
$script:ExpectedFrameVersion = 4
$script:ExpectedPixelEncoding = 1
$script:FrameHeaderSize = 80
$script:FrameSlotSequenceOffset = 56
$script:MaximumFrameCount = 3

function Get-Sha256Lower {
    param([Parameter(Mandatory = $true)][string]$LiteralPath)

    if (-not (Test-Path -LiteralPath $LiteralPath -PathType Leaf)) {
        throw "Required evidence file is missing: $LiteralPath"
    }
    return (Get-FileHash -LiteralPath $LiteralPath -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Normalize-ObsDirectory {
    param([Parameter(Mandatory = $true)][string]$Candidate)

    $path = [System.IO.Path]::GetFullPath($Candidate.Trim().Trim('"'))
    if ([System.IO.Path]::GetFileName($path) -ieq 'obs64.exe') {
        $path = Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $path))
    } elseif ([System.IO.Path]::GetFileName($path) -ieq '64bit') {
        $path = Split-Path -Parent (Split-Path -Parent $path)
    } elseif ([System.IO.Path]::GetFileName($path) -ieq 'bin') {
        $path = Split-Path -Parent $path
    }

    $executable = Join-Path $path 'bin\64bit\obs64.exe'
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "OBS root must contain bin\64bit\obs64.exe: $path"
    }
    return [System.IO.Path]::GetFullPath($path)
}

function Find-ObsDirectory {
    param([string]$ExplicitDirectory)

    if ($ExplicitDirectory) {
        return Normalize-ObsDirectory -Candidate $ExplicitDirectory
    }

    $candidates = @()
    if ($env:BBT_OBS_DIR) { $candidates += $env:BBT_OBS_DIR }
    if ($env:ProgramFiles) { $candidates += (Join-Path $env:ProgramFiles 'obs-studio') }
    if (${env:ProgramFiles(x86)}) { $candidates += (Join-Path ${env:ProgramFiles(x86)} 'obs-studio') }
    foreach ($candidate in $candidates | Select-Object -Unique) {
        try {
            return Normalize-ObsDirectory -Candidate $candidate
        } catch {
            # Discovery is best-effort. An explicit path, by contrast, fails above.
        }
    }
    throw 'OBS Studio was not found. Pass -ObsDirectory with the OBS root or obs64.exe path.'
}

function Get-ObsVersionEvidence {
    param([Parameter(Mandatory = $true)][string]$ExecutablePath)

    $version = [System.Diagnostics.FileVersionInfo]::GetVersionInfo($ExecutablePath)
    # OBS 32.1.2's ProductVersion string is populated, but its numeric product
    # tuple is 0.0.0. The signed executable's numeric FileVersion tuple is the
    # stable machine-readable 32.1.2 value used by Explorer and `obs64 --version`.
    return [pscustomobject][ordered]@{
        productVersion = $version.ProductVersion
        fileVersion = $version.FileVersion
        major = $version.FileMajorPart
        minor = $version.FileMinorPart
        patch = $version.FileBuildPart
        revision = $version.FilePrivatePart
    }
}

function Assert-ExactObsVersion {
    param(
        [Parameter(Mandatory = $true)]$VersionEvidence,
        [Parameter(Mandatory = $true)][string]$ExpectedVersion
    )

    if ($ExpectedVersion -notmatch '^(\d+)\.(\d+)\.(\d+)$') {
        throw "Expected OBS version must be major.minor.patch, got: $ExpectedVersion"
    }
    $expected = @([int]$Matches[1], [int]$Matches[2], [int]$Matches[3])
    $actual = @(
        [int]$VersionEvidence.major,
        [int]$VersionEvidence.minor,
        [int]$VersionEvidence.patch
    )
    $actualText = $actual -join '.'
    if (($actual -join '.') -ne ($expected -join '.')) {
        throw "OBS Studio $ExpectedVersion x64 is required for issue #28; found $actualText. No samples were taken."
    }
    return $actualText
}

function Get-FileEvidence {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string]$LiteralPath
    )

    $item = Get-Item -LiteralPath $LiteralPath -ErrorAction Stop
    return [pscustomobject][ordered]@{
        name = $Name
        path = $item.FullName
        bytes = $item.Length
        lastWriteUtc = $item.LastWriteTimeUtc.ToString('o')
        sha256 = Get-Sha256Lower -LiteralPath $item.FullName
    }
}

function Read-ReleaseChecksums {
    param([Parameter(Mandatory = $true)][string]$LiteralPath)

    if (-not (Test-Path -LiteralPath $LiteralPath -PathType Leaf)) {
        throw "Release checksum file is missing: $LiteralPath"
    }
    $entries = @{}
    foreach ($line in Get-Content -LiteralPath $LiteralPath) {
        if ($line -match '^([0-9a-fA-F]{64})\s+(.+)$') {
            $entries[[System.IO.Path]::GetFileName($Matches[2].Trim())] = $Matches[1].ToLowerInvariant()
        }
    }
    return $entries
}

function Get-ArtifactPreflight {
    param(
        [Parameter(Mandatory = $true)][string]$Installer,
        [Parameter(Mandatory = $true)][string]$PluginArtifact,
        [Parameter(Mandatory = $true)][string]$InstalledPlugin,
        [Parameter(Mandatory = $true)][string]$Checksums,
        [string]$SourceRoot = $script:RepositoryRoot
    )

    $installerEvidence = Get-FileEvidence -Name 'Beatblock Online installer' -LiteralPath $Installer
    $artifactEvidence = Get-FileEvidence -Name 'OBS plugin build artifact' -LiteralPath $PluginArtifact
    $installedEvidence = Get-FileEvidence -Name 'Installed OBS plugin' -LiteralPath $InstalledPlugin
    if ($artifactEvidence.sha256 -ne $installedEvidence.sha256) {
        throw "Installed OBS plugin hash does not match the current build artifact. Close OBS and install the current release before sampling."
    }

    $checksumEntries = Read-ReleaseChecksums -LiteralPath $Checksums
    foreach ($evidence in @($installerEvidence, $artifactEvidence)) {
        $fileName = [System.IO.Path]::GetFileName($evidence.path)
        if (-not $checksumEntries.ContainsKey($fileName)) {
            throw "Release checksum file has no entry for $fileName."
        }
        if ($checksumEntries[$fileName] -ne $evidence.sha256) {
            throw "Release checksum mismatch for $fileName."
        }
    }

    $manifestPath = [System.IO.Path]::ChangeExtension($PluginArtifact, '.build.json')
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw "OBS plugin build manifest is missing: $manifestPath"
    }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.schemaVersion -ne 1 -or $manifest.artifactSha256 -ne $artifactEvidence.sha256) {
        throw 'OBS plugin build manifest does not match the current plugin artifact.'
    }
    if ([string]::IsNullOrWhiteSpace($manifest.sourcePath) -or [System.IO.Path]::IsPathRooted($manifest.sourcePath)) {
        throw 'OBS plugin build manifest contains an invalid source path.'
    }
    $resolvedSourceRoot = [System.IO.Path]::GetFullPath($SourceRoot).TrimEnd('\', '/')
    $sourcePath = [System.IO.Path]::GetFullPath((Join-Path $resolvedSourceRoot $manifest.sourcePath))
    $sourcePrefix = $resolvedSourceRoot + [System.IO.Path]::DirectorySeparatorChar
    if (-not $sourcePath.StartsWith($sourcePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
        throw 'OBS plugin build manifest source escapes the selected source root.'
    }
    $sourceEvidence = Get-FileEvidence -Name 'OBS plugin source' -LiteralPath $sourcePath
    if ($manifest.sourceSha256 -ne $sourceEvidence.sha256) {
        throw 'OBS plugin build manifest is stale for the current plugin source.'
    }

    return [pscustomobject][ordered]@{
        installer = $installerEvidence
        pluginArtifact = $artifactEvidence
        installedPlugin = $installedEvidence
        pluginSource = $sourceEvidence
        releaseChecksums = Get-FileEvidence -Name 'Release checksums' -LiteralPath $Checksums
        pluginBuildManifest = [pscustomobject][ordered]@{
            path = [System.IO.Path]::GetFullPath($manifestPath)
            obsVersion = $manifest.obsVersion
            sourcePath = $manifest.sourcePath
            sourceSha256 = $manifest.sourceSha256
            artifactSha256 = $manifest.artifactSha256
        }
    }
}

function Read-ExactBytes {
    param(
        [Parameter(Mandatory = $true)][System.IO.Stream]$Stream,
        [Parameter(Mandatory = $true)][int]$Count
    )

    $buffer = New-Object byte[] $Count
    $offset = 0
    while ($offset -lt $Count) {
        $read = $Stream.Read($buffer, $offset, $Count - $offset)
        if ($read -eq 0) { break }
        $offset += $read
    }
    if ($offset -ne $Count) { return $null }
    return $buffer
}

function Convert-FrameHeader {
    param(
        [Parameter(Mandatory = $true)][string]$Slot,
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][byte[]]$Header
    )

    if ($Header.Length -lt $script:FrameHeaderSize) {
        return [pscustomobject][ordered]@{ slot = $Slot; path = $Path; status = 'short' }
    }
    $magic = [System.Text.Encoding]::ASCII.GetString($Header, 0, 8)
    if ($magic -ne $script:ExpectedFrameMagic) {
        return [pscustomobject][ordered]@{ slot = $Slot; path = $Path; status = 'invalid_magic' }
    }

    $version = [BitConverter]::ToUInt32($Header, 8)
    $encoding = [BitConverter]::ToUInt32($Header, 28)
    $frameCount = [BitConverter]::ToUInt32($Header, 24)
    $sequence = [BitConverter]::ToUInt64($Header, 32)
    $status = 'ok'
    if ($version -ne $script:ExpectedFrameVersion -or $encoding -ne $script:ExpectedPixelEncoding) {
        $status = 'unsupported'
    } elseif ($frameCount -lt 1 -or $frameCount -gt $script:MaximumFrameCount -or $sequence -eq 0) {
        $status = 'invalid_metadata'
    } else {
        $slotOffset = $script:FrameSlotSequenceOffset + ($sequence % $frameCount) * 8
        if ([BitConverter]::ToUInt64($Header, $slotOffset) -ne $sequence) {
            $status = 'changing'
        }
    }
    return [pscustomobject][ordered]@{
        slot = $Slot
        path = $Path
        status = $status
        version = $version
        width = [BitConverter]::ToUInt32($Header, 12)
        height = [BitConverter]::ToUInt32($Header, 16)
        stride = [BitConverter]::ToUInt32($Header, 20)
        frameCount = $frameCount
        pixelEncoding = $encoding
        sequence = $sequence
        frameSize = [BitConverter]::ToUInt64($Header, 40)
        droppedFrames = [BitConverter]::ToUInt64($Header, 48)
    }
}

function Get-RendererFrameHeader {
    param(
        [Parameter(Mandatory = $true)][string]$Slot,
        [Parameter(Mandatory = $true)][string]$Path
    )

    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        return [pscustomobject][ordered]@{ slot = $Slot; path = $Path; status = 'missing' }
    }

    try {
        $share = [System.IO.FileShare]::ReadWrite -bor [System.IO.FileShare]::Delete
        $stream = New-Object System.IO.FileStream(
            $Path,
            [System.IO.FileMode]::Open,
            [System.IO.FileAccess]::Read,
            $share
        )
        try {
            # Header v4 publishes both a global generation and one generation
            # per modulo slot. Two matching complete snapshots prevent an
            # evidence row from accepting an in-progress N/N+3 slot reuse.
            for ($attempt = 0; $attempt -lt 3; $attempt++) {
                $stream.Position = 0
                $first = Read-ExactBytes -Stream $stream -Count $script:FrameHeaderSize
                if ($null -eq $first) {
                    return [pscustomobject][ordered]@{ slot = $Slot; path = $Path; status = 'short' }
                }
                $stream.Position = 0
                $second = Read-ExactBytes -Stream $stream -Count $script:FrameHeaderSize
                if ($null -eq $second) {
                    return [pscustomobject][ordered]@{ slot = $Slot; path = $Path; status = 'short' }
                }
                $firstFrame = Convert-FrameHeader -Slot $Slot -Path $Path -Header $first
                $secondFrame = Convert-FrameHeader -Slot $Slot -Path $Path -Header $second
                if ($firstFrame.status -eq 'ok' -and $secondFrame.status -eq 'ok' -and
                    $firstFrame.sequence -eq $secondFrame.sequence) {
                    return $secondFrame
                }
                if ($secondFrame.status -notin @('ok', 'changing')) {
                    return $secondFrame
                }
            }
            return [pscustomobject][ordered]@{ slot = $Slot; path = $Path; status = 'changing' }
        } finally {
            $stream.Dispose()
        }
    } catch {
        return [pscustomobject][ordered]@{
            slot = $Slot
            path = $Path
            status = 'unreadable'
            error = $_.Exception.Message
        }
    }
}

function Get-ProcessRole {
    param([Parameter(Mandatory = $true)]$Process)

    if ($Process.ProcessName -ieq 'obs64') { return 'obs' }
    if ($Process.ProcessName -ieq 'BeatblockOnlineRuntime') { return 'runtime' }
    if ($Process.MainWindowTitle -match '^Beatblock Online Renderer ([A-D])$') {
        return "renderer-$($Matches[1])"
    }
    if ($Process.MainWindowTitle -eq 'Beatblock Online Autoplay') { return 'autoplay' }
    return 'host-or-other-beatblock'
}

function Get-TrialProcessSamples {
    param(
        [Parameter(Mandatory = $true)][datetime]$TimestampUtc,
        [Parameter(Mandatory = $true)][hashtable]$PreviousCpu,
        [Parameter(Mandatory = $true)][int]$LogicalProcessors
    )

    $processes = Get-Process -Name 'obs64', 'BeatblockOnlineRuntime', 'Beatblock' -ErrorAction SilentlyContinue
    $samples = @()
    foreach ($process in $processes) {
        try { $started = $process.StartTime.ToUniversalTime() } catch { $started = [datetime]::MinValue }
        try { $cpuSeconds = [double]$process.CPU } catch { $cpuSeconds = 0.0 }
        try { $title = [string]$process.MainWindowTitle } catch { $title = '' }
        $identity = "$($process.Id):$($started.Ticks)"
        $cpuPercent = $null
        if ($PreviousCpu.ContainsKey($identity)) {
            $previous = $PreviousCpu[$identity]
            $elapsed = ($TimestampUtc - $previous.timestampUtc).TotalSeconds
            if ($elapsed -gt 0) {
                $cpuPercent = [math]::Round(
                    [math]::Max(0.0, ($cpuSeconds - $previous.cpuSeconds) / $elapsed / $LogicalProcessors * 100.0),
                    3
                )
            }
        }
        $PreviousCpu[$identity] = @{ timestampUtc = $TimestampUtc; cpuSeconds = $cpuSeconds }
        $shape = [pscustomobject]@{
            ProcessName = $process.ProcessName
            MainWindowTitle = $title
        }
        $samples += [pscustomobject][ordered]@{
            identity = $identity
            role = Get-ProcessRole -Process $shape
            pid = $process.Id
            processName = $process.ProcessName
            windowTitle = $title
            startedUtc = if ($started -eq [datetime]::MinValue) { $null } else { $started.ToString('o') }
            cpuSeconds = [math]::Round($cpuSeconds, 3)
            cpuPercentOfMachine = $cpuPercent
            workingSetBytes = [int64]$process.WorkingSet64
            privateBytes = [int64]$process.PrivateMemorySize64
        }
    }
    return $samples
}

function Get-FrameSummaries {
    param([Parameter(Mandatory = $true)][object[]]$Samples)

    $summaries = @()
    foreach ($slot in @('A', 'B', 'C', 'D')) {
        $valid = @()
        foreach ($sample in $Samples) {
            $frame = @($sample.frames | Where-Object { $_.slot -eq $slot }) | Select-Object -First 1
            if ($null -ne $frame -and $frame.status -eq 'ok') {
                $valid += [pscustomobject]@{
                    timestampUtc = [datetime]$sample.timestampUtc
                    sequence = [uint64]$frame.sequence
                    droppedFrames = [uint64]$frame.droppedFrames
                }
            }
        }
        [uint64]$published = 0
        [uint64]$dropped = 0
        $observedSeconds = 0.0
        $resets = 0
        $stagnantSeconds = 0.0
        $maxStagnantSeconds = 0.0
        for ($index = 1; $index -lt $valid.Count; $index++) {
            $previous = $valid[$index - 1]
            $current = $valid[$index]
            $elapsed = ($current.timestampUtc - $previous.timestampUtc).TotalSeconds
            if ($current.sequence -lt $previous.sequence -or $current.droppedFrames -lt $previous.droppedFrames) {
                $resets++
                $stagnantSeconds = 0.0
                continue
            }
            $published += $current.sequence - $previous.sequence
            $dropped += $current.droppedFrames - $previous.droppedFrames
            $observedSeconds += $elapsed
            if ($current.sequence -eq $previous.sequence) {
                $stagnantSeconds += $elapsed
                $maxStagnantSeconds = [math]::Max($maxStagnantSeconds, $stagnantSeconds)
            } else {
                $stagnantSeconds = 0.0
            }
        }
        $fps = $null
        if ($observedSeconds -gt 0) { $fps = [math]::Round($published / $observedSeconds, 3) }
        $dropPercent = $null
        if (($published + $dropped) -gt 0) {
            $dropPercent = [math]::Round($dropped / ($published + $dropped) * 100.0, 4)
        }
        $summaries += [pscustomobject][ordered]@{
            slot = $slot
            validSamples = $valid.Count
            publishedFrames = $published
            droppedFrames = $dropped
            observedSeconds = [math]::Round($observedSeconds, 3)
            observedFps = $fps
            dropPercent = $dropPercent
            sequenceResets = $resets
            maxUnchangedSeconds = [math]::Round($maxStagnantSeconds, 3)
        }
    }
    return $summaries
}

function Get-ProcessSummaries {
    param([Parameter(Mandatory = $true)][object[]]$Samples)

    $flat = @($Samples | ForEach-Object { $_.processes })
    $summaries = @()
    foreach ($group in $flat | Group-Object identity) {
        $rows = @($group.Group)
        $cpu = @($rows | Where-Object { $null -ne $_.cpuPercentOfMachine } | ForEach-Object { [double]$_.cpuPercentOfMachine })
        $summaries += [pscustomobject][ordered]@{
            identity = $group.Name
            role = $rows[-1].role
            pid = $rows[-1].pid
            samples = $rows.Count
            averageCpuPercentOfMachine = if ($cpu.Count) { [math]::Round(($cpu | Measure-Object -Average).Average, 3) } else { $null }
            peakCpuPercentOfMachine = if ($cpu.Count) { [math]::Round(($cpu | Measure-Object -Maximum).Maximum, 3) } else { $null }
            peakWorkingSetBytes = [int64](($rows.workingSetBytes | Measure-Object -Maximum).Maximum)
            peakPrivateBytes = [int64](($rows.privateBytes | Measure-Object -Maximum).Maximum)
        }
    }
    return $summaries
}

function ConvertTo-MarkdownCell {
    param($Value)
    if ($null -eq $Value) { return '' }
    return ([string]$Value).Replace('|', '\|').Replace("`r", ' ').Replace("`n", ' ')
}

function Write-TextAtomically {
    param(
        [Parameter(Mandatory = $true)][string]$LiteralPath,
        [Parameter(Mandatory = $true)][string]$Contents
    )

    $directory = Split-Path -Parent $LiteralPath
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    $temporary = "$LiteralPath.$PID.tmp"
    try {
        [System.IO.File]::WriteAllText($temporary, $Contents, [System.Text.UTF8Encoding]::new($false))
        Move-Item -LiteralPath $temporary -Destination $LiteralPath -Force
    } finally {
        if (Test-Path -LiteralPath $temporary) { Remove-Item -LiteralPath $temporary -Force }
    }
}

function Write-TrialEvidence {
    param(
        [Parameter(Mandatory = $true)]$Report,
        [Parameter(Mandatory = $true)][string]$Directory
    )

    $jsonPath = Join-Path $Directory 'obs-32.1.2-hardware-latest.json'
    $markdownPath = Join-Path $Directory 'obs-32.1.2-hardware-latest.md'
    Write-TextAtomically -LiteralPath $jsonPath -Contents (($Report | ConvertTo-Json -Depth 12) + "`n")

    $lines = @(
        '# OBS 32.1.2 hardware evidence',
        '',
        "Generated: $($Report.generatedAt)",
        '',
        "Sampler status: **$($Report.samplerStatus)**",
        '',
        "Physical gate: **MANUAL REVIEW REQUIRED**",
        ''
    )
    if ($Report.error) {
        $lines += "Preflight error: $(ConvertTo-MarkdownCell $Report.error)"
        $lines += ''
    }
    if ($Report.preflight) {
        $lines += @(
            '## Preflight',
            '',
            '| Item | Value |',
            '| --- | --- |',
            "| Git commit | $(ConvertTo-MarkdownCell $Report.preflight.gitCommit) |",
            "| Git worktree clean | $($Report.preflight.gitWorktreeClean) |",
            "| OBS executable | $(ConvertTo-MarkdownCell $Report.preflight.obs.executable) |",
            "| OBS version | $(ConvertTo-MarkdownCell $Report.preflight.obs.exactVersion) |",
            "| Installer SHA-256 | $(ConvertTo-MarkdownCell $Report.preflight.artifacts.installer.sha256) |",
            "| Built plugin SHA-256 | $(ConvertTo-MarkdownCell $Report.preflight.artifacts.pluginArtifact.sha256) |",
            "| Installed plugin SHA-256 | $(ConvertTo-MarkdownCell $Report.preflight.artifacts.installedPlugin.sha256) |",
            '',
            '## Renderer frame-ring summary',
            '',
            '| Slot | Valid samples | Published | Dropped | Observed fps | Drop % | Sequence resets | Max unchanged (s) |',
            '| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |'
        )
        foreach ($frame in $Report.frameSummaries) {
            $lines += "| $($frame.slot) | $($frame.validSamples) | $($frame.publishedFrames) | $($frame.droppedFrames) | $(ConvertTo-MarkdownCell $frame.observedFps) | $(ConvertTo-MarkdownCell $frame.dropPercent) | $($frame.sequenceResets) | $($frame.maxUnchangedSeconds) |"
        }
        $lines += @(
            '',
            '## Process summary',
            '',
            '| Role | PID | Samples | Avg CPU % of machine | Peak CPU % | Peak working set MiB | Peak private MiB |',
            '| --- | ---: | ---: | ---: | ---: | ---: | ---: |'
        )
        foreach ($process in $Report.processSummaries) {
            $working = [math]::Round($process.peakWorkingSetBytes / 1MB, 2)
            $private = [math]::Round($process.peakPrivateBytes / 1MB, 2)
            $lines += "| $(ConvertTo-MarkdownCell $process.role) | $($process.pid) | $($process.samples) | $(ConvertTo-MarkdownCell $process.averageCpuPercentOfMachine) | $(ConvertTo-MarkdownCell $process.peakCpuPercentOfMachine) | $working | $private |"
        }
    }
    $lines += @(
        '',
        '## Manual issue #28 evidence',
        '',
        '| Gate | Status | Evidence / operator notes |',
        '| --- | --- | --- |',
        '| OBS 32.1.2 module and source registration | NOT RECORDED | Attach sanitized log and source-menu screenshot. |',
        '| Raw renderer / game / OBS sRGB pixel comparison | NOT RECORDED | Record sampled pixels and maximum per-channel delta. |',
        '| A-D and Autoplay mixer routing | NOT RECORDED | Attach mixer and source-property screenshots. |',
        '| Exact-PID desktop mute and restoration matrix | NOT RECORDED | Cover normal exit, crash, reassignment, runtime restart, and OBS restart. |',
        '| 250/500/1500 ms video and Autoplay alignment | NOT RECORDED | Record measurements in frames/ms and network jitter. |',
        '| Positive hitsounds; mines and mine-holds silent | NOT RECORDED | Record chart section and observed counts. |',
        '| Ten-minute four-renderer plus Autoplay soak | NOT RECORDED | Add OBS rendered/missed frames, drift, stale frames, and duplicate-audio result. |',
        '',
        'The sampler is passive evidence only. These manual gates determine issue #28 completion.',
        ''
    )
    Write-TextAtomically -LiteralPath $markdownPath -Contents (($lines -join "`n") + "`n")
    return [pscustomobject]@{ json = $jsonPath; markdown = $markdownPath }
}

function Get-GitEvidence {
    $commit = (& git -C $script:RepositoryRoot rev-parse HEAD 2>$null)
    if ($LASTEXITCODE -ne 0) { $commit = 'unavailable' }
    $status = (& git -C $script:RepositoryRoot status --porcelain 2>$null)
    if ($LASTEXITCODE -ne 0) { $status = @('unavailable') }
    return [pscustomobject]@{
        commit = ([string]$commit).Trim()
        clean = @($status).Count -eq 0
    }
}

function Invoke-ObsTrialPreflight {
    param(
        [Parameter(Mandatory = $true)][string]$SelectedObsDirectory,
        [Parameter(Mandatory = $true)][string]$ExpectedVersion,
        [Parameter(Mandatory = $true)][string]$Installer,
        [Parameter(Mandatory = $true)][string]$PluginArtifact,
        [Parameter(Mandatory = $true)][string]$InstalledPlugin,
        [Parameter(Mandatory = $true)][string]$Checksums
    )

    $obsRoot = Find-ObsDirectory -ExplicitDirectory $SelectedObsDirectory
    $obsExecutable = Join-Path $obsRoot 'bin\64bit\obs64.exe'
    $versionEvidence = Get-ObsVersionEvidence -ExecutablePath $obsExecutable
    $exactVersion = Assert-ExactObsVersion -VersionEvidence $versionEvidence -ExpectedVersion $ExpectedVersion
    $matchingObs = @(Get-Process -Name obs64 -ErrorAction SilentlyContinue | Where-Object {
        try { [System.IO.Path]::GetFullPath($_.Path) -ieq [System.IO.Path]::GetFullPath($obsExecutable) } catch { $false }
    })
    if ($matchingObs.Count -eq 0) {
        throw "OBS $ExpectedVersion must already be running from $obsExecutable. The sampler never launches it."
    }
    $git = Get-GitEvidence
    return [pscustomobject][ordered]@{
        gitCommit = $git.commit
        gitWorktreeClean = $git.clean
        obs = [pscustomobject][ordered]@{
            root = $obsRoot
            executable = $obsExecutable
            productVersion = $versionEvidence.productVersion
            fileVersion = $versionEvidence.fileVersion
            exactVersion = $exactVersion
            processIds = @($matchingObs | ForEach-Object { $_.Id })
        }
        artifacts = Get-ArtifactPreflight -Installer $Installer -PluginArtifact $PluginArtifact -InstalledPlugin $InstalledPlugin -Checksums $Checksums
    }
}

if ($LibraryOnly) { return }

if (-not $RendererDirectory) {
    $RendererDirectory = Join-Path $env:LOCALAPPDATA 'BeatblockOnline\BeatblockOnline\data\render-streams'
}
if (-not $InstallerPath) { $InstallerPath = Join-Path $script:RepositoryRoot 'release\BeatblockOnlineInstaller.exe' }
if (-not $ObsPluginArtifactPath) { $ObsPluginArtifactPath = Join-Path $script:RepositoryRoot 'artifacts\obs\beatblock-online-obs.dll' }
if (-not $InstalledObsPluginPath) {
    $InstalledObsPluginPath = Join-Path $env:ProgramData 'obs-studio\plugins\beatblock-online-obs\bin\64bit\beatblock-online-obs.dll'
}
if (-not $ReleaseChecksumsPath) { $ReleaseChecksumsPath = Join-Path $script:RepositoryRoot 'release\SHA256SUMS.txt' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $script:RepositoryRoot 'reports\trial-runs' }

$generatedAt = [datetime]::UtcNow.ToString('o')
$preflight = $null
try {
    $selectedObs = Find-ObsDirectory -ExplicitDirectory $ObsDirectory
    $preflight = Invoke-ObsTrialPreflight `
        -SelectedObsDirectory $selectedObs `
        -ExpectedVersion $script:RequiredObsVersion `
        -Installer $InstallerPath `
        -PluginArtifact $ObsPluginArtifactPath `
        -InstalledPlugin $InstalledObsPluginPath `
        -Checksums $ReleaseChecksumsPath
} catch {
    $failed = [pscustomobject][ordered]@{
        schemaVersion = 1
        issue = 28
        generatedAt = $generatedAt
        samplerStatus = 'PREFLIGHT_FAILED'
        error = $_.Exception.Message
        preflight = $null
        frameSummaries = @()
        processSummaries = @()
        samples = @()
    }
    $paths = Write-TrialEvidence -Report $failed -Directory $OutputDirectory
    Write-Error "Issue #28 preflight failed. Evidence: $($paths.json). $($_.Exception.Message)"
    exit 1
}

$logicalProcessors = [math]::Max(1, [Environment]::ProcessorCount)
$previousCpu = @{}
$samples = @()
$clock = [System.Diagnostics.Stopwatch]::StartNew()
do {
    $timestamp = [datetime]::UtcNow
    $frames = @()
    foreach ($slot in @('A', 'B', 'C', 'D')) {
        $frames += Get-RendererFrameHeader -Slot $slot -Path (Join-Path $RendererDirectory "stream-$slot.bbtframe")
    }
    $samples += [pscustomobject][ordered]@{
        timestampUtc = $timestamp.ToString('o')
        elapsedSeconds = [math]::Round($clock.Elapsed.TotalSeconds, 3)
        processes = @(Get-TrialProcessSamples -TimestampUtc $timestamp -PreviousCpu $previousCpu -LogicalProcessors $logicalProcessors)
        frames = $frames
    }
    $remaining = $DurationSeconds - $clock.Elapsed.TotalSeconds
    if ($remaining -gt 0) {
        $sleepSeconds = [math]::Min($SampleIntervalSeconds, $remaining)
        Start-Sleep -Milliseconds ([math]::Max(1, [int][math]::Ceiling($sleepSeconds * 1000.0)))
    }
} while ($clock.Elapsed.TotalSeconds -lt $DurationSeconds)
$clock.Stop()

$report = [pscustomobject][ordered]@{
    schemaVersion = 1
    issue = 28
    generatedAt = $generatedAt
    completedAt = [datetime]::UtcNow.ToString('o')
    samplerStatus = 'EVIDENCE_CAPTURED'
    error = $null
    requestedDurationSeconds = $DurationSeconds
    observedDurationSeconds = [math]::Round($clock.Elapsed.TotalSeconds, 3)
    sampleIntervalSeconds = $SampleIntervalSeconds
    logicalProcessors = $logicalProcessors
    rendererDirectory = [System.IO.Path]::GetFullPath($RendererDirectory)
    preflight = $preflight
    frameSummaries = @(Get-FrameSummaries -Samples $samples)
    processSummaries = @(Get-ProcessSummaries -Samples $samples)
    samples = $samples
}
$paths = Write-TrialEvidence -Report $report -Directory $OutputDirectory
Write-Host "Issue #28 passive evidence captured."
Write-Host "JSON: $($paths.json)"
Write-Host "Markdown: $($paths.markdown)"
Write-Host 'Manual color, routing, mute restoration, sync, hitsound, and OBS Stats review is still required.'
