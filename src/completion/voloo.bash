_voloo_decode_word() {
    # COMP_WORDS retains quotes and backslashes. Decode common path and
    # nickname forms as text without expanding anything in the command line.
    case $1 in
        \'*) REPLY=${1#\'}; REPLY=${REPLY%\'} ;;
        \"*) REPLY=${1#\"}; REPLY=${REPLY%\"} ;;
        *) IFS= read REPLY <<< "$1" ;;
    esac
}

_voloo_complete() {
    local n word previous candidate results REPLY replace_prefix=
    local -a args=()
    COMPREPLY=()

    # Bash splits --option=value at '=' by default. Rejoin the parts for
    # clap, then return only the part Readline will replace.
    for ((n = 1; n <= COMP_CWORD; n++)); do
        word=${COMP_WORDS[n]-}
        previous=
        if ((${#args[@]})); then
            previous=${args[${#args[@]}-1]}
        fi
        if [[ $word == = && $previous == --* && $previous != *=* ]]; then
            replace_prefix=$previous=
            unset 'args[${#args[@]}-1]'
            if (( n < COMP_CWORD )); then
                ((n += 1))
                word=$replace_prefix${COMP_WORDS[n]-}
            else
                word=$replace_prefix
            fi
        fi
        _voloo_decode_word "$word"
        word=$REPLY
        if (( n < COMP_CWORD )); then
            args+=("$word")
            replace_prefix=
        fi
    done

    # Bash 3.2 can keep the assignment whole in COMP_WORDS while Readline
    # replaces only the value. Use the Readline word up to the cursor.
    if [[ $word == --*=* && $2 != --*=* && $COMP_WORDBREAKS == *=* ]]; then
        replace_prefix=${word%%=*}=
        _voloo_decode_word "$2"
        word=$replace_prefix$REPLY
    fi

    results=$(command voloo __complete "=$word" "${args[@]}" 2>/dev/null) || return
    [[ -n $results ]] || return 0
    while IFS= read -r candidate; do
        COMPREPLY+=("${candidate#"$replace_prefix"}")
    done <<< "$results"
}

complete -o filenames -F _voloo_complete voloo
