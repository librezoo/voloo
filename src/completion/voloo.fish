function __voloo_complete
    set -l words (commandline --current-process --tokenize --cut-at-cursor)
    set -l prefix (commandline --current-token --tokenize --cut-at-cursor)
    command voloo __complete "=$prefix" $words[2..-1] 2>/dev/null
end

complete --exclusive --command voloo --arguments '(__voloo_complete)'
