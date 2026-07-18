param(
    [string]$Executable = "$PSScriptRoot/../bin/x64/Debug-NAudio/net10.0-windows/win-x64/Emuera.ReferenceCli.exe"
)

$ErrorActionPreference = "Stop"
$executablePath = [System.IO.Path]::GetFullPath($Executable)
if (-not [System.IO.File]::Exists($executablePath)) {
    throw "Reference CLI not found: $executablePath. Build Debug-NAudio/win-x64 first."
}

$startInfo = [System.Diagnostics.ProcessStartInfo]::new()
$startInfo.FileName = $executablePath
$startInfo.UseShellExecute = $false
$startInfo.RedirectStandardInput = $true
$startInfo.RedirectStandardOutput = $true
$startInfo.RedirectStandardError = $true
$startInfo.StandardInputEncoding = [System.Text.UTF8Encoding]::new($false)
$startInfo.StandardOutputEncoding = [System.Text.UTF8Encoding]::new($false)
$process = [System.Diagnostics.Process]::new()
$process.StartInfo = $startInfo
if (-not $process.Start()) { throw "Could not start reference CLI" }

function Invoke-Oracle([hashtable]$Request) {
    $json = $Request | ConvertTo-Json -Compress -Depth 20
    $process.StandardInput.WriteLine($json)
    $process.StandardInput.Flush()
    $line = $process.StandardOutput.ReadLine()
    if ($null -eq $line) {
        throw "Reference CLI exited early: $($process.StandardError.ReadToEnd())"
    }
    return $line | ConvertFrom-Json
}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
}

$tempGame = $null
$tempSystemGame = $null
$tempOneInputGame = $null
$tempOneInputLongGame = $null
try {
    $bad = Invoke-Oracle @{ id = 1; op = "doesNotExist" }
    Assert-True (-not $bad.ok) "unknown operation should fail"

    $caps = Invoke-Oracle @{ id = 2; op = "capabilities" }
    Assert-True $caps.ok "process did not survive the failed request"
    Assert-True ($caps.schemaVersion -eq 1) "unexpected schema version"

    $lex = Invoke-Oracle @{ id = 3; op = "lex"; source = "RESULT = 1 + 2" }
    Assert-True $lex.ok "lex failed"
    Assert-True ($lex.result.tokens.Count -gt 3) "lex returned too few tokens"

    $expression = Invoke-Oracle @{ id = 4; op = "parseExpression"; source = "1 + 2 * 3" }
    Assert-True $expression.ok "parseExpression failed"

    $logicalLine = Invoke-Oracle @{ id = 5; op = "parseLine"; source = "PRINTL hello"; reduceArguments = $false }
    Assert-True $logicalLine.ok "parseLine failed"
    Assert-True ($logicalLine.result.functionCode -eq "PRINTL") "wrong instruction code"

    $tempGame = Join-Path ([System.IO.Path]::GetTempPath()) ("emuera-oracle-" + [guid]::NewGuid())
    Copy-Item "$PSScriptRoot/fixture" $tempGame -Recurse

    $load = Invoke-Oracle @{ id = 6; op = "load"; gameDir = $tempGame }
    Assert-True $load.ok "fixture game failed to load"
    Assert-True ($load.result.termination -eq "waitingInput") "fixture title did not request input"

    $timedOneInput = Invoke-Oracle @{
        id = "toneinput"
        op = "execute"
        statement = 'TONEINPUTS 1000, "DEFAULT", 1, "timeout", 0, 0'
    }
    Assert-True $timedOneInput.ok "timed one-input execution failed"
    Assert-True ($timedOneInput.result.termination -eq "waitingInput") "timed one-input did not wait"
    Assert-True ($timedOneInput.result.inputRequest.InputType -eq "StrValue") "timed one-input type differs"
    Assert-True $timedOneInput.result.inputRequest.OneInput "timed one-input flag differs"
    Assert-True ($timedOneInput.result.inputRequest.Timelimit -eq "1000") "timed one-input limit differs"

    $project = Invoke-Oracle @{ id = "project"; op = "analyzeProject" }
    Assert-True $project.ok "project semantic projection failed"
    Assert-True (($project.result.functions.name -contains "SYSTEM_TITLE") -and
        ($project.result.functions.name -contains "ORACLE_TEST") -and
        ($project.result.functions.name -contains "ORACLE_INPUT") -and
        ($project.result.functions.name -contains "ORACLE_MAP") -and
        ($project.result.functions.name -contains "ORACLE_NATIVE") -and
        ($project.result.functions.name -contains "ORACLE_REFLECTION") -and
        ($project.result.functions.name -contains "ORACLE_PRESENTATION") -and
        ($project.result.functions.name -contains "ORACLE_STRUCTURED")) "project function projection differs"

    $varSize = Invoke-Oracle @{ id = "csv-varsize"; op = "eval"; source = 'VARSIZE("ABL")' }
    Assert-True ($varSize.ok -and $varSize.result.value -eq 120) "VariableSize.CSV did not resize ABL"
    $nameIndex = Invoke-Oracle @{ id = "csv-name"; op = "eval"; source = 'GETNUM(ABL, "later")' }
    Assert-True ($nameIndex.ok -and $nameIndex.result.value -eq 2) "ABL name lookup differs"
    $itemPrice = Invoke-Oracle @{ id = "csv-price"; op = "eval"; source = "ITEMPRICE:5" }
    Assert-True ($itemPrice.ok -and $itemPrice.result.value -eq 120) "ITEM price differs"
    $strValue = Invoke-Oracle @{ id = "csv-str"; op = "eval"; source = "STR:0" }
    Assert-True ($strValue.ok -and $strValue.result.value -eq "initial text") "STR initial data differs"
    $character = Invoke-Oracle @{ id = "csv-character"; op = "eval"; source = "CSVABL(10, 2)" }
    Assert-True ($character.ok -and $character.result.value -eq 5) "character CSV data differs"
    $gameCode = Invoke-Oracle @{ id = "csv-gamebase"; op = "eval"; source = "GAMEBASE_GAMECODE" }
    Assert-True ($gameCode.ok -and $gameCode.result.value -eq 42) "GAMEBASE code differs"

    $analyzed = Invoke-Oracle @{ id = 7; op = "analyzeLine"; source = "RESULT = 9" }
    Assert-True $analyzed.ok "semantic line analysis failed"
    Assert-True ($null -ne $analyzed.result.argument) "semantic argument was not produced"

    $executed = Invoke-Oracle @{ id = 8; op = "execute"; statement = "RESULT = 9"; watch = @("RESULT") }
    Assert-True $executed.ok "single-instruction execution failed"
    Assert-True ($executed.result.watches.RESULT -eq 9) "execute did not update RESULT"

    $putform = Invoke-Oracle @{ id = "putform"; op = "execute"; statement = 'PUTFORM suffix'; watch = @("SAVEDATA_TEXT") }
    Assert-True ($putform.ok -and $putform.result.watches.SAVEDATA_TEXT -eq "suffix") "PUTFORM differs"
    $saveNos = Invoke-Oracle @{ id = "savenos"; op = "eval"; source = "SAVENOS()" }
    Assert-True ($saveNos.ok -and $saveNos.result.value -eq 20) "SAVENOS differs"

    $run = Invoke-Oracle @{ id = 9; op = "run"; entry = "ORACLE_TEST"; watch = @("RESULT") }
    Assert-True $run.ok "isolated function run failed"
    Assert-True ($run.result.termination -eq "completed") "function did not complete"
    Assert-True (($run.result.output -join "`n") -match "ORACLE_OK") "function output missing"

    $nativeTail = Invoke-Oracle @{
        id = "native-tail"
        op = "run"
        entry = "ORACLE_NATIVE"
        watch = @("RESULT:0", "RESULT:1", "RESULT:2", "RESULT:3", "RESULT:4", "RESULT:5", "RESULT:6", "RESULT:7", "RESULT:8", "RESULT:9", "RESULT:10", "RESULT:11", "RESULT:12", "RESULTS:0", "RESULTS:1", "RESULTS:2", "RESULTS:3", "RESULTS:4", "RESULTS:5", "RESULTS:6", "RESULTS:7")
    }
    Assert-True ($nativeTail.ok -and $nativeTail.result.termination -eq "completed") "native tail failed"
    Assert-True (($nativeTail.result.watches.'RESULT:0' -eq 0) -and
        ($nativeTail.result.watches.'RESULT:1' -eq 4) -and
        ($nativeTail.result.watches.'RESULT:2' -eq 1) -and
        ($nativeTail.result.watches.'RESULT:3' -eq 1) -and
        ($nativeTail.result.watches.'RESULT:4' -eq 2) -and
        ($nativeTail.result.watches.'RESULT:5' -eq 2) -and
        ($nativeTail.result.watches.'RESULT:6' -eq 1) -and
        ($nativeTail.result.watches.'RESULT:7' -eq 946) -and
        ($nativeTail.result.watches.'RESULT:8' -eq 946) -and
        ($nativeTail.result.watches.'RESULT:9' -eq 12) -and
        ($nativeTail.result.watches.'RESULT:10' -eq 1) -and
        ($nativeTail.result.watches.'RESULT:11' -eq 66051) -and
        ($nativeTail.result.watches.'RESULT:12' -eq 2) -and
        ($nativeTail.result.watches.'RESULTS:0' -eq 'a\+b') -and
        ($nativeTail.result.watches.'RESULTS:1' -eq 'β') -and
        ($nativeTail.result.watches.'RESULTS:2' -eq 'ABC') -and
        ($nativeTail.result.watches.'RESULTS:3' -eq 'abc') -and
        ($nativeTail.result.watches.'RESULTS:4' -eq 'b/c') -and
        ($nativeTail.result.watches.'RESULTS:5' -eq 'a,b,c,') -and
        ($nativeTail.result.watches.'RESULTS:6' -eq 'β') -and
        ($nativeTail.result.watches.'RESULTS:7' -eq 'ff')) "native tail differs"

    $reflection = Invoke-Oracle @{
        id = "reflection"
        op = "run"
        entry = "ORACLE_REFLECTION"
        watch = @("RESULT:12", "RESULT:13", "RESULTS:8", "RESULTS:9")
    }
    Assert-True ($reflection.ok -and $reflection.result.termination -eq "completed") "reflection run failed"
    Assert-True (($reflection.result.watches.'RESULT:12' -eq 1) -and
        ($reflection.result.watches.'RESULT:13' -eq 1) -and
        ($reflection.result.watches.'RESULTS:8' -eq 'ORACLE_REFLECTION') -and
        ($reflection.result.watches.'RESULTS:9' -eq 'SAVEDATA_TEXT')) "reflection result differs"

    $mapRun = Invoke-Oracle @{ id = "map"; op = "run"; entry = "ORACLE_MAP"; watch = @("RESULT", "RESULTS") }
    Assert-True $mapRun.ok "map function run failed"
    Assert-True ($mapRun.result.termination -eq "completed") "map function did not complete"
    Assert-True (($mapRun.result.output -join "`n") -match [regex]::Escape("MAP=2,1,1,1|3|b,a")) "map output differs"

    $presentationRun = Invoke-Oracle @{ id = "presentation"; op = "run"; entry = "ORACLE_PRESENTATION" }
    Assert-True $presentationRun.ok "presentation function run failed"
    Assert-True ($presentationRun.result.termination -eq "completed") "presentation function did not complete"
    Assert-True (($presentationRun.result.output -join "`n").Contains("VISIBLE")) "NOSKIP presentation output differs"

    $structuredRun = Invoke-Oracle @{
        id = "structured"
        op = "run"
        entry = "ORACLE_STRUCTURED"
        watch = @("RESULT:0", "RESULT:1", "RESULT:2", "RESULT:3", "RESULT:4", "RESULT:5", "RESULTS:0", "RESULTS:1", "RESULTS:2")
    }
    Assert-True $structuredRun.ok "structured function run failed"
    Assert-True ($structuredRun.result.termination -eq "completed") "structured function did not complete"
    Assert-True ($structuredRun.result.watches.'RESULTS:0'.Contains('<xs:schema id="NewDataSet"')) "DataTable schema differs"
    Assert-True ($structuredRun.result.watches.'RESULTS:1'.Contains('A&amp;B')) "DataTable data XML differs"
    Assert-True ($structuredRun.result.watches.'RESULTS:2' -eq '<root><item id="a" kind="first">one</item><item id="b">changed</item></root>') "XML mutation differs"
    Assert-True (($structuredRun.result.watches.'RESULT:4' -eq 1) -and ($structuredRun.result.watches.'RESULT:5' -eq 1)) "XML mutation counts differ"

    $inputRun = Invoke-Oracle @{ id = 10; op = "run"; entry = "ORACLE_INPUT"; inputs = @("42"); watch = @("RESULT") }
    Assert-True $inputRun.ok "input function run failed"
    Assert-True ($inputRun.result.termination -eq "completed") "input function did not complete"
    Assert-True ($inputRun.result.watches.RESULT -eq 42) "input did not update RESULT"

    $tempOneInputGame = Join-Path ([System.IO.Path]::GetTempPath()) ("emuera-oneinput-oracle-" + [guid]::NewGuid())
    Copy-Item "$PSScriptRoot/fixture" $tempOneInputGame -Recurse
    Copy-Item "$PSScriptRoot/fixture-oneinput/*" $tempOneInputGame -Recurse -Force
    $oneInputLoad = Invoke-Oracle @{ id = "oneinput-load"; op = "load"; gameDir = $tempOneInputGame }
    Assert-True $oneInputLoad.ok "ONEINPUT fixture failed to load"
    $oneInputText = Invoke-Oracle @{
        id = "oneinput-text"
        op = "run"
        entry = "ORACLE_ONEINPUT"
        uiInputs = @(
            @{ text = "12" }, @{ text = "βx" }, @{ text = "34" }, @{ text = "yz" }
        )
        watch = @("RESULT:40", "RESULT:41", "RESULTS:40", "RESULTS:41")
    }
    Assert-True (($oneInputText.result.watches.'RESULT:40' -eq 1) -and
        ($oneInputText.result.watches.'RESULT:41' -eq 3) -and
        ($oneInputText.result.watches.'RESULTS:40' -eq "β") -and
        ($oneInputText.result.watches.'RESULTS:41' -eq "y")) "ONEINPUT text truncation differs"
    $oneInputMouse = Invoke-Oracle @{
        id = "oneinput-mouse-default"
        op = "run"
        entry = "ORACLE_ONEINPUT_MOUSE"
        uiInputs = @(
            @{ text = "42"; changedByMouse = $true },
            @{ text = "LONG"; changedByMouse = $true }
        )
        watch = @("RESULT:42", "RESULTS:42")
    }
    Assert-True (($oneInputMouse.result.watches.'RESULT:42' -eq 4) -and
        ($oneInputMouse.result.watches.'RESULTS:42' -eq "L")) "default mouse ONEINPUT truncation differs"

    $tempOneInputLongGame = Join-Path ([System.IO.Path]::GetTempPath()) ("emuera-oneinput-long-oracle-" + [guid]::NewGuid())
    Copy-Item "$PSScriptRoot/fixture" $tempOneInputLongGame -Recurse
    Copy-Item "$PSScriptRoot/fixture-oneinput/*" $tempOneInputLongGame -Recurse -Force
    Copy-Item "$PSScriptRoot/fixture-oneinput-long/*" $tempOneInputLongGame -Recurse -Force
    $oneInputLongLoad = Invoke-Oracle @{ id = "oneinput-long-load"; op = "load"; gameDir = $tempOneInputLongGame }
    Assert-True $oneInputLongLoad.ok "long ONEINPUT fixture failed to load"
    $oneInputLong = Invoke-Oracle @{
        id = "oneinput-mouse-long"
        op = "run"
        entry = "ORACLE_ONEINPUT_MOUSE"
        uiInputs = @(
            @{ text = "42"; changedByMouse = $true },
            @{ text = "LONG"; changedByMouse = $true }
        )
        watch = @("RESULT:42", "RESULTS:42")
    }
    Assert-True (($oneInputLong.result.watches.'RESULT:42' -eq 42) -and
        ($oneInputLong.result.watches.'RESULTS:42' -eq "LONG")) "AllowLongInputByMouse differs"

    $tempSystemGame = Join-Path ([System.IO.Path]::GetTempPath()) ("emuera-system-oracle-" + [guid]::NewGuid())
    Copy-Item "$PSScriptRoot/fixture" $tempSystemGame -Recurse
    Copy-Item "$PSScriptRoot/fixture-system/*" $tempSystemGame -Recurse -Force
    $systemLoad = Invoke-Oracle @{ id = "system-load"; op = "load"; gameDir = $tempSystemGame }
    Assert-True ($systemLoad.ok -and $systemLoad.result.termination -eq "error") "system fixture did not reproduce the reference STOPCALLTRAIN error"
    $stopCallTrain = Invoke-Oracle @{ id = "stopcalltrain"; op = "run"; watch = @("RESULT:30", "RESULT:31") }
    Assert-True ($stopCallTrain.ok -and $stopCallTrain.result.termination -eq "error") "STOPCALLTRAIN reference termination differs"
    Assert-True (($stopCallTrain.result.watches.'RESULT:30' -eq 0) -and
        ($stopCallTrain.result.watches.'RESULT:31' -eq 1)) "STOPCALLTRAIN did not discard its caller before CALLTRAINEND"

    $reset = Invoke-Oracle @{ id = 11; op = "reset" }
    Assert-True $reset.ok "reset failed"
    Write-Host "Emuera reference CLI smoke test passed."
}
finally {
    if (-not $process.HasExited) {
        $process.StandardInput.Close()
        if (-not $process.WaitForExit(2000)) { $process.Kill($true) }
    }
    $process.Dispose()
    if ($null -ne $tempGame -and [System.IO.Directory]::Exists($tempGame)) {
        Remove-Item $tempGame -Recurse -Force
    }
    if ($null -ne $tempSystemGame -and [System.IO.Directory]::Exists($tempSystemGame)) {
        Remove-Item $tempSystemGame -Recurse -Force
    }
    if ($null -ne $tempOneInputGame -and [System.IO.Directory]::Exists($tempOneInputGame)) {
        Remove-Item $tempOneInputGame -Recurse -Force
    }
    if ($null -ne $tempOneInputLongGame -and [System.IO.Directory]::Exists($tempOneInputLongGame)) {
        Remove-Item $tempOneInputLongGame -Recurse -Force
    }
}
