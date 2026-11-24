_voloo_complete() {
    local n word previous candidate results replace_prefix=
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
        if (( n < COMP_CWORD )); then
            args+=("$word")
            replace_prefix=
        fi
    done

    # Bash 3.2 can keep the assignment whole in COMP_WORDS while Readline
    # replaces only the value. Use the Readline word up to the cursor.
    if [[ $word == --*=* && $2 != --*=* && $COMP_WORDBREAKS == *=* ]]; then
        replace_prefix=${word%%=*}=
        word=$replace_prefix$2
    fi

    results=$(command voloo __complete "=$word" "${args[@]}" 2>/dev/null) || return
    [[ -n $results ]] || return 0
    while IFS= read -r candidate; do
        COMPREPLY+=("${candidate#"$replace_prefix"}")
    done <<< "$results"
}

complete -o filenames -F _voloo_complete voloo
