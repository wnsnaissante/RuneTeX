#requires -Version 5.1
[CmdletBinding()]
param(
    [ValidatePattern('^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$')]
    [string]$Version
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$workspace = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
. (Join-Path $PSScriptRoot 'NoticeBundle.ps1')

function Read-Inventory([string]$Relative) {
    $logical = $Relative.Replace('\', '/')
    if (-not $logical.StartsWith('licenses/', [StringComparison]::Ordinal)) { throw 'Inventory must be inside the notice bundle.' }
    return Get-Content -LiteralPath (Join-Path $noticeRoot $logical.Substring(9)) -Raw -Encoding UTF8 | ConvertFrom-Json
}

function Get-Cell([string]$Text) {
    return $Text.Replace('|', '\|').Replace("`r", ' ').Replace("`n", ' ')
}

function Get-Link([string]$Label, [string]$Url) {
    $parsed = $null
    if (-not [Uri]::TryCreate($Url, [UriKind]::Absolute, [ref]$parsed) -or $parsed.Scheme -notin @('http', 'https')) {
        throw "Missing or invalid original source URL: $Label"
    }
    return '[' + (Get-Cell $Label) + '](' + $Url.Replace('(', '%28').Replace(')', '%29') + ')'
}

function Get-Digest([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $sha = [Security.Cryptography.SHA256]::Create()
    try {
        return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '').ToLowerInvariant()
    } finally {
        $sha.Dispose()
        $stream.Dispose()
    }
}

$cargoText = Get-Content -LiteralPath (Join-Path $workspace 'Cargo.toml') -Raw -Encoding UTF8
$packageSection = [regex]::Match($cargoText, '(?ms)^\[package\]\s*(.*?)(?=^\[|\z)').Groups[1].Value
$packageVersion = [regex]::Match($packageSection, '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($packageVersion -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') { throw 'Cargo.toml has no valid package version.' }
if (-not $Version) { $Version = $packageVersion }
if ($Version -cne $packageVersion) {
    throw 'Release version must match Cargo.toml and the current versioned source inventories.'
}
$changelog = Get-Content -LiteralPath (Join-Path $workspace 'CHANGELOG.md') -Raw -Encoding UTF8
$section = [regex]::Match($changelog, '(?ms)^##\s+\[' + [regex]::Escape($Version) + '\][^\r\n]*\r?\n(.*?)(?=^##\s+\[|\z)')
if (-not $section.Success -or [string]::IsNullOrWhiteSpace($section.Groups[1].Value)) {
    throw "CHANGELOG.md has no changes for version $Version. Add the release changes before publishing."
}
$changes = $section.Groups[1].Value.Trim()
$noticeRoot = Get-NoticeRoot -Workspace $workspace
$texlab = Read-Inventory 'licenses\texlab\inventory.json'
$native = Read-Inventory 'licenses\poppler\inventory.json'
$rust = Read-Inventory 'licenses\rust\inventory.json'
$texlabDependencies = Read-Inventory 'licenses\texlab\dependencies\inventory.json'
$fonts = Read-Inventory 'licenses\fonts\inventory.json'
foreach ($inventory in @($texlab, $rust, $texlabDependencies, $fonts)) {
    if ($inventory.target -cne 'x86_64-pc-windows-msvc') { throw 'Source inventories must match the Windows x64 release.' }
}
foreach ($noticeInput in @(@{ File = 'Cargo.toml'; Expected = $rust.cargo_toml_sha256 }, @{ File = 'Cargo.lock'; Expected = $rust.cargo_lock_sha256 })) {
    $digest = Get-Digest (Join-Path $workspace $noticeInput.File)
    if ($digest -cne $noticeInput.Expected) { throw "Source inventory is stale: $($noticeInput.File). Refresh the versioned dependency notices first." }
}

$lines = New-Object 'Collections.Generic.List[string]'
$lines.Add('# RuneTeX ' + $Version)
$lines.Add('')
$lines.Add('## 변경 사항')
$lines.Add('')
$lines.Add($changes)
$lines.Add('')
$lines.Add('## 설치')
$lines.Add('')
$lines.Add('Windows x64용 `RuneTeX-Setup-' + $Version + '-windows-x64.exe`를 실행하세요. 현재 사용자 계정에 설치합니다.')
$lines.Add('')
$lines.Add('설치 파일에는 texlab과 Poppler가 포함되어 있습니다. MiKTeX 또는 TeX Live는 별도로 설치해야 합니다. 기본 엔진은 XeLaTeX입니다.')
$lines.Add('')
$lines.Add('`RuneTeX-' + $Version + '-SHA256SUMS.txt`에는 설치 파일의 SHA-256이 들어 있습니다.')
$lines.Add('')
$lines.Add('## 동봉 구성 요소의 원본 소스')
$lines.Add('')
$lines.Add('아래 링크는 이 릴리스에 동봉한 버전의 원본 소스와 빌드 자료입니다. 라이선스·저작권 고지, 파일 해시와 빌드 레시피·패치는 설치 폴더의 `licenses/`에 포함되어 있습니다.')
$lines.Add('')
$lines.Add('### texlab ' + (Get-Cell $texlab.version))
$lines.Add('')
$lines.Add('- ' + (Get-Link '원본 소스 다운로드' $texlab.source.archive_url) + ' · 커밋 `' + $texlab.source.commit + '`')
$lines.Add('- ' + (Get-Link '소스 코드 보기' $texlab.source.tree_url))
$lines.Add('- ' + (Get-Link '원본 Cargo.lock' $texlab.source.cargo_lock_url))
$lines.Add('- ' + (Get-Link '공식 Windows 릴리스 빌드 절차' $texlab.source.release_build_recipe_url))
$lines.Add('- ' + (Get-Link '소스 빌드 안내' $texlab.source.build_instructions_url))
$lines.Add('')
$lines.Add('### Poppler 및 동봉 네이티브 구성 요소')
$lines.Add('')
$lines.Add('- ' + (Get-Link ('원본 Windows 묶음 ' + $native.distribution.version) $native.distribution.release_url))
$lines.Add('- ' + (Get-Link 'Windows 패키징 소스' $native.distribution.recipe_url))
$lines.Add('- ' + (Get-Link '원본 빌드 기록' $native.distribution.build_log_url))
$lines.Add('')
$lines.Add('<details>')
$lines.Add('<summary>구성 요소별 원본 소스·빌드 자료</summary>')
$lines.Add('')
$lines.Add('Microsoft 런타임 행은 원본 재배포 파일을 안내합니다.')
$lines.Add('')
$lines.Add('| 구성 요소와 빌드 | 원본 소스 또는 재배포 파일 | 빌드 절차·패치 |')
$lines.Add('| --- | --- | --- |')
foreach ($package in ($native.packages | Sort-Object name, version, build)) {
    $preferred = $package.PSObject.Properties['preferred_source_urls']
    $sources = if ($preferred -and @($preferred.Value).Count -gt 0) { @($preferred.Value) } else { @($package.source_urls) }
    $sourceLinks = New-Object 'Collections.Generic.List[string]'
    $number = 0
    foreach ($source in $sources) {
        $availability = $source.PSObject.Properties['availability']
        if ($availability -and $availability.Value -in @('historical-404', 'mirror-timeout')) { continue }
        $number++
        $label = if ($package.name -in @('ucrt', 'vc14_runtime', 'vcomp14')) { '원본 재배포 파일 ' + $number } else { '소스 ' + $number }
        $sourceLinks.Add((Get-Link $label $source.url))
    }
    if ($sourceLinks.Count -eq 0) { throw "No available original source for native package: $($package.id)" }
    $recipe = if ($package.recipe_url) { Get-Link '빌드·패치' $package.recipe_url } else { '소스에 포함' }
    $lines.Add('| ' + (Get-Cell $package.id) + ' | ' + ($sourceLinks -join '<br>') + ' | ' + $recipe + ' |')
}
$lines.Add('')
$lines.Add('</details>')
$lines.Add('')
$lines.Add('### Rune 및 Rust 의존성')
$lines.Add('')
$lines.Add('- ' + (Get-Link ('Rust 표준 라이브러리 ' + $rust.rust_toolchain.version + ' 원본 소스') $rust.rust_toolchain.source_url))
$lines.Add('')
$lines.Add('<details>')
$lines.Add('<summary>Windows 빌드에 사용한 crate별 원본 소스</summary>')
$lines.Add('')
$lines.Add('빌드 도구와 proc-macro를 포함한 Windows 의존성 목록입니다. 개발 전용·다른 플랫폼 전용 항목은 제외했습니다.')
$lines.Add('')
$lines.Add('| 패키지 | 사용 구성 요소 | 원본 소스 |')
$lines.Add('| --- | --- | --- |')
$crates = New-Object 'Collections.Generic.Dictionary[string,object]' ([StringComparer]::Ordinal)
foreach ($group in @(@{ Inventory = $rust; Product = 'RuneTeX' }, @{ Inventory = $texlabDependencies; Product = 'texlab' })) {
    foreach ($package in $group.Inventory.packages) {
        $key = $package.name + '|' + $package.version + '|' + $package.source_url
        if (-not $crates.ContainsKey($key)) {
            $crates[$key] = [pscustomobject]@{ Name = $package.name; Version = $package.version; Url = $package.source_url; Products = (New-Object 'Collections.Generic.List[string]') }
        }
        if (-not $crates[$key].Products.Contains($group.Product)) { $crates[$key].Products.Add($group.Product) }
    }
}
foreach ($package in ($crates.Values | Sort-Object Name, Version, Url)) {
    $lines.Add('| ' + (Get-Cell ($package.Name + ' ' + $package.Version)) + ' | ' + ($package.Products -join ' · ') + ' | ' + (Get-Link '소스 다운로드' $package.Url) + ' |')
}
$lines.Add('')
$lines.Add('</details>')
$lines.Add('')
$lines.Add('### 내장 Inter 글꼴')
$lines.Add('')
foreach ($font in $fonts.fonts) {
    $lines.Add('- ' + (Get-Link ($font.path + ' · ' + $font.version) $font.source_url))
}
$lines.Add('')
$body = ($lines -join "`n")
if ($body.Length -gt 125000) { throw 'Release body exceeds 125000 characters. Shorten the changelog or review the source catalog.' }

# Preserve Unicode for native redirected stdout and restore the caller's encoding.
$consoleEncoding = [Console]::OutputEncoding
try {
    if ([Console]::IsOutputRedirected) { [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false) }
    Write-Output $body
} finally {
    [Console]::OutputEncoding = $consoleEncoding
}
