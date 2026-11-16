# Shell integration for PowerShell.
#
# Tells the terminal where the shell is, where each prompt begins and ends, which
# command line is running and how it ended. Everything is said with standard escape
# sequences that a terminal either understands or ignores:
#
#   OSC 7    the working directory, as a file:// URL
#   OSC 133  A: a prompt begins   B: the prompt has ended, typing begins
#            C: a command's output begins   D: the command has ended, with its exit code
#   OSC 2    the title: the command line while it runs, the folder at the prompt
#
# Dot-source it from your profile ($PROFILE), on this machine or another one. It
# changes no setting: it puts itself around the prompt function that is there.

# Only the console host has a prompt to mark, and only a session that may call what
# this needs. Once is enough.
if ($Host.Name -ne 'ConsoleHost') { return }
if ($ExecutionContext.SessionState.LanguageMode -ne 'FullLanguage') { return }
if (Test-Path -LiteralPath Variable:Global:__LumeIntegration) { return }
# Another integration already marks the prompts: leave it to that one.
if ("$function:prompt" -match '\]133;|\]633;|__VSCode|__wezterm|iterm2') { return }

$Global:__LumeIntegration = @{
    # The prompt as it was, which still draws everything the user sees.
    Prompt        = $function:prompt
    # The function that reads a command line, once it has been put round.
    ReadLine      = $null
    # The newest entry of the history when the last prompt was drawn.
    HistoryId     = -1
    # The newest error there was when the last prompt was drawn.
    LatestError   = $null
    HostName      = [System.Net.Dns]::GetHostName()
    # The most of a command line that is reported, and the most that is shown as title.
    CommandMax    = 1024
    TitleMax      = 256
}

function Global:__Lume-Osc([string] $Body) {
    "$([char] 0x1b)]$Body$([char] 0x1b)\"
}

# Text with everything that could end or break an escape sequence taken out.
function Global:__Lume-Plain([string] $Text, [int] $Max) {
    if ($Text.Length -gt $Max) { $Text = $Text.Substring(0, $Max) }
    $Text -replace '[\x00-\x1f\x7f-\x9f]', ' '
}

function Global:__Lume-Url([string] $Text) {
    [uri]::EscapeDataString($Text).Replace('%2F', '/').Replace('%3A', ':')
}

# Puts this integration round the function that reads a command line, where the
# line editor has provided one. Without it the commands' beginnings are not marked.
function Global:__Lume-WrapReadLine {
    $State = $Global:__LumeIntegration
    if ($State.ReadLine -or -not (Test-Path -LiteralPath Function:PSConsoleHostReadLine)) { return }
    $State.ReadLine = $function:PSConsoleHostReadLine
    function Global:PSConsoleHostReadLine {
        # The line editor asks first of all whether the last command line succeeded,
        # so it is told what it would have seen without this.
        $Succeeded = $?
        $State = $Global:__LumeIntegration
        if (-not $Succeeded) { Write-Error 'failed' -ErrorAction Ignore }
        $Line = $State.ReadLine.Invoke()
        $Text = "$Line"
        if ($Text.Trim()) {
            $Shown = $Text
            if ($Shown.Length -gt $State.CommandMax) { $Shown = $Shown.Substring(0, $State.CommandMax) }
            $Marks = (__Lume-Osc "133;C;cmdline_url=$([uri]::EscapeDataString($Shown))") +
                (__Lume-Osc "2;$(__Lume-Plain $Text $State.TitleMax)")
            [Console]::Write($Marks)
        }
        $Line
    }
}

function Global:prompt {
    # Before anything else here can change them.
    $Succeeded = $?
    $ExitCode = $Global:LASTEXITCODE
    $State = $Global:__LumeIntegration
    $Marks = ''

    # A failure of PowerShell's own leaves a new error behind; a program that ended
    # with an exit code leaves none. So the exit code is the command line's own only
    # when it failed without a new error: otherwise it is an older program's.
    $LatestError = $null
    if ($Global:Error.Count -gt 0) { $LatestError = $Global:Error[0] }
    $FailedItself = -not [object]::ReferenceEquals($LatestError, $State.LatestError)
    $State.LatestError = $LatestError

    # A command line was run if the history has a newer entry than last time.
    $Newest = Get-History -Count 1
    if ($Newest -and $Newest.Id -ne $State.HistoryId) {
        if ($State.HistoryId -ne -1) {
            $Code = 0
            if (-not $Succeeded) {
                $Code = 1
                if (-not $FailedItself -and $ExitCode -is [int] -and $ExitCode -ne 0) { $Code = $ExitCode }
            }
            $Marks += __Lume-Osc "133;D;$Code"
        }
        $State.HistoryId = $Newest.Id
    } elseif (-not $Newest) {
        $State.HistoryId = 0
    }

    $Location = $ExecutionContext.SessionState.Path.CurrentLocation
    if ($Location.Provider.Name -eq 'FileSystem') {
        $Path = $Location.ProviderPath.Replace('\', '/')
        if (-not $Path.StartsWith('/')) { $Path = "/$Path" }
        $Marks += __Lume-Osc "7;file://$($State.HostName)$(__Lume-Url $Path)"
    }
    $Folder = Split-Path -Leaf -Path $Location.Path
    if (-not $Folder) { $Folder = $Location.Path }
    $Marks += __Lume-Osc "2;$(__Lume-Plain $Folder $State.TitleMax)"

    __Lume-WrapReadLine

    # Written now, not handed back with the prompt's text: a prompt that draws itself
    # has drawn before anything handed back is written.
    [Console]::Write($Marks + (__Lume-Osc '133;A'))

    # The user's own prompt sees the exit code and the success of the command line,
    # as it would without this.
    $Global:LASTEXITCODE = $ExitCode
    if (-not $Succeeded) { Write-Error 'failed' -ErrorAction Ignore }
    $Drawn = "$($State.Prompt.Invoke())"
    $Global:LASTEXITCODE = $ExitCode
    # A prompt function that returns nothing gets this from the host.
    if (-not $Drawn) { $Drawn = 'PS>' }

    $Drawn + (__Lume-Osc '133;B')
}

__Lume-WrapReadLine
