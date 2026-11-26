# Homer mode in the Terminal (Iliada): Ctrl+O at the prompt.
#
# An empty prompt: Homer, working in this folder. Something typed: Homer,
# asked about it ("why does make fail" then Ctrl+O). Ctrl+O at Homer's
# prompt comes back. The key belongs to the prompt only: programs started
# from it (nano, vim, less, mc) still get their own Ctrl+O.

function homer --description 'Homer, working in this folder'
    iliada ask --here $argv
end

function __iliada_homer
    set -l typed (commandline | string collect)
    # The leading space keeps it out of the shell's history.
    if test -n "$typed"
        commandline -r -- " homer "(string escape -- $typed)
    else
        commandline -r -- " homer"
    end
    commandline -f execute
end

bind \co __iliada_homer
bind -M insert \co __iliada_homer 2>/dev/null
