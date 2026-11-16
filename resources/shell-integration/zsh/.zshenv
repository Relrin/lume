# Loads the shell integration into a zsh the terminal started.
#
# The terminal cannot add to a start-up file that is not its own, so it starts zsh with
# ZDOTDIR naming this folder: zsh then reads this file where it would have read the
# user's .zshenv. Here ZDOTDIR is put back to what it was, so that every later start-up
# file is the user's own, the user's .zshenv is read, and the integration is loaded.

if [[ -n "${LUME_ZSH_ZDOTDIR+set}" ]]; then
    'builtin' 'export' ZDOTDIR="$LUME_ZSH_ZDOTDIR"
    'builtin' 'unset' 'LUME_ZSH_ZDOTDIR'
else
    'builtin' 'unset' 'ZDOTDIR'
fi

# Where the shell could not be known beforehand, a bash was provided for as well. This
# is not one, and a bash started from here is not the terminal's to set up.
if [[ "${PROMPT_COMMAND-}" == '[ -r "${LUME_SHELL_INTEGRATION_DIR-}/bash/bootstrap.bash" ] && . "$LUME_SHELL_INTEGRATION_DIR/bash/bootstrap.bash"' ]]; then
    'builtin' 'unset' 'PROMPT_COMMAND'
fi

{
    # zsh treats an unset ZDOTDIR as the home directory.
    __lume_user_zshenv="${ZDOTDIR-$HOME}/.zshenv"
    # Not this file again, should ZDOTDIR have come back as this folder.
    if [[ -r "$__lume_user_zshenv" && ! "$__lume_user_zshenv" -ef "${(%):-%x}" ]]; then
        'builtin' 'source' '--' "$__lume_user_zshenv"
    fi
} always {
    'builtin' 'unset' '__lume_user_zshenv'
    if [[ -o interactive && -n "${LUME_SHELL_INTEGRATION-}" && -r "${LUME_SHELL_INTEGRATION_DIR-}/lume.zsh" ]]; then
        'builtin' 'source' '--' "$LUME_SHELL_INTEGRATION_DIR/lume.zsh"
    fi
}
