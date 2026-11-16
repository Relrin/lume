# Shell integration for fish.
#
# Tells the terminal where the shell is, where each prompt begins, which command line
# is running and how it ended. Everything is said with standard escape sequences that
# a terminal either understands or ignores:
#
#   OSC 7    the working directory, as a file:// URL
#   OSC 133  A: a prompt begins
#            C: a command's output begins   D: the command has ended, with its exit code
#
# Source it from ~/.config/fish/config.fish, on this machine or another one. It
# changes no setting, no prompt and no function: it only listens to what fish
# announces. The title is left to fish, which sets it to the running command itself.

# Only a shell somebody types into has a prompt to mark, and once is enough.
if status is-interactive; and not set -q __lume_loaded
    set -g __lume_loaded 1

    # Each sequence ends with an escape and a backslash, written \x5c here so that
    # neither the quotes nor printf can take the backslash for the start of something.

    function __lume_report_cwd --on-event fish_prompt
        printf '\e]7;file://%s%s\e\x5c' $hostname (string escape --style=url -- $PWD)
    end

    # From version 4 fish marks its prompts and its commands itself, in these same
    # words. An older one is helped.
    if not string match -rq '^([4-9]|[1-9][0-9])\.' -- $version
        function __lume_prompt --on-event fish_prompt
            printf '\e]133;A\e\x5c'
        end

        function __lume_preexec --on-event fish_preexec
            printf '\e]133;C;cmdline_url=%s\e\x5c' (string escape --style=url -- (string sub -l 1024 -- "$argv[1]") | string join '%0A')
        end

        function __lume_postexec --on-event fish_postexec
            printf '\e]133;D;%s\e\x5c' $status
        end
    end
end
