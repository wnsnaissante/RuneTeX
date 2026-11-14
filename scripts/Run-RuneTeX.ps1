#requires -Version 5.1
[CmdletBinding()]
param(
    [string]$Project,
    [switch]$Release
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot

if (-not $env:RUNETEX_TEXLAB -and -not (Get-Command texlab -CommandType Application -ErrorAction SilentlyContinue)) {
    $developmentTexlab = Join-Path $workspace '.deps\texlab\texlab.exe'
    if (-not (Test-Path -LiteralPath $developmentTexlab -PathType Leaf)) {
        & (Join-Path $PSScriptRoot 'Install-Texlab.ps1')
    }
    $env:RUNETEX_TEXLAB = $developmentTexlab
}

if (-not (Get-Command cargo -CommandType Application -ErrorAction SilentlyContinue)) {
    throw 'Cargo was not found. Install Rust 1.99.1 and Visual Studio C++ Build Tools, then reopen PowerShell.'
}

$projectPath = Join-Path $workspace 'projects\research-paper'
if ($Project) {
    $projectPath = (Resolve-Path -LiteralPath $Project).ProviderPath
    if ((Test-Path -LiteralPath $projectPath -PathType Leaf) -and [IO.Path]::GetExtension($projectPath) -ine '.tex') {
        throw 'Project must be a directory or a .tex file.'
    }
}

$renderer = & (Join-Path $PSScriptRoot 'Find-PdfRenderer.ps1')
if ($renderer) {
    $env:RUNETEX_PDFTOPPM = $renderer
} else {
    Write-Warning 'pdftoppm was not found. PDF compilation is available; install Poppler or set RUNETEX_PDFTOPPM for previews.'
}

Push-Location -LiteralPath $workspace
try {
    $buildArguments = @('build', '--locked', '--bin', 'runetex')
    $configuration = 'debug'
    if ($Release) {
        $buildArguments += '--release'
        $configuration = 'release'
    }
    & cargo @buildArguments
    if ($LASTEXITCODE -ne 0) { throw "Cargo build failed (exit $LASTEXITCODE)." }

    $executable = Join-Path $workspace "target\$configuration\runetex.exe"
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
        throw "The expected executable was not created: $executable"
    }
    $runArguments = @()
    if ($projectPath) { $runArguments += @('--project', $projectPath) }
    & $executable @runArguments
    if ($LASTEXITCODE -ne 0) { throw "RuneTeX exited with code $LASTEXITCODE." }
} finally {
    Pop-Location
}
