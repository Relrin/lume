# Shell integration for zsh.
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
# Source it from ~/.zshrc, on this machine or another one. It changes no setting and
# no prompt: it adds itself around what is there.

# Only a shell somebody types into has a prompt to mark, and once is enough.
[[ -o interactive ]] || return 0
(( ! ${+__lume_loaded} )) || return 0

# The functions below are read with the options zsh starts with, whatever the user has
# set, and with no alias standing in for a word of theirs. What was set is put back at
# the end of this file.
'builtin' 'local' '-a' '__lume_options'
[[ ! -o 'aliases' ]] || __lume_options+=('aliases')
[[ ! -o 'sh_glob' ]] || __lume_options+=('sh_glob')
[[ ! -o 'no_brace_expand' ]] || __lume_options+=('no_brace_expand')
'builtin' 'setopt' 'no_aliases' 'no_sh_glob' 'brace_expand'

builtin typeset -g __lume_loaded=1

# Writes $1 into REPLY with every byte outside the unreserved set as %XX.
__lume_urlencode() {
    emulate -L zsh
    setopt no_multibyte
    local text=$1 char
    local -i index
    REPLY=
    for (( index = 1; index <= ${#text}; index++ )); do
        char=$text[index]
        if [[ $char == [a-zA-Z0-9/:._~-] ]]; then
            REPLY+=$char
        else
            REPLY+=%${(l:2::0:)$(( [##16] #char ))}
        fi
    done
}

# Runs first of the functions zsh runs before a prompt, so that the exit code it sees
# is the command line's own.
__lume_precmd() {
    local -i exit_code=$?
    emulate -L zsh
    if (( ${+__lume_ran} )); then
        unset __lume_ran
        builtin printf '\e]133;D;%d\e\\' $exit_code
    fi
    local REPLY
    __lume_urlencode $PWD
    builtin printf '\e]7;file://%s%s\e\\\e]2;%s\e\\' "$HOST" "$REPLY" "${${(%):-%~}//[[:cntrl:]]/ }"
}

# Runs last of them, after whatever else sets the prompt has set it.
__lume_mark_prompt() {
    emulate -L zsh
    # %{ and %} tell zsh that what lies between takes no room.
    local start=$'%{\e]133;A\e\\%}' finish=$'%{\e]133;B\e\\%}'
    if [[ $PS1 != *$start* ]]; then
        PS1=$start$PS1$finish
    fi
}

# Run by zsh with the command line, once it has been read and before it runs.
__lume_preexec() {
    emulate -L zsh
    typeset -g __lume_ran=1
    local REPLY line=${1[1,1024]}
    __lume_urlencode $line
    builtin printf '\e]133;C;cmdline_url=%s\e\\\e]2;%s\e\\' "$REPLY" "${${line[1,256]}//[[:cntrl:]]/ }"
}

# Run once, before the first prompt: by then the user's own start-up files have set
# their prompt and their own functions, and these go around them.
__lume_install() {
    emulate -L zsh
    builtin typeset -ga precmd_functions preexec_functions
    precmd_functions=(${precmd_functions:#__lume_install})
    builtin unfunction __lume_install
    # Another integration already marks the prompts: leave it to that one.
    if [[ "$PS1 $precmd_functions $preexec_functions" == (*']133;'*|*__vsc_*|*_ksi_*|*_ghostty_*|*__wezterm_*|*iterm2_*) ]]; then
        return
    fi
    precmd_functions=(__lume_precmd $precmd_functions __lume_mark_prompt)
    preexec_functions+=(__lume_preexec)
    # The prompt about to be shown is the first, and the list zsh is working through
    # was made before these were in it.
    __lume_precmd
    __lume_mark_prompt
}

builtin typeset -ga precmd_functions
precmd_functions+=(__lume_install)

(( ${#__lume_options} )) && 'builtin' 'setopt' ${__lume_options[@]}
'builtin' 'unset' '__lume_options'
