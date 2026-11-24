Register-ArgumentCompleter -Native -CommandName voloo, voloo.cmd, voloo.ps1, voloo.exe -ScriptBlock {
    param($wordToComplete, $commandAst, $cursorPosition)

    $words = @(
        foreach ($element in $commandAst.CommandElements | Select-Object -Skip 1) {
            if ($element.Extent.EndOffset -ge $cursorPosition) { break }
            if ($element -is [System.Management.Automation.Language.StringConstantExpressionAst]) {
                $element.Value
            } else {
                $element.Extent.Text
            }
        }
    )

    $prefix = ([string]$wordToComplete).TrimStart("'", '"')
    # Pass the parsed words as data. Never evaluate text from the command line.
    # Native stdout is UTF-8, including on Windows PowerShell's legacy code pages.
    $encoding = [Console]::OutputEncoding
    try {
        if ($encoding.CodePage -ne 65001) { [Console]::OutputEncoding = [System.Text.Encoding]::UTF8 }
        $candidates = & voloo __complete "=$prefix" @words 2>$null
    } finally {
        if ($encoding.CodePage -ne 65001) { [Console]::OutputEncoding = $encoding }
    }
    foreach ($value in $candidates) {
        $text = $value
        if ($text -match '[^\p{L}\p{N}_.=/:\\-]') {
            $text = "'" + $text.Replace("'", "''") + "'"
        }
        [System.Management.Automation.CompletionResult]::new($text, $value, 'ParameterValue', $value)
    }
}
