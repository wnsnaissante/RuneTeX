#requires -Version 5.1
[CmdletBinding()]
param()
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$installRoot = [IO.Path]::GetFullPath((Join-Path $workspace '.deps\texlab'))
if (-not $installRoot.StartsWith([IO.Path]::GetFullPath($workspace) + '\', [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid install directory.' }
New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
$archive = Join-Path $installRoot 'texlab-v5.26.0.zip'
Invoke-WebRequest -UseBasicParsing -Uri 'https://github.com/latex-lsp/texlab/releases/download/v5.26.0/texlab-x86_64-windows.zip' -OutFile $archive
$expected = 'cb028d44c3d2b85d36a2ed52d41a0ff43a341b1f04c500c56c4524c4eb72b316'
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ine $expected) { throw 'Texlab archive SHA256 verification failed.' }
Expand-Archive -LiteralPath $archive -DestinationPath $installRoot -Force
Invoke-WebRequest -UseBasicParsing -Uri 'https://raw.githubusercontent.com/latex-lsp/texlab/v5.26.0/LICENSE' -OutFile (Join-Path $installRoot 'LICENSE')
& (Join-Path $installRoot 'texlab.exe') --version
if ($LASTEXITCODE -ne 0) { throw 'Texlab failed to start.' }
