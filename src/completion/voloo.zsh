_voloo_complete() {
    local -a candidates directories values
    local candidate
    candidates=("${(@f)$(command voloo __complete "=${(Q)PREFIX}" "${(@Q)words[2,CURRENT-1]}" 2>/dev/null)}")
    for candidate in "${candidates[@]}"; do
        case $candidate in
            */|*\\) directories+=("$candidate") ;;
            ?*) values+=("$candidate") ;;
        esac
    done
    compadd -S '' -a directories
    compadd -a values
}

autoload -Uz compinit
if (( ! $+functions[compdef] )); then
    compinit
fi
compdef _voloo_complete voloo
