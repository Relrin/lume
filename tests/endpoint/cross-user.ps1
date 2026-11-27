# Checks that the application's control endpoint, and the session host's, belong to one
# user only, on Windows.
#
#   tests\endpoint\cross-user.ps1 -App <path to lume-gui.exe> -Lume <path to lume.exe>
#
# Three things are checked, each from the side of whoever it protects:
#
#   1. Another user cannot open the pipe of a running application.
#   2. Another user cannot open the pipe of a running session host.
#   3. The console command refuses a pipe that another user created under the name the
#      application's would have, and one under the name the session host's would have.
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

# A script that tries to open a pipe. It exits with 0 when the pipe could not be opened
# for want of permission, 1 when it was opened, 2 for anything else; what that was is
# written to $Report, which every user may write to.
function Open-Script([string] $Pipe, [string] $Report) {
    return @"
try {
    `$pipe = New-Object System.IO.Pipes.NamedPipeClientStream('.', '$Pipe', 'InOut')
    `$pipe.Connect(5000)
    exit 1
} catch [System.UnauthorizedAccessException] {
    exit 0
} catch {
    Set-Content -LiteralPath '$Report' -Value (`$_.Exception.GetType().FullName + ': ' + `$_.Exception.Message)
    exit 2
}
"@
}

# Runs Open-Script as the other user against a pipe, and returns its exit code with what
# it reported.
function Open-AsOther([string] $Pipe) {
    $report = Join-Path $env:PUBLIC "lume-xuser-$PID-$($handed.Count).txt"
    $handed.Add($report)
    $code = Invoke-AsOther (Open-Script $Pipe $report)
    $said = if (Test-Path -LiteralPath $report) { (Get-Content -LiteralPath $report -Raw).Trim() } else { '' }
    return $code, $said
}

# A script that creates a pipe under a name, open to everyone, and keeps it until it is
# ended. An instance that a client has connected to is not given to another until its
# server lets go of it, which this one never does: every client that is to find the pipe
# needs an instance of its own, so there are as many as the name may have.
function Squat-Script([string] $Pipe) {
    return @"
`$everyone = New-Object System.Security.Principal.SecurityIdentifier('S-1-1-0')
`$rule = New-Object System.IO.Pipes.PipeAccessRule(`$everyone, 'FullControl', 'Allow')
`$security = New-Object System.IO.Pipes.PipeSecurity
`$security.AddAccessRule(`$rule)
`$pipes = foreach (`$n in 1..4) {
    New-Object System.IO.Pipes.NamedPipeServerStream('$Pipe', 'InOut', 4, 'Byte', 'None', 4096, 4096, `$security)
}
Start-Sleep -Seconds 600
"@
}

# Waits for a pipe to be gone.
function Wait-NoPipe([string] $Pipe) {
    foreach ($try in 1..100) {
        if (-not ([System.IO.Directory]::GetFiles('\\.\pipe\') -contains "\\.\pipe\$Pipe")) { return $true }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

# Waits for a pipe to exist.
function Wait-Pipe([string] $Pipe) {
    foreach ($try in 1..100) {
        if ([System.IO.Directory]::GetFiles('\\.\pipe\') -contains "\\.\pipe\$Pipe") { return $true }
        Start-Sleep -Milliseconds 100
    }
    return $false
}

$application = $null
$squatter = $null
$hostSquatter = $null
$hostStarted = $false
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

    $code, $said = Open-AsOther "lume-$name-$sid"
    if ($code -eq 1) { Fail 'another user opened the pipe' }
    if ($code -ne 0) { Fail "the attempt to open the pipe ended unexpectedly ($code): $said" }
    Write-Output 'cross-user: another user cannot open the pipe'

    & $Lume ctl --endpoint $name quit | Out-Null
    $application.WaitForExit(30000) | Out-Null

    # 2. A running session host's pipe. It is named after the application's.
    & $Lume host --endpoint $name start | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail 'the session host did not start' }
    $hostStarted = $true
    $code, $said = Open-AsOther "lume-$name-host-$sid"
    if ($code -eq 1) { Fail 'another user opened the session host''s pipe' }
    if ($code -ne 0) { Fail "the attempt to open the session host's pipe ended unexpectedly ($code): $said" }
    Write-Output 'cross-user: another user cannot open the session host''s pipe'
    & $Lume host --endpoint $name stop --end-sessions | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail 'the session host did not stop' }
    $hostStarted = $false

    # 3. A pipe of another user's making, under the name the application's would have,
    # open to everyone.
    $squatter = Start-AsOther (Squat-Script "lume-$planted-$sid")
    # The pipe exists once the other user's process has created it.
    if (-not (Wait-Pipe "lume-$planted-$sid")) { Fail 'the other user''s pipe did not appear' }

    $answer = & $Lume ctl --endpoint $planted status
    if ($LASTEXITCODE -eq 0) { Fail "the console command talked to another user's pipe: $answer" }
    if ($answer -notmatch 'another user') { Fail "the console command did not say why it refused: $answer" }
    Write-Output 'cross-user: the console command refuses a pipe another user made'

    # Gone again, so that `lume ctl` below finds no application and turns to the session
    # host, which is what is checked there.
    $squatter.Kill()
    if (-not (Wait-NoPipe "lume-$planted-$sid")) { Fail 'the other user''s pipe did not go' }

    # The same under the name the session host's would have: neither `lume host` nor
    # `lume ctl`, which turns to the session host when no application runs, talks to it.
    $hostSquatter = Start-AsOther (Squat-Script "lume-$planted-host-$sid")
    if (-not (Wait-Pipe "lume-$planted-host-$sid")) { Fail 'the other user''s host pipe did not appear' }
    $answer = & $Lume host --endpoint $planted status
    if ($LASTEXITCODE -eq 0) { Fail "lume host talked to another user's pipe: $answer" }
    if ($answer -notmatch 'another user') { Fail "lume host did not say why it refused: $answer" }
    $answer = & $Lume ctl --endpoint $planted list
    if ($LASTEXITCODE -eq 0) { Fail "lume ctl talked to another user's session host: $answer" }
    if ($answer -notmatch 'another user') { Fail "lume ctl did not say why it refused: $answer" }
    Write-Output 'cross-user: the console command refuses a session host another user made'
}
finally {
    if ($squatter -and -not $squatter.HasExited) { $squatter.Kill() }
    if ($hostSquatter -and -not $hostSquatter.HasExited) { $hostSquatter.Kill() }
    if ($hostStarted) { & $Lume host --endpoint $name stop --end-sessions *> $null }
    if ($application -and -not $application.HasExited) {
        & $Lume ctl --endpoint $name quit *> $null
        if (-not $application.WaitForExit(10000)) { $application.Kill() }
    }
    foreach ($path in $handed) { Remove-Item -LiteralPath $path -ErrorAction SilentlyContinue }
    Remove-LocalUser -Name $other
}
