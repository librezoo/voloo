_voloo_complete() {
    local -a candidates
    candidates=("${(@f)$(command voloo __complete "=$PREFIX" "${(@)words[2,CURRENT-1]}" 2>/dev/null)}")
    compadd -a candidates
}

autoload -Uz compinit
if (( ! $+functions[compdef] )); then
    compinit
fi
compdef _voloo_complete voloo
