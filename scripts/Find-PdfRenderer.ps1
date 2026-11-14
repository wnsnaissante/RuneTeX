# Returns a pdftoppm executable, or no output when none is installed.
# No recursive search and no changes to machine/user environment variables.
Set-StrictMode -Version Latest

if ($env:RUNETEX_PDFTOPPM -and (Test-Path -LiteralPath $env:RUNETEX_PDFTOPPM -PathType Leaf)) {
    Write-Output $env:RUNETEX_PDFTOPPM
    return
}

$renderer = Get-Command pdftoppm -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
if ($renderer) {
    Write-Output $renderer.Source
    return
}

if ($env:ProgramFiles) {
    $windowsApps = Join-Path $env:ProgramFiles 'WindowsApps'
    $packages = Get-ChildItem -LiteralPath $windowsApps -Directory -Filter 'OpenAI.CodexPrimaryRuntime*' -ErrorAction SilentlyContinue |
        Sort-Object -Property Name -Descending
    foreach ($package in $packages) {
        $candidate = Join-Path $package.FullName 'dependencies\native\poppler\Library\bin\pdftoppm.exe'
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            Write-Output $candidate
            return
        }
    }
}
