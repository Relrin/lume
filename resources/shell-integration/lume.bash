# Shell integration for bash.
#
# Tells the terminal where the shell is, where each prompt begins and ends, which
# command line is running and how it ended. Everything is said with standard escape
# sequences that a terminal either understands or ignores:
#
#   OSC 7    the working directory, as a file:// URL
#   OSC 133  A: a prompt begins   B: the prompt has ended, typing begins
#            C: a command's output begins   D: the command has ended, with its exit code
#   OSC 2    the title: the command line while it runs, the directory at the prompt
#
# Source it from ~/.bashrc, on this machine or another one. It changes no setting and
# no prompt: it adds itself around what is there.

# Only a shell somebody types into has a prompt to mark.
[[ $- == *i* ]] || return 0
# Once is enough.
[[ -z ${__lume_loaded-} ]] || return 0
# Another integration already marks the prompts: leave it to that one.
case "${PS1-}${PS0-}${PROMPT_COMMAND[*]-}" in
*']133;'* | *__vsc_* | *_ksi_* | *__ghostty_* | *__wezterm_* | *iterm2_* | *__vte_osc7*) return 0 ;;
esac
__lume_loaded=1

# The marks put around the prompt. The \[ and \] tell bash they take no room.
__lume_ps1_start='\[\e]133;A\e\\\]'
__lume_ps1_end='\[\e]133;B\e\\\]'
# What is added to PS0, which bash prints after a command line is read and before it
# runs. The first part prints nothing: it only notes that a command line was run, by
# an assignment made while the index of an array that does not exist is worked out.
__lume_ps0='${__lume_none[__lume_ran=1]-}$(__lume_preexec)'

# Writes $1 into __lume_encoded with every byte outside the unreserved set as %XX.
__lume_urlencode() {
    local LC_ALL=C text=$1 safe hex
    __lume_encoded=
    while [[ -n $text ]]; do
        safe=${text%%[!a-zA-Z0-9/:._~-]*}
        __lume_encoded+=$safe
        text=${text#"$safe"}
        if [[ -n $text ]]; then
            printf -v hex '%%%02X' "'${text:0:1}"
            __lume_encoded+=$hex
            text=${text:1}
        fi
    done
}

# Reports the working directory, when it is not the one reported last.
__lume_report_cwd() {
    [[ ${PWD-} != "${__lume_cwd-}" ]] || return 0
    __lume_cwd=$PWD
    local dir=$PWD
    case "${OSTYPE-}" in
    msys* | cygwin*)
        # A drive's own spelling is turned round here; anything else is asked for,
        # which costs a process, so only when the directory changed.
        if [[ $dir =~ ^/([a-zA-Z])(/.*)?$ ]]; then
            dir="/${BASH_REMATCH[1]^^}:${BASH_REMATCH[2]:-/}"
        else
            dir=$(cygpath -m -- "$dir" 2>/dev/null) || dir=$PWD
            [[ $dir == /* ]] || dir="/$dir"
        fi
        ;;
    esac
    __lume_urlencode "$dir"
    __lume_url="file://${HOSTNAME-}${__lume_encoded}"
}

# Runs first of everything bash runs before a prompt, so that the exit code it sees
# is the command line's own.
__lume_precmd() {
    local status=$?
    if [[ -n ${__lume_ran-} ]]; then
        unset __lume_ran
        printf '\e]133;D;%s\e\\' "$status"
    fi
    __lume_report_cwd
    local title=${PWD-}
    [[ -n ${HOME-} && $title == "$HOME"* ]] && title="~${title#"$HOME"}"
    printf '\e]7;%s\e\\\e]2;%s\e\\' "${__lume_url-}" "${title//[[:cntrl:]]/ }"
    return "$status"
}

# Runs last of everything bash runs before a prompt, after whatever else sets the
# prompt has set it.
__lume_mark_prompt() {
    local status=$?
    if [[ ${PS1-} != *"$__lume_ps1_start"* ]]; then
        PS1="${__lume_ps1_start}${PS1-}${__lume_ps1_end}"
    fi
    if [[ -n ${__lume_has_ps0-} && ${PS0-} != *"$__lume_ps0"* ]]; then
        PS0="${PS0-}${__lume_ps0}"
    fi
    return "$status"
}

# Printed through PS0, in a shell of its own: says which command line begins.
__lume_preexec() {
    local line=
    if [[ -o history ]]; then
        line=$(HISTTIMEFORMAT='' builtin history 1 2>/dev/null)
        # Without the number the history gives it, the star of a changed entry, and
        # the blanks around them.
        line=${line#"${line%%[![:space:]]*}"}
        line=${line#"${line%%[![:digit:]]*}"}
        line=${line#\*}
        line=${line#"${line%%[![:space:]]*}"}
    fi
    if [[ -n $line ]]; then
        __lume_urlencode "${line:0:1024}"
        local title=${line:0:256}
        printf '\e]133;C;cmdline_url=%s\e\\\e]2;%s\e\\' "$__lume_encoded" "${title//[[:cntrl:]]/ }"
    else
        printf '\e]133;C\e\\'
    fi
}

# PS0 exists since bash 4.4, and is worked out like this only while the shell
# expands variables in prompts. Without it the commands are not marked; the
# directory and the prompts still are.
if [[ ${BASH_VERSINFO[0]:-0} -gt 4 || (${BASH_VERSINFO[0]:-0} -eq 4 && ${BASH_VERSINFO[1]:-0} -ge 4) ]] &&
    shopt -q promptvars; then
    __lume_has_ps0=1
fi

# Around whatever is run before a prompt already. Lines, not semicolons, lie between:
# a line may follow anything, a semicolon may not follow another.
if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a'* ]]; then
    PROMPT_COMMAND=(__lume_precmd "${PROMPT_COMMAND[@]}" __lume_mark_prompt)
else
    PROMPT_COMMAND="__lume_precmd${PROMPT_COMMAND:+$'\n'$PROMPT_COMMAND}"$'\n''__lume_mark_prompt'
fi
