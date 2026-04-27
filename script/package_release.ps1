param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$ScriptArgs
)

$ErrorActionPreference = 'Stop'

$scriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$repoRoot = Split-Path -Parent $scriptDir

function Get-VsDevCmdPath {
    if ($env:VSDEVCMD_PATH -and (Test-Path $env:VSDEVCMD_PATH)) {
        return $env:VSDEVCMD_PATH
    }

    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    if (Test-Path $vswhere) {
        $installPath = & $vswhere -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
        if ($LASTEXITCODE -eq 0 -and $installPath) {
            $candidate = Join-Path $installPath 'Common7\Tools\VsDevCmd.bat'
            if (Test-Path $candidate) {
                return $candidate
            }
        }
    }

    foreach ($candidate in @(
        'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\Tools\VsDevCmd.bat',
        'C:\Program Files\Microsoft Visual Studio\2022\Professional\Common7\Tools\VsDevCmd.bat',
        'C:\Program Files\Microsoft Visual Studio\2022\Enterprise\Common7\Tools\VsDevCmd.bat',
        'C:\Program Files\Microsoft Visual Studio\2022\BuildTools\Common7\Tools\VsDevCmd.bat'
    )) {
        if (Test-Path $candidate) {
            return $candidate
        }
    }

    throw 'Could not find VsDevCmd.bat. Install Visual Studio Build Tools with the Desktop development with C++ workload, or set VSDEVCMD_PATH.'
}

function Get-GitBashPath {
    if ($env:GIT_BASH_PATH -and (Test-Path $env:GIT_BASH_PATH)) {
        return $env:GIT_BASH_PATH
    }

    $candidates = @()
    $gitCommand = Get-Command git.exe -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($gitCommand) {
        $gitRoot = Split-Path (Split-Path $gitCommand.Path -Parent) -Parent
        $candidates += Join-Path $gitRoot 'bin\bash.exe'
    }

    $candidates += @(
        'C:\Program Files\Git\bin\bash.exe',
        'C:\Program Files (x86)\Git\bin\bash.exe'
    )

    foreach ($candidate in ($candidates | Select-Object -Unique)) {
        if (Test-Path $candidate) {
            return $candidate
        }
    }

    throw 'Could not find Git Bash. Install Git for Windows, or set GIT_BASH_PATH.'
}

function Import-VsDevEnvironment([string]$VsDevCmdPath) {
    cmd.exe /d /c ('@echo off && call "' + $VsDevCmdPath + '" -arch=x64 -host_arch=x64 >nul && set') |
        ForEach-Object {
            if ($_ -match '^(.*?)=(.*)$') {
                Set-Item -Path ("Env:{0}" -f $matches[1]) -Value $matches[2]
            }
        }
}

function Convert-ToPosixPathText([string]$Path) {
    $text = $Path -replace '\\', '/'
    if ($text -match '^([A-Za-z]):(.*)$') {
        return '/' + $matches[1].ToLowerInvariant() + $matches[2]
    }
    return $text
}

function Convert-ToPosixPath([string]$Path) {
    return Convert-ToPosixPathText((Resolve-Path $Path).Path)
}

function Quote-BashArgument([string]$Value) {
    if ($Value.Contains("'")) {
        throw "Arguments containing single quotes are not supported: $Value"
    }
    return "'$Value'"
}

$vsDevCmd = Get-VsDevCmdPath
$gitBash = Get-GitBashPath

Import-VsDevEnvironment $vsDevCmd

if ($env:VCToolsInstallDir) {
    $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_LINKER = Join-Path $env:VCToolsInstallDir 'bin\Hostx64\x64\link.exe'
}

$repoRootPosix = Convert-ToPosixPath $repoRoot
$normalizedScriptArgs = $ScriptArgs | ForEach-Object {
    if ($_ -match '^[A-Za-z]:[\\/]' -or $_ -match '^\\\\') {
        Convert-ToPosixPathText $_
    } else {
        $_
    }
}
$bashCommandParts = @('./script/package_release.sh') + $normalizedScriptArgs
$bashCommand = 'cd ' + (Quote-BashArgument $repoRootPosix) + ' && ' + (($bashCommandParts | ForEach-Object { Quote-BashArgument $_ }) -join ' ')

& $gitBash -lc $bashCommand
exit $LASTEXITCODE
