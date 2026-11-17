# The defaults of every interactive shell on the phone: over SSH, on the
# serial console, and on the web interface's Shell page.  brush reads this
# file first, then ~/.bashrc and ~/.brushrc (in /nvdata/root, which
# survives firmware updates), which can change any of it.

case $- in
    *i*) ;;
    *) return ;;
esac

# Color where the terminal has it: the web interface's terminal
# (xterm-256color) and the usual SSH clients.  The serial console (vt100)
# and a dumb terminal get none.
case $TERM in
    xterm* | rxvt* | screen* | tmux* | linux | *color*) __dphone_color=1 ;;
    *) __dphone_color= ;;
esac

# The prompt: user@host:directory, with the exit status of the last
# command when it failed, e.g., `root@d60-000FD3010203:/tmp [1]# `.
__dphone_prompt() {
    local status=$?
    if [ "$status" -eq 0 ]; then
        __dphone_status=
    else
        __dphone_status=" [$status]"
    fi
}
PROMPT_COMMAND=__dphone_prompt
if [ -n "$__dphone_color" ]; then
    PS1='\[\e[1;32m\]\u@\h\[\e[0m\]:\[\e[1;34m\]\w\[\e[1;31m\]${__dphone_status}\[\e[0m\]\$ '
else
    PS1='\u@\h:\w${__dphone_status}\$ '
fi
# On a terminal with a window title, the title says the same.
case $TERM in
    xterm* | rxvt*) PS1='\[\e]0;\u@\h: \w\a\]'$PS1 ;;
esac

if [ -n "$__dphone_color" ]; then
    alias ls='ls --color=auto'
fi
alias ll='ls -l'
alias la='ls -lA'

# Tab completion of `dphone`'s commands and options, from the program
# itself (`dphone completion`).  The script is about 150 KB, which takes
# brush longer to read than brush takes to start, so it is read the first
# time a `dphone` command line is completed, not at every start.
__dphone_load_completion() {
    complete -r dphone
    eval "$(dphone completion)" && _dphone "$@"
}
complete -F __dphone_load_completion -o default dphone
