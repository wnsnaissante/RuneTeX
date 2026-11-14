#requires -Version 5.1
[CmdletBinding()]
param(
    [ValidateSet('Release', 'Debug')]
    [string]$Configuration = 'Release',
    [string]$EditorExecutable,
    [string]$SetupExecutable,
    [string]$TexlabDirectory,
    [string]$PopplerDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$utf8 = New-Object Text.UTF8Encoding($false)
. (Join-Path $PSScriptRoot 'NoticeBundle.ps1')
$noticeRoot = Get-NoticeRoot -Workspace $workspace

function Resolve-RegularFile([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Expected a regular file: $Path"
    }
    return $item.FullName
}

function Assert-X64Executable([string]$Path) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $reader = New-Object IO.BinaryReader($stream)
        if ($stream.Length -lt 64 -or $reader.ReadUInt16() -ne 0x5A4D) { throw "Not a Windows executable: $Path" }
        $stream.Position = 0x3c
        $pe = $reader.ReadUInt32()
        if ($pe -gt ($stream.Length - 6)) { throw "Invalid PE header: $Path" }
        $stream.Position = $pe
        if ($reader.ReadUInt32() -ne 0x4550 -or $reader.ReadUInt16() -ne 0x8664) {
            throw "The Windows x64 package requires x64 executables: $Path"
        }
    } finally { $stream.Dispose() }
}

function Assert-UnsignedSetupStub([string]$Path) {
    $stream = [IO.File]::Open($Path, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        $reader = New-Object IO.BinaryReader($stream)
        $stream.Position = 0x3c
        $pe = $reader.ReadUInt32()
        $stream.Position = $pe + 20
        $optionalSize = $reader.ReadUInt16()
        $optionalStart = $pe + 24
        $stream.Position = $optionalStart
        if ($reader.ReadUInt16() -ne 0x20b -or $optionalSize -lt 152 -or
            $optionalStart + $optionalSize -gt $stream.Length) { throw "Invalid x64 optional header: $Path" }
        $stream.Position = $optionalStart + 108
        if ($reader.ReadUInt32() -le 4) { throw "Setup stub has no complete data-directory table: $Path" }
        $stream.Position = $optionalStart + 144
        $certificateOffset = $reader.ReadUInt32()
        $certificateLength = $reader.ReadUInt32()
        if ($certificateOffset -ne 0 -or $certificateLength -ne 0) {
            throw 'Setup stub already has an Authenticode certificate table. Appending a payload would invalidate its signature. Package an unsigned stub, then sign the final installer.'
        }
    } finally { $stream.Dispose() }
}

function Get-DistributionFiles([string]$Root) {
    $rootItem = Get-Item -LiteralPath $Root -Force
    if (-not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Expected a distribution directory without reparse points: $Root"
    }
    $pending = New-Object 'Collections.Generic.Stack[string]'
    $pending.Push($rootItem.FullName)
    while ($pending.Count -gt 0) {
        foreach ($item in (Get-ChildItem -LiteralPath $pending.Pop() -Force)) {
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Distribution contains a reparse point: $($item.FullName)"
            }
            if ($item.PSIsContainer) { $pending.Push($item.FullName) } else { $item }
        }
    }
}

function Add-PayloadFile([string]$Relative, [string]$Source) {
    if ($Relative -match '[\\:*?"<>|\x00-\x1f]' -or $Relative.StartsWith('/') -or
        @($Relative.Split('/') | Where-Object {
            $_ -eq '' -or $_ -eq '.' -or $_ -eq '..' -or $_.EndsWith('.') -or $_.EndsWith(' ') -or
            $_.Split('.')[0] -match '^(CON|PRN|AUX|NUL|CONIN\$|CONOUT\$|COM[1-9]|LPT[1-9])$'
        }).Count -gt 0 -or $Relative -ieq 'uninstall.exe' -or $Relative -ieq '.runetex-install.json') {
        throw "Invalid payload path: $Relative"
    }
    if ($payloadSources.ContainsKey($Relative)) { throw "Duplicate payload path: $Relative" }
    $payloadSources.Add($Relative, (Resolve-RegularFile $Source))
}

function Get-Digest([string]$Path) {
    return (Get-FileHash -LiteralPath (Resolve-RegularFile $Path) -Algorithm SHA256).Hash.ToLowerInvariant()
}

function Remove-LegacyReleaseArtifacts([string]$Directory, [string]$Version, [string]$InstallerName) {
    $root = [IO.Path]::GetFullPath($Directory).TrimEnd('\')
    foreach ($name in @(($InstallerName + '.sha256'), "RuneTeX-$Version-SOURCES.md", "RuneTeX-$Version-LICENSES.zip")) {
        $path = [IO.Path]::GetFullPath((Join-Path $root $name))
        if (-not [String]::Equals([IO.Path]::GetDirectoryName($path), $root, [StringComparison]::OrdinalIgnoreCase)) {
            throw 'Legacy artifact path is outside the release directory.'
        }
        $item = Get-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
        if ($null -eq $item) { continue }
        if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Legacy artifact is not a regular file; it was preserved: $path"
        }
        Remove-Item -LiteralPath $path -Force
    }
}

function Assert-NoticeBundle([string]$Texlab, [string]$TexlabLicense, [string]$PopplerRoot, [object[]]$PopplerFiles) {
    $manifestPath = Join-Path $noticeRoot 'manifest.json'
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
        throw 'Third-party notices are missing. Prepare the versioned license inventories and run scripts\Update-NoticeManifest.ps1 before packaging.'
    }
    $manifest = Get-Content -LiteralPath (Resolve-RegularFile $manifestPath) -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($manifest.schema_version -ne 1 -or $manifest.target -ne 'x86_64-pc-windows-msvc') { throw 'Unsupported third-party notice manifest.' }
    if ((Get-Digest (Join-Path $workspace 'licenses\SOURCES.md')) -ne (Get-Digest (Join-Path $noticeRoot 'SOURCES.md'))) {
        throw 'Source index changed; run scripts\Update-NoticeManifest.ps1 before packaging.'
    }
    $expectedInputs = @('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')
    if (@($manifest.inputs).Count -ne $expectedInputs.Count) { throw 'Incomplete third-party notice build inputs.' }
    foreach ($relative in $expectedInputs) {
        $entry = @($manifest.inputs | Where-Object { $_.path -ceq $relative })
        if ($entry.Count -ne 1 -or $entry[0].sha256 -ne (Get-Digest (Join-Path $workspace $relative))) {
            throw "Dependency input changed; refresh its notices before packaging: $relative"
        }
    }
    if ($manifest.texlab_sha256 -ne (Get-Digest $Texlab) -or $manifest.texlab_license_sha256 -ne (Get-Digest $TexlabLicense)) {
        throw 'texlab changed; refresh its versioned sources and license inventory before packaging.'
    }
    $native = @{}
    foreach ($entry in $manifest.poppler_files) {
        if ($native.ContainsKey($entry.path)) { throw 'Duplicate Poppler notice inventory entry.' }
        $native[$entry.path] = $entry.sha256
    }
    if ($native.Count -ne $PopplerFiles.Count) { throw 'Poppler file list changed; refresh its dependency notices before packaging.' }
    foreach ($file in $PopplerFiles) {
        $relative = $file.FullName.Substring($PopplerRoot.TrimEnd('\').Length + 1).Replace('\', '/')
        if (-not $native.ContainsKey($relative) -or $native[$relative] -ne (Get-Digest $file.FullName)) {
            throw "Poppler changed; refresh its versioned license inventory before packaging: $relative"
        }
    }
    $actual = @{}
    foreach ($relative in @('LICENSE', 'THIRD-PARTY-NOTICES.md')) { $actual[$relative] = Join-Path $workspace $relative }
    foreach ($file in (Get-DistributionFiles $noticeRoot)) {
        $relative = 'licenses/' + $file.FullName.Substring($noticeRoot.TrimEnd('\').Length + 1).Replace('\', '/')
        if ($relative -ne 'licenses/manifest.json') { $actual[$relative] = $file.FullName }
    }
    $seen = @{}
    foreach ($entry in $manifest.notice_files) {
        if ($seen.ContainsKey($entry.path) -or -not $actual.ContainsKey($entry.path) -or
            $entry.sha256 -ne (Get-Digest $actual[$entry.path])) {
            throw "Notice files changed or are missing; refresh the notice manifest: $($entry.path)"
        }
        $seen[$entry.path] = $true
    }
    if ($seen.Count -ne $actual.Count -or -not $seen.ContainsKey('licenses/SOURCES.md')) { throw 'Incomplete third-party notice bundle.' }
}

function Find-PopplerRoot {
    $renderer = & (Join-Path $PSScriptRoot 'Find-PdfRenderer.ps1')
    if (-not $renderer) { return $null }
    $bin = Split-Path -Parent (Resolve-RegularFile $renderer)
    if ((Split-Path -Leaf $bin) -ieq 'bin') {
        $parent = Split-Path -Parent $bin
        if ((Split-Path -Leaf $parent) -ieq 'Library') { return (Split-Path -Parent $parent) }
        if ((Split-Path -Leaf $parent) -match '^poppler(?:[-_.].*)?$') { return $parent }
    }
    if ((Split-Path -Leaf $bin) -match '^poppler(?:[-_.].*)?$') { return $bin }
    return $null
}

$packageText = Get-Content -LiteralPath (Join-Path $workspace 'Cargo.toml') -Raw
$packageSection = [regex]::Match($packageText, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)').Groups[1].Value
$version = [regex]::Match($packageSection, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($version -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') { throw 'Cargo.toml has no valid package version.' }

if (-not $TexlabDirectory) { $TexlabDirectory = Join-Path $workspace '.deps\texlab' }
$texlab = Resolve-RegularFile (Join-Path $TexlabDirectory 'texlab.exe')
$texlabLicense = Resolve-RegularFile (Join-Path $TexlabDirectory 'LICENSE')
Assert-X64Executable $texlab

if (-not $PopplerDirectory) { $PopplerDirectory = Find-PopplerRoot }
if (-not $PopplerDirectory) {
    throw 'Provide -PopplerDirectory with the complete Poppler distribution, including DLLs and licenses. Nothing is downloaded automatically.'
}
$popplerRoot = (Get-Item -LiteralPath $PopplerDirectory -Force).FullName
$popplerFiles = @(Get-DistributionFiles $popplerRoot)
$renderer = Join-Path $popplerRoot 'Library\bin\pdftoppm.exe'
if (-not (Test-Path -LiteralPath $renderer -PathType Leaf)) {
    throw 'PopplerDirectory must be the complete distribution root containing Library\bin\pdftoppm.exe, its DLLs, data and license notices.'
}
$renderer = Resolve-RegularFile $renderer
Assert-X64Executable $renderer
$rendererDirectory = Split-Path -Parent $renderer
$popplerDlls = @($popplerFiles | Where-Object { $_.DirectoryName -ieq $rendererDirectory -and $_.Extension -ieq '.dll' })
if ($popplerDlls.Count -eq 0) { throw 'The Poppler distribution is missing its runtime DLLs.' }
foreach ($dll in $popplerDlls) { Assert-X64Executable $dll.FullName }
$popplerLicenses = @($popplerFiles | Where-Object { $_.Name -match '^(license|licence|copying|copyright|notice)([._-].*)?$' })
if ($popplerLicenses.Count -eq 0) {
    throw 'The Poppler distribution must include its license notices. Supply the distribution root, not just its bin folder.'
}
Assert-NoticeBundle $texlab $texlabLicense $popplerRoot $popplerFiles

$buildBins = @()
if (-not $EditorExecutable) { $buildBins += 'runetex' }
if (-not $SetupExecutable) { $buildBins += 'runetex-setup' }
if ($buildBins.Count -gt 0) {
    if (-not (Get-Command cargo -CommandType Application -ErrorAction SilentlyContinue)) { throw 'Cargo was not found.' }
    $buildArguments = @('build', '--locked')
    if ($Configuration -eq 'Release') { $buildArguments += '--release' }
    foreach ($binary in $buildBins) { $buildArguments += @('--bin', $binary) }
    Push-Location -LiteralPath $workspace
    try {
        & cargo @buildArguments
        if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed. Close any running copy of the selected binaries and retry.' }
    } finally { Pop-Location }
}
$buildProfile = $Configuration.ToLowerInvariant()
if (-not $EditorExecutable) { $EditorExecutable = Join-Path $workspace "target\$buildProfile\runetex.exe" }
if (-not $SetupExecutable) { $SetupExecutable = Join-Path $workspace "target\$buildProfile\runetex-setup.exe" }
$EditorExecutable = Resolve-RegularFile $EditorExecutable
$SetupExecutable = Resolve-RegularFile $SetupExecutable
foreach ($executable in @($EditorExecutable, $SetupExecutable)) {
    Assert-X64Executable $executable
    if ((Get-Item -LiteralPath $executable).VersionInfo.ProductVersion -ne $version) {
        throw "Executable version does not match Cargo.toml ($version): $executable"
    }
}
Assert-UnsignedSetupStub $SetupExecutable

$payloadSources = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
Add-PayloadFile 'runetex.exe' $EditorExecutable
Add-PayloadFile 'tools/texlab/texlab.exe' $texlab
Add-PayloadFile 'tools/texlab/LICENSE' $texlabLicense
foreach ($file in $popplerFiles) {
    $relative = $file.FullName.Substring($popplerRoot.TrimEnd('\').Length + 1).Replace('\', '/')
    Add-PayloadFile ("tools/poppler/$relative") $file.FullName
}
foreach ($notice in @('README.md', 'LICENSE', 'THIRD-PARTY-NOTICES.md')) {
    $source = Join-Path $workspace $notice
    if (Test-Path -LiteralPath $source -PathType Leaf) { Add-PayloadFile $notice $source }
}
foreach ($file in (Get-DistributionFiles $noticeRoot)) {
    $relative = 'licenses/' + $file.FullName.Substring($noticeRoot.TrimEnd('\').Length + 1).Replace('\', '/')
    Add-PayloadFile $relative $file.FullName
}
[string[]]$payloadPaths = @($payloadSources.Keys)
[Array]::Sort($payloadPaths, [StringComparer]::Ordinal)
if ($payloadPaths.Count -eq 0 -or $payloadPaths.Count -gt 50000) { throw 'Payload must contain between 1 and 50000 files.' }

$outputDirectory = [IO.Path]::GetFullPath((Join-Path $workspace 'dist'))
if (-not $outputDirectory.StartsWith($workspace + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid output directory.' }
New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
if ((Get-Item -LiteralPath $outputDirectory -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Output directory must not be a reparse point.' }
$outputName = "RuneTeX-Setup-$version-windows-x64.exe"
$outputPath = Join-Path $outputDirectory $outputName
$temporary = Join-Path $outputDirectory ('.' + $outputName + '.' + [guid]::NewGuid().ToString('N') + '.tmp')
$output = $null
try {
    $output = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    $stub = [IO.File]::Open($SetupExecutable, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try { $stub.CopyTo($output, 1048576) } finally { $stub.Dispose() }
    [uint64]$payloadStart = $output.Position
    $entries = New-Object 'Collections.Generic.List[object]'
    foreach ($relative in $payloadPaths) {
        $sourceStream = [IO.File]::Open($payloadSources[$relative], [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            [uint64]$size = $sourceStream.Length
            $digest = [BitConverter]::ToString($sha.ComputeHash($sourceStream)).Replace('-', '').ToLowerInvariant()
            $sourceStream.Position = 0
            [uint64]$offset = $output.Position - $payloadStart
            $sourceStream.CopyTo($output, 1048576)
            $entries.Add([ordered]@{ path = $relative; size = $size; sha256 = $digest; offset = $offset })
        } finally { $sha.Dispose(); $sourceStream.Dispose() }
    }
    $manifest = [ordered]@{ app_id = 'RuneTeX'; version = $version; files = @($entries.ToArray()) }
    $manifestBytes = $utf8.GetBytes(($manifest | ConvertTo-Json -Depth 5 -Compress))
    if ($manifestBytes.Length -gt 16777216) { throw 'Payload manifest exceeds the supported 16 MiB limit.' }
    [uint64]$manifestStart = $output.Position
    $output.Write($manifestBytes, 0, $manifestBytes.Length)
    $writer = [IO.BinaryWriter]::new($output, [Text.Encoding]::UTF8, $true)
    try {
        $writer.Write([Text.Encoding]::ASCII.GetBytes('RUNETEX_SETUP_V1'))
        $writer.Write([uint64]$payloadStart)
        $writer.Write([uint64]$manifestStart)
        $writer.Write([uint64]$manifestBytes.Length)
        $writer.Flush()
    } finally { $writer.Dispose() }
    $output.Flush($true)
    $output.Dispose()
    $output = $null
    Move-Item -LiteralPath $temporary -Destination $outputPath -Force
    $packageHash = (Get-FileHash -LiteralPath $outputPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $checksumsPath = Join-Path $outputDirectory "RuneTeX-$version-SHA256SUMS.txt"
    [IO.File]::WriteAllText($checksumsPath, "$packageHash  $outputName`n", $utf8)
    Remove-LegacyReleaseArtifacts $outputDirectory $version $outputName
    Write-Output "Installer: $outputPath"
    Write-Output "SHA256: $checksumsPath"
    Write-Output "Payload files: $($payloadPaths.Count)"
    Write-Output 'Release body: scripts\Write-ReleaseNotes.ps1 (Markdown to standard output)'
} finally {
    if ($output) { $output.Dispose() }
    if (Test-Path -LiteralPath $temporary -PathType Leaf) { Remove-Item -LiteralPath $temporary -Force }
}
