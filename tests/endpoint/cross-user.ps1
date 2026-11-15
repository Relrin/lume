# Checks that the application's control endpoint belongs to one user only, on Windows.
#
#   tests\endpoint\cross-user.ps1 -App <path to lume-gui.exe> -Lume <path to lume.exe>
#
# Two things are checked, each from the side of whoever it protects:
#
#   1. Another user cannot open the pipe of a running application.
#   2. The console command refuses a pipe that another user created under the name the
#      application's would have.
#
# It creates a local user for the length of the run and removes it again, so it needs
# an elevated shell. It is meant for a CI runner.

param(
    [Parameter(Mandatory)] [string] $App,
    [Parameter(Mandatory)] [string] $Lume
)

$ErrorActionPreference = 'Stop'

function Fail([string] $Why) {
    Write-Error "cross-user: FAILED: $Why"
    exit 1
}

$sid = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$name = "xuser-$PID"
$planted = "xplant-$PID"

# The other user, with a password nobody needs to know afterwards.
$other = "lume-xuser-$PID"
$bytes = New-Object byte[] 24
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($bytes)
$secret = ConvertTo-SecureString ('Aa1!' + [Convert]::ToBase64String($bytes)) -AsPlainText -Force
New-LocalUser -Name $other -Password $secret -AccountNeverExpires | Out-Null
$credential = New-Object System.Management.Automation.PSCredential("$env:COMPUTERNAME\$other", $secret)

# The scripts handed to the other user, to be removed at the end.
$handed = New-Object System.Collections.Generic.List[string]

# Starts a script as the other user and returns its process.
#
# The script is handed over as a file. A process started as another user may have a
# command line of 1024 characters at most, and a script passed on the command line is
# soon longer than that; the start then fails with "The parameter is incorrect". The
# file is put where every user may read it: the other user cannot read this one's
# temporary directory.
function Start-AsOther([string] $Script, [switch] $Wait) {
    $path = Join-Path $env:PUBLIC "lume-xuser-$PID-$($handed.Count).ps1"
    Set-Content -LiteralPath $path -Value $Script -Encoding Ascii
    $handed.Add($path)
    # The new user's own policy may forbid scripts; this one is ours.
    $arguments = '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', "`"$path`""
    Start-Process powershell.exe -Credential $credential -WorkingDirectory $env:SystemRoot `
        -ArgumentList $arguments -WindowStyle Hidden -PassThru -Wait:$Wait
}

# Runs a script as the other user and returns its exit code.
function Invoke-AsOther([string] $Script) {
    return (Start-AsOther $Script -Wait).ExitCode
}

$application = $null
$squatter = $null
try {
    # 1. A running application's pipe.
    $application = Start-Process $App -ArgumentList '--endpoint', $name -PassThru
    $started = $false
    foreach ($try in 1..300) {
        & $Lume ctl --endpoint $name status *> $null
        if ($LASTEXITCODE -eq 0) { $started = $true; break }
        Start-Sleep -Milliseconds 200
    }
    if (-not $started) { Fail 'the application did not start' }

    # Exit code 0 when the pipe could not be opened for want of permission, 1 when it
    # was opened, 2 for anything else.
    $open = @"
try {
    `$pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', 'lume-$name-$sid', 'InOut')
    `$pipe.Connect(5000)
    exit 1
} catch [System.UnauthorizedAccessException] {
    exit 0
} catch {
    exit 2
}
"@
    $code = Invoke-AsOther $open
    if ($code -eq 1) { Fail 'another user opened the pipe' }
    if ($code -ne 0) { Fail "the attempt to open the pipe ended unexpectedly ($code)" }
    Write-Output 'cross-user: another user cannot open the pipe'

    & $Lume ctl --endpoint $name quit | Out-Null
    $application.WaitForExit(30000) | Out-Null

    # 2. A pipe of another user's making, under the name the application's would have,
    # open to everyone.
    $squat = @"
`$everyone = New-Object System.Security.Principal.SecurityIdentifier('S-1-1-0')
`$rule = New-Object System.IO.Pipes.PipeAccessRule(`$everyone, 'FullControl', 'Allow')
`$security = New-Object System.IO.Pipes.PipeSecurity
`$security.AddAccessRule(`$rule)
`$pipe = New-Object System.IO.Pipes.NamedPipeServerStream('lume-$planted-$sid', 'InOut', 4, 'Byte', 'None', 4096, 4096, `$security)
Start-Sleep -Seconds 30
"@
    $squatter = Start-AsOther $squat

    # The pipe exists once the other user's process has created it.
    $exists = $false
    foreach ($try in 1..100) {
        if ([System.IO.Directory]::GetFiles('\\.\pipe\') -contains "\\.\pipe\lume-$planted-$sid") {
            $exists = $true; break
        }
        Start-Sleep -Milliseconds 100
    }
    if (-not $exists) { Fail 'the other user''s pipe did not appear' }

    $answer = & $Lume ctl --endpoint $planted status
    if ($LASTEXITCODE -eq 0) { Fail "the console command talked to another user's pipe: $answer" }
    if ($answer -notmatch 'another user') { Fail "the console command did not say why it refused: $answer" }
    Write-Output 'cross-user: the console command refuses a pipe another user made'
}
finally {
    if ($squatter -and -not $squatter.HasExited) { $squatter.Kill() }
    if ($application -and -not $application.HasExited) {
        & $Lume ctl --endpoint $name quit *> $null
        if (-not $application.WaitForExit(10000)) { $application.Kill() }
    }
    foreach ($path in $handed) { Remove-Item -LiteralPath $path -ErrorAction SilentlyContinue }
    Remove-LocalUser -Name $other
}
