# Native Windows installation; transfer tools come from a matched Cygwin install.
[CmdletBinding()]
param(
    [switch]$CliOnly,
    [switch]$PathOnly,
    [string]$CygwinBin = $(if ($env:CODESYNC_CYGWIN_BIN) { $env:CODESYNC_CYGWIN_BIN } else { 'C:\cygwin64\bin' }),
    [int]$WaitForProcess = 0
)
$ErrorActionPreference = 'Stop'
try {
    if ($CliOnly -and $PathOnly) { throw 'Choose either -CliOnly or -PathOnly.' }
    if ($WaitForProcess -gt 0) { Wait-Process -Id $WaitForProcess -ErrorAction SilentlyContinue }
    if (-not $env:USERPROFILE) { throw 'USERPROFILE is not set.' }
    $installRoot = if ($env:CARGO_INSTALL_ROOT) { $env:CARGO_INSTALL_ROOT } elseif ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $installRoot = [IO.Path]::GetFullPath($installRoot)
    $binDir = Join-Path $installRoot 'bin'
    if ($PathOnly) {
        if (-not (Test-Path (Join-Path $binDir 'codesync.exe'))) { throw 'Codesync is not installed. Run install.ps1 first.' }
    } else {
        $CygwinBin = [IO.Path]::GetFullPath($CygwinBin)
        foreach ($tool in @('ssh.exe', 'ssh-keygen.exe', 'rsync.exe', 'sha256sum.exe', 'cygwin1.dll')) {
            if (-not (Test-Path (Join-Path $CygwinBin $tool))) { throw "Missing $tool in $CygwinBin. Install Cygwin with openssh, rsync, and coreutils, or specify -CygwinBin." }
        }
        $cargo = Get-Command cargo.exe -ErrorAction SilentlyContinue
        if (-not $cargo) {
            $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
            $cargo = Get-Command (Join-Path $cargoHome 'bin\cargo.exe') -ErrorAction SilentlyContinue
        }
        if (-not $cargo) { throw 'Install native Windows Rust/Cargo and the Visual Studio C++ build tools, then retry.' }
        $cargoArgs = @('install', '--path', $PSScriptRoot, '--root', $installRoot, '--force', '--locked')
        if ($CliOnly) { $cargoArgs += @('--no-default-features', '--bin', 'codesync') }
        & $cargo.Source @cargoArgs
        if ($LASTEXITCODE -ne 0) { throw 'Cargo installation failed.' }
        [Environment]::SetEnvironmentVariable('CODESYNC_CYGWIN_BIN', $CygwinBin, 'User')
        $env:CODESYNC_CYGWIN_BIN = $CygwinBin
    }
    $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
    if (-not (@($userPath -split ';' | ForEach-Object { $_.TrimEnd('\') }) -contains $binDir.TrimEnd('\'))) {
        [Environment]::SetEnvironmentVariable('Path', ($binDir + ';' + $userPath).TrimEnd(';'), 'User')
    }
    if (-not (($env:Path -split ';') -contains $binDir)) { $env:Path = "$binDir;$env:Path" }
    Write-Host 'Codesync installed. Open a new terminal to use it elsewhere.'
    Write-Host "Direct launch: & '$binDir\codesync.exe' gui"
} catch {
    Write-Error $_ -ErrorAction Continue
    exit 1
}
