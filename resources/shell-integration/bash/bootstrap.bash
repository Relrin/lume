# Loads the shell integration into a bash the terminal started.
#
# The terminal cannot add to a start-up file that is not its own, so it starts bash with
# one line in PROMPT_COMMAND, which bash runs before its first prompt: by then the
# shell's own start-up files have been read. That line reads this file. Here the line
# is taken out again, and the integration is loaded around whatever else was there.

__lume_bootstrap() {
    local status=$? line index
    unset -f __lume_bootstrap
    line='[ -r "${LUME_SHELL_INTEGRATION_DIR-}/bash/bootstrap.bash" ] && . "$LUME_SHELL_INTEGRATION_DIR/bash/bootstrap.bash"'

    # The variable came with the environment. A shell started from this one must not
    # be handed it in turn.
    export -n PROMPT_COMMAND
    # Where the shell could not be known beforehand, a zsh was provided for as well.
    # This is not one, and a zsh started from here is not the terminal's to set up.
    if [[ -n ${LUME_SHELL_INTEGRATION_DIR-} && ${ZDOTDIR-} == "$LUME_SHELL_INTEGRATION_DIR/zsh" ]]; then
        unset ZDOTDIR
    fi
    if [[ $(declare -p PROMPT_COMMAND 2>/dev/null) == 'declare -a'* ]]; then
        for index in "${!PROMPT_COMMAND[@]}"; do
            PROMPT_COMMAND[index]=${PROMPT_COMMAND[index]//"$line"/:}
        done
    elif [[ ${PROMPT_COMMAND-} == "$line" ]]; then
        unset PROMPT_COMMAND
    else
        # Something was put before or after the line. A colon does nothing and may
        # stand wherever a command may.
        PROMPT_COMMAND=${PROMPT_COMMAND//"$line"/:}
    fi

    if [[ -n ${LUME_SHELL_INTEGRATION-} && -z ${__lume_loaded-} && -r ${LUME_SHELL_INTEGRATION_DIR-}/lume.bash ]]; then
        . "$LUME_SHELL_INTEGRATION_DIR/lume.bash"
        if [[ -n ${__lume_loaded-} ]]; then
            # The prompt about to be shown is the first, and what bash runs before it
            # was decided before the integration was there.
            __lume_precmd
            __lume_mark_prompt
        fi
    fi
    return "$status"
}
__lume_bootstrap
