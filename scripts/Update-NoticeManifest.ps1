#requires -Version 5.1
[CmdletBinding()]
param(
    [string]$TexlabDirectory,
    [Parameter(Mandatory = $true)][string]$PopplerDirectory
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
. (Join-Path $PSScriptRoot 'NoticeBundle.ps1')
$licenseRoot = Get-NoticeRoot -Workspace $workspace -AllowWorkingChanges
$utf8 = New-Object Text.UTF8Encoding($false)
if (-not $TexlabDirectory) { $TexlabDirectory = Join-Path $workspace '.deps\texlab' }
$popplerRoot = (Get-Item -LiteralPath $PopplerDirectory -Force).FullName

function Get-RegularFiles([string]$Root) {
    $rootItem = Get-Item -LiteralPath $Root -Force
    if (-not $rootItem.PSIsContainer -or ($rootItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "Expected a regular directory: $Root"
    }
    $pending = New-Object 'Collections.Generic.Stack[string]'
    $pending.Push($rootItem.FullName)
    while ($pending.Count -gt 0) {
        foreach ($item in (Get-ChildItem -LiteralPath $pending.Pop() -Force)) {
            if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse point in notice input: $($item.FullName)" }
            if ($item.PSIsContainer) { $pending.Push($item.FullName) } else { $item }
        }
    }
}
function Get-Digest([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force
    if ($item.PSIsContainer -or ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw "Expected a regular file: $Path" }
    return (Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
}
function Read-Inventory([string]$Relative) {
    return Get-Content -LiteralPath (Join-Path $licenseRoot $Relative) -Raw -Encoding UTF8 | ConvertFrom-Json
}

$sourceIndex = Join-Path $workspace 'licenses\SOURCES.md'
$cachedIndex = Join-Path $licenseRoot 'SOURCES.md'
Get-Digest $sourceIndex | Out-Null
Get-Digest $cachedIndex | Out-Null
[IO.File]::WriteAllBytes($cachedIndex, [IO.File]::ReadAllBytes($sourceIndex))

$rust = Read-Inventory 'rust\inventory.json'
$texlab = Read-Inventory 'texlab\inventory.json'
$texlabDependencies = Read-Inventory 'texlab\dependencies\inventory.json'
$fonts = Read-Inventory 'fonts\inventory.json'
$poppler = Read-Inventory 'poppler\inventory.json'
if ($rust.target -ne 'x86_64-pc-windows-msvc' -or
    $rust.cargo_lock_sha256 -ne (Get-Digest (Join-Path $workspace 'Cargo.lock')) -or
    $rust.cargo_toml_sha256 -ne (Get-Digest (Join-Path $workspace 'Cargo.toml'))) {
    throw 'Rust notices do not match this Windows dependency graph. Regenerate them before updating the notice manifest.'
}
$toolchainText = Get-Content -LiteralPath (Join-Path $workspace 'rust-toolchain.toml') -Raw
$toolchainVersion = [regex]::Match($toolchainText, '(?m)^channel\s*=\s*"([^"]+)"').Groups[1].Value
if ($rust.rust_toolchain.version -ne $toolchainVersion) { throw 'Rust standard library notices do not match the selected toolchain.' }
$cargoText = Get-Content -LiteralPath (Join-Path $workspace 'Cargo.toml') -Raw
$runeCommit = [regex]::Match($cargoText, '(?m)^rune\s*=.*?rev\s*=\s*"([a-f0-9]{40})"').Groups[1].Value
if (-not $runeCommit -or $fonts.source_commit -ne $runeCommit) { throw 'Embedded font notices do not match the Rune revision.' }
if ($texlabDependencies.target -ne 'x86_64-pc-windows-msvc' -or
    $texlabDependencies.cargo_lock_sha256 -ne (Get-Digest (Join-Path $licenseRoot 'texlab\SOURCE-Cargo.lock'))) {
    throw 'texlab dependency notices do not match its original source lockfile.'
}
$texlabPath = Join-Path $TexlabDirectory 'texlab.exe'
if ($texlab.binary.sha256 -ne (Get-Digest $texlabPath)) { throw 'texlab notices do not match the executable.' }
if ($texlab.license.sha256 -ne (Get-Digest (Join-Path $TexlabDirectory 'LICENSE'))) { throw 'texlab license does not match the upstream inventory.' }
$native = @{}
foreach ($entry in $poppler.files) {
    if ($native.ContainsKey($entry.path)) { throw "Duplicate Poppler inventory path: $($entry.path)" }
    $native[$entry.path] = $entry.sha256
}
$popplerFiles = @(Get-RegularFiles $popplerRoot)
if ($popplerFiles.Count -ne $native.Count) { throw 'Poppler notice inventory does not cover the complete distribution.' }
$nativeFiles = @()
foreach ($file in ($popplerFiles | Sort-Object FullName)) {
    $relative = $file.FullName.Substring($popplerRoot.TrimEnd('\').Length + 1).Replace('\', '/')
    $digest = Get-Digest $file.FullName
    if (-not $native.ContainsKey($relative) -or $native[$relative] -ne $digest) { throw "Poppler notices do not match: $relative" }
    $nativeFiles += [ordered]@{ path = $relative; sha256 = $digest }
}
$inputs = @()
foreach ($relative in @('Cargo.toml', 'Cargo.lock', 'rust-toolchain.toml')) {
    $inputs += [ordered]@{ path = $relative; sha256 = (Get-Digest (Join-Path $workspace $relative)) }
}
$noticeFiles = @()
foreach ($relative in @('LICENSE', 'THIRD-PARTY-NOTICES.md')) {
    $noticeFiles += [ordered]@{ path = $relative; sha256 = (Get-Digest (Join-Path $workspace $relative)) }
}
foreach ($file in (Get-RegularFiles $licenseRoot | Sort-Object FullName)) {
    $relative = 'licenses/' + $file.FullName.Substring($licenseRoot.TrimEnd('\').Length + 1).Replace('\', '/')
    if ($relative -eq 'licenses/manifest.json') { continue }
    $noticeFiles += [ordered]@{ path = $relative; sha256 = (Get-Digest $file.FullName) }
}
$manifest = [ordered]@{
    schema_version = 1
    target = 'x86_64-pc-windows-msvc'
    inputs = $inputs
    texlab_sha256 = $texlab.binary.sha256
    texlab_license_sha256 = $texlab.license.sha256
    poppler_files = $nativeFiles
    notice_files = $noticeFiles
}
$json = $manifest | ConvertTo-Json -Depth 8
[IO.File]::WriteAllText((Join-Path $licenseRoot 'manifest.json'), $json + "`n", $utf8)

$trackedRoot = [IO.Path]::GetFullPath((Join-Path $workspace 'licenses'))
$cacheRoot = [IO.Path]::GetFullPath((Join-Path $workspace '.cache\notices'))
foreach ($directory in @($trackedRoot, $cacheRoot)) {
    if (-not $directory.StartsWith($workspace + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Notice output is outside the workspace.' }
    $directoryItem = Get-Item -LiteralPath $directory -Force
    if (-not $directoryItem.PSIsContainer -or ($directoryItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) { throw 'Notice output directory must not be a reparse point.' }
}
$archivePath = Join-Path $trackedRoot 'notices.zip'
$checksumPath = Join-Path $trackedRoot 'notices.sha256'
foreach ($path in @($archivePath, $checksumPath)) {
    Get-Digest $path | Out-Null
}
$temporary = [IO.Path]::GetFullPath((Join-Path $cacheRoot ('.pack-' + [guid]::NewGuid().ToString('N') + '.zip')))
if (-not $temporary.StartsWith($cacheRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid temporary notice archive.' }
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
try {
    $archiveStream = [IO.File]::Open($temporary, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
    try {
        $archive = [IO.Compression.ZipArchive]::new($archiveStream, [IO.Compression.ZipArchiveMode]::Create, $true)
        try {
            foreach ($file in (Get-RegularFiles $licenseRoot | Sort-Object FullName)) {
                $relative = 'licenses/' + $file.FullName.Substring($licenseRoot.TrimEnd('\').Length + 1).Replace('\', '/')
                [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $file.FullName, $relative, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
            }
        } finally { $archive.Dispose() }
        $archiveStream.Flush($true)
    } finally { $archiveStream.Dispose() }
    $archiveDigest = Get-Digest $temporary
    Move-Item -LiteralPath $temporary -Destination $archivePath -Force
    [IO.File]::WriteAllText($checksumPath, "$archiveDigest  notices.zip`n", $utf8)
} finally {
    if (Test-Path -LiteralPath $temporary -PathType Leaf) { Remove-Item -LiteralPath $temporary -Force }
}
Write-Output "Notice archive updated: $($noticeFiles.Count) notice files, $($nativeFiles.Count) Poppler files"
