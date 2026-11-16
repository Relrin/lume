# Loads the shell integration into a fish the terminal started.
#
# The terminal cannot add to a start-up file that is not its own, so it starts fish
# with the folder above this one at the front of XDG_DATA_DIRS: fish reads the files it
# finds there before the user's own. Here that list is put back to what it was, so
# that no program started from this shell sees the folder, and the integration is
# loaded.

if set -q LUME_SHELL_INTEGRATION; and set -q LUME_SHELL_INTEGRATION_DIR
    if set -q LUME_FISH_XDG_DATA_DIRS
        set -gx XDG_DATA_DIRS $LUME_FISH_XDG_DATA_DIRS
        set -e LUME_FISH_XDG_DATA_DIRS
    else
        set -e XDG_DATA_DIRS
    end
    if status is-interactive; and test -r "$LUME_SHELL_INTEGRATION_DIR/lume.fish"
        source "$LUME_SHELL_INTEGRATION_DIR/lume.fish"
    end
end
