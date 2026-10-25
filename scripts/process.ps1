<#
.SYNOPSIS
  Runs a program with its output in log files and waits for that program
  alone (dot-sourced by verify.ps1 and verify-linux.ps1).

.DESCRIPTION
  Start-Process keeps a program's output as it is (UTF-8, no error records
  around the lines of stderr) and gives its exit code. Its -Wait, in
  Windows PowerShell 5.1, also waits for every process the program started
  and left behind: the linker of MSVC starts vctip.exe (Visual C++
  telemetry), which can hang for a quarter of an hour after cargo is done,
  and a step of verify.ps1 stood still that long (run 12). So the program
  is started without -Wait and only its own exit is awaited.

  Start-Process joins the arguments with spaces, so one with a space (a
  path) is quoted. A program that does not even start gives -1, never a
  pass.
#>

function Invoke-Logged([string] $FilePath, [string[]] $Arguments, [string] $Log, [string] $ErrorLog) {
    $quoted = @($Arguments | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } })
    try {
        $process = Start-Process -FilePath $FilePath -ArgumentList $quoted -NoNewWindow -PassThru `
            -RedirectStandardOutput $Log -RedirectStandardError $ErrorLog -ErrorAction Stop
    } catch {
        Add-Content -LiteralPath $ErrorLog -Value "could not start ${FilePath}: $($_.Exception.Message)"
        return -1
    }
    # The handle first: a Process that Start-Process returned loses the exit
    # code of a program that ended before anyone held its handle.
    $null = $process.Handle
    $process.WaitForExit()
    return $process.ExitCode
}
