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

    $analyzed = Invoke-Oracle @{ id = 7; op = "analyzeLine"; source = "RESULT = 9" }
    Assert-True $analyzed.ok "semantic line analysis failed"
    Assert-True ($null -ne $analyzed.result.argument) "semantic argument was not produced"

    $executed = Invoke-Oracle @{ id = 8; op = "execute"; statement = "RESULT = 9"; watch = @("RESULT") }
    Assert-True $executed.ok "single-instruction execution failed"
    Assert-True ($executed.result.watches.RESULT -eq 9) "execute did not update RESULT"

    $run = Invoke-Oracle @{ id = 9; op = "run"; entry = "ORACLE_TEST"; watch = @("RESULT") }
    Assert-True $run.ok "isolated function run failed"
    Assert-True ($run.result.termination -eq "completed") "function did not complete"
    Assert-True (($run.result.output -join "`n") -match "ORACLE_OK") "function output missing"

    $inputRun = Invoke-Oracle @{ id = 10; op = "run"; entry = "ORACLE_INPUT"; inputs = @("42"); watch = @("RESULT") }
    Assert-True $inputRun.ok "input function run failed"
    Assert-True ($inputRun.result.termination -eq "completed") "input function did not complete"
    Assert-True ($inputRun.result.watches.RESULT -eq 42) "input did not update RESULT"

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
}
