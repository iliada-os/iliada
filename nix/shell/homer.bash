# Homer mode in the Terminal (Iliada): Ctrl+O at the prompt.
#
# An empty prompt: Homer, working in this folder. Something typed: Homer,
# asked about it. Ctrl+O at Homer's prompt comes back. Only the prompt's
# Ctrl+O changes (it was "run this history line and show the next");
# programs started from it still get their own.

homer() {
    iliada ask --here "$@"
}

__iliada_homer_line() {
    if [[ -n $READLINE_LINE ]]; then
        READLINE_LINE=" homer $(printf '%q' "$READLINE_LINE")"
    else
        READLINE_LINE=" homer"
    fi
    READLINE_POINT=${#READLINE_LINE}
}

if [[ $- == *i* && -t 0 ]]; then
    # Rewrite the line (a key nobody types), then run it.
    bind -x '"\C-x\C-_h": __iliada_homer_line'
    bind '"\C-o": "\C-x\C-_h\C-m"'
fi
