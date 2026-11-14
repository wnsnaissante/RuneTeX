#requires -Version 5.1
# The tracked ZIP is authoritative. Expanded copies live only in the ignored cache.
$script:NoticeBundleWorkspace = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))

function Assert-NoticeNoReparse([string]$Path) {
    $current = [IO.Path]::GetFullPath($Path)
    while ($current) {
        try {
            $attributes = [IO.File]::GetAttributes($current)
            if ($attributes -band [IO.FileAttributes]::ReparsePoint) {
                throw "Reparse points are not allowed in the notice bundle/cache: $current"
            }
        } catch [IO.FileNotFoundException] {
        } catch [IO.DirectoryNotFoundException] {
        }
        $current = [IO.Path]::GetDirectoryName($current)
    }
}

function Get-NoticeStreamDigest([IO.Stream]$Stream) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($sha.ComputeHash($Stream)).Replace('-', '').ToLowerInvariant() }
    finally { $sha.Dispose() }
}

function Get-NoticeFileMap([string]$Root, [string]$RelativeTo) {
    $files = New-Object 'Collections.Generic.Dictionary[string,string]' ([StringComparer]::OrdinalIgnoreCase)
    $pending = New-Object 'Collections.Generic.Stack[string]'
    $pending.Push($Root)
    while ($pending.Count -gt 0) {
        foreach ($path in [IO.Directory]::EnumerateFileSystemEntries($pending.Pop())) {
            $attributes = [IO.File]::GetAttributes($path)
            if ($attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Reparse point in notice cache: $path" }
            if ($attributes -band [IO.FileAttributes]::Directory) {
                $pending.Push($path)
            } else {
                $relative = $path.Substring($RelativeTo.TrimEnd('\').Length + 1).Replace('\', '/')
                if ($files.ContainsKey($relative)) { throw "Duplicate notice cache path: $relative" }
                $files.Add($relative, $path)
            }
        }
    }
    return ,$files
}

function Assert-NoticeCache([string]$Cache, [string]$Hash, $Archive, $ArchiveFiles, [switch]$AllowWorkingChanges) {
    Assert-NoticeNoReparse $Cache
    $marker = Join-Path $Cache '.bundle.sha256'
    $noticeRoot = Join-Path $Cache 'licenses'
    Assert-NoticeNoReparse $marker
    Assert-NoticeNoReparse $noticeRoot
    if (-not [IO.File]::Exists($marker) -or -not [IO.Directory]::Exists($noticeRoot) -or
        ([IO.FileInfo]$marker).Length -gt 1024 -or [IO.File]::ReadAllText($marker).Trim() -cne $Hash) {
        throw "Incomplete notice cache; preserve any edits and remove this cache before retrying: $Cache"
    }
    $files = Get-NoticeFileMap $noticeRoot $Cache
    if ($AllowWorkingChanges) { return }
    if ($files.Count -ne $ArchiveFiles.Count) {
        throw 'Notice working files differ from notices.zip. Run Update-NoticeManifest.ps1 to repack the reviewed changes.'
    }
    foreach ($relative in $ArchiveFiles.Keys) {
        if (-not $files.ContainsKey($relative)) {
            throw "Notice working file is missing: $relative. Run Update-NoticeManifest.ps1 to repack reviewed changes."
        }
        $source = $ArchiveFiles[$relative].Open()
        try { $expected = Get-NoticeStreamDigest $source } finally { $source.Dispose() }
        $actual = [IO.File]::OpenRead($files[$relative])
        try { $digest = Get-NoticeStreamDigest $actual } finally { $actual.Dispose() }
        if ($digest -cne $expected) {
            throw "Notice working file differs from notices.zip: $relative. Run Update-NoticeManifest.ps1 to repack reviewed changes."
        }
    }
}

function Remove-NoticeStaging([string]$Stage, [string]$CacheRoot) {
    $resolved = [IO.Path]::GetFullPath($Stage).TrimEnd('\')
    if (-not [String]::Equals([IO.Path]::GetDirectoryName($resolved), $CacheRoot, [StringComparison]::OrdinalIgnoreCase) -or
        [IO.Path]::GetFileName($resolved) -notmatch '^\.extract-[0-9a-f]{12}-[0-9a-f]{32}$') {
        throw 'Notice staging cleanup path is outside its expected cache directory.'
    }
    if (-not [IO.Directory]::Exists($resolved)) { return }
    Assert-NoticeNoReparse $resolved
    $null = Get-NoticeFileMap $resolved $CacheRoot
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

function Get-NoticeRoot {
    [CmdletBinding()]
    param([string]$Workspace, [switch]$AllowWorkingChanges)
    if (-not $Workspace) { $Workspace = $script:NoticeBundleWorkspace }
    $workspaceRoot = [IO.Path]::GetFullPath($Workspace).TrimEnd('\')
    $archivePath = Join-Path $workspaceRoot 'licenses\notices.zip'
    $digestPath = Join-Path $workspaceRoot 'licenses\notices.sha256'
    foreach ($path in @($archivePath, $digestPath)) {
        Assert-NoticeNoReparse $path
        if (-not [IO.File]::Exists($path)) { throw "Missing tracked notice bundle input: $path" }
    }
    if (([IO.FileInfo]$digestPath).Length -gt 1024) { throw 'Invalid notices.sha256.' }
    $hashText = [IO.File]::ReadAllText($digestPath)
    $match = [regex]::Match($hashText, '\A([a-fA-F0-9]{64})[ \t]+\*?notices\.zip\s*\z')
    if (-not $match.Success) { throw 'notices.sha256 must contain one SHA-256 entry for notices.zip.' }
    $expectedHash = $match.Groups[1].Value.ToLowerInvariant()
    Add-Type -AssemblyName System.IO.Compression
    $stream = [IO.File]::Open($archivePath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        if ($stream.Length -eq 0 -or $stream.Length -gt 67108864) { throw 'Notice archive exceeds the supported compressed size.' }
        if ((Get-NoticeStreamDigest $stream) -cne $expectedHash) { throw 'notices.zip SHA-256 does not match notices.sha256.' }
        $stream.Position = 0
        $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Read, $true)
        try {
            if ($archive.Entries.Count -eq 0 -or $archive.Entries.Count -gt 10000) { throw 'Invalid notice archive file count.' }
            $seen = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
            $archiveFiles = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::OrdinalIgnoreCase)
            [long]$expandedSize = 0
            foreach ($entry in $archive.Entries) {
                $name = $entry.FullName
                $directory = $name.EndsWith('/')
                $relative = $name.TrimEnd('/')
                if ($relative -ne 'licenses' -and -not $relative.StartsWith('licenses/', [StringComparison]::Ordinal)) { throw "Notice archive path is outside licenses/: $name" }
                if ($relative -eq 'licenses' -and -not $directory) { throw 'The notice archive root must be a directory.' }
                foreach ($part in $relative.Split('/')) {
                    if (-not $part -or $part -in @('.', '..') -or $part.EndsWith('.') -or $part.EndsWith(' ') -or
                        $part -match '[\x00-\x1f<>:"|?*\\]' -or
                        $part -match '^(CON|PRN|AUX|NUL|CONIN\$|CONOUT\$|COM[1-9]|LPT[1-9])(?:\.|$)') {
                        throw "Unsafe notice archive path: $name"
                    }
                }
                if (-not $seen.Add($relative)) { throw "Duplicate notice archive path: $name" }
                if (((($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000) -or
                    ($entry.ExternalAttributes -band 0x400)) { throw "Links are not allowed in the notice archive: $name" }
                if ($entry.Length -lt 0 -or $entry.Length -gt 33554432 -or ($directory -and $entry.Length -ne 0)) { throw "Invalid notice archive entry size: $name" }
                $expandedSize += $entry.Length
                if ($expandedSize -gt 134217728) { throw 'Notice archive exceeds the supported expanded size.' }
                if (-not $directory) { $archiveFiles.Add($relative, $entry) }
            }
            foreach ($required in @('licenses/manifest.json', 'licenses/SOURCES.md', 'licenses/rust/inventory.json', 'licenses/texlab/inventory.json', 'licenses/texlab/dependencies/inventory.json', 'licenses/poppler/inventory.json', 'licenses/fonts/inventory.json')) {
                if (-not $archiveFiles.ContainsKey($required)) { throw "Required notice archive input is missing: $required" }
            }
            $cacheRoot = [IO.Path]::GetFullPath((Join-Path $workspaceRoot '.cache\notices')).TrimEnd('\')
            if (-not $cacheRoot.StartsWith($workspaceRoot + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid notice cache directory.' }
            Assert-NoticeNoReparse $cacheRoot
            [IO.Directory]::CreateDirectory($cacheRoot) | Out-Null
            $cache = Join-Path $cacheRoot $expectedHash
            if ([IO.Directory]::Exists($cache)) {
                Assert-NoticeCache $cache $expectedHash $archive $archiveFiles -AllowWorkingChanges:$AllowWorkingChanges
                return (Join-Path $cache 'licenses')
            }
            Assert-NoticeNoReparse $cache
            $stage = Join-Path $cacheRoot ('.extract-' + $expectedHash.Substring(0, 12) + '-' + [guid]::NewGuid().ToString('N'))
            if ([IO.Directory]::Exists($stage) -or [IO.File]::Exists($stage)) { throw 'Notice staging directory already exists.' }
            [IO.Directory]::CreateDirectory($stage) | Out-Null
            try {
                foreach ($relative in $archiveFiles.Keys) {
                    $target = [IO.Path]::GetFullPath((Join-Path $stage $relative.Replace('/', '\')))
                    if (-not $target.StartsWith($stage + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Notice extraction path escaped staging.' }
                    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($target)) | Out-Null
                    $source = $archiveFiles[$relative].Open()
                    try {
                        $output = [IO.File]::Open($target, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::None)
                        try {
                            $source.CopyTo($output)
                            if ($output.Length -ne $archiveFiles[$relative].Length) { throw "Truncated notice archive entry: $relative" }
                        } finally { $output.Dispose() }
                    } finally { $source.Dispose() }
                }
                [IO.File]::WriteAllText((Join-Path $stage '.bundle.sha256'), $expectedHash + "`n", (New-Object Text.UTF8Encoding($false)))
                # Both resolved move targets must remain immediate children of this cache.
                foreach ($path in @($stage, $cache)) {
                    if (-not [String]::Equals([IO.Path]::GetDirectoryName([IO.Path]::GetFullPath($path)), $cacheRoot, [StringComparison]::OrdinalIgnoreCase)) { throw 'Notice cache move path is outside its cache directory.' }
                }
                Assert-NoticeNoReparse $stage
                $null = Get-NoticeFileMap $stage $cacheRoot
                try { [IO.Directory]::Move($stage, $cache) }
                catch [IO.IOException] {
                    if (-not [IO.Directory]::Exists($cache)) { throw }
                    Assert-NoticeCache $cache $expectedHash $archive $archiveFiles -AllowWorkingChanges:$AllowWorkingChanges
                }
                return (Join-Path $cache 'licenses')
            } finally { Remove-NoticeStaging $stage $cacheRoot }
        } finally { $archive.Dispose() }
    } finally { $stream.Dispose() }
}
