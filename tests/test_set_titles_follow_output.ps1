# set-titles: the host terminal's title follows the pane title while the pane
# keeps printing.
#
# WHAT WAS WRONG
# --------------
# `set-titles-string "#{pane_title}"` expands `pane.title`, which follows the
# program's OSC 0/2 title only through `propagate_osc_titles`. The server's
# pushed frame never called it; only the dump-state reply did. An attached
# client stamps its idle clock on every frame it draws, so while the pane
# printed (Claude Code's spinner) the client never sent that dump-state, and
# the tab kept the old title until the pane went quiet for a second.
#
# THE CHECK
# ---------
# The pane sets TTL_1, TTL_2, TTL_3 through OSC 2, each followed by six seconds
# of a spinner redrawn every 100 ms, and records when it set each one. A real
# client attached in a hidden console turns host_title into an OSC 0, which the
# console host makes its title; the suite reads that title every 100 ms. Each
# title must reach it within $MaxLagMs of being set.
#
# Run: pwsh -NoProfile -File tests\test_set_titles_follow_output.ps1 [-Binary <psmux.exe>]

param(
    [string]$Binary = "",
    [int]$MaxLagMs = 1500
)

$ErrorActionPreference = "Continue"
$script:Passed = 0
$script:Failed = 0
function Write-Pass($m) { Write-Host "  [PASS] $m" -ForegroundColor Green; $script:Passed++ }
function Write-Fail($m) { Write-Host "  [FAIL] $m" -ForegroundColor Red; $script:Failed++ }
function Write-Info($m) { Write-Host "  [INFO] $m" -ForegroundColor DarkCyan }

if (-not $Binary) { $Binary = $env:PSMUX_TEST_EXE }
if (-not $Binary) { $Binary = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -EA SilentlyContinue).Path }
if (-not $Binary) { $Binary = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -EA SilentlyContinue).Path }
if (-not $Binary -or -not (Test-Path $Binary)) { Write-Host "psmux not found"; exit 2 }
Write-Info "binary under test: $Binary"

foreach ($v in @('PSMUX_SESSION','PSMUX_SESSION_NAME','PSMUX_PANE','PSMUX_PANE_ID','PSMUX_SOCKET','TMUX','TMUX_PANE','PSMUX','PSMUX_PTY_TRACE')) {
    Remove-Item "Env:\$v" -EA SilentlyContinue
}

$NS = "titles_$PID"
$SESS = "st"
$Changes = 3
$work = Join-Path $env:TEMP "psmux_titles_$PID"
New-Item -ItemType Directory -Force -Path $work | Out-Null
$exeName = [IO.Path]::GetFileName($Binary)
function P { & $Binary -L $NS @args 2>&1 | Out-String }

# ── the console title oracle ─────────────────────────────────────────────────
# A separate process, as in conread.cs: attaching to another console detaches
# the caller from its own.
$csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
if (-not (Test-Path $csc)) { $csc = Join-Path ([Runtime.InteropServices.RuntimeEnvironment]::GetRuntimeDirectory()) "csc.exe" }
$titleSrc = Join-Path $work "contitle.cs"
$titleExe = Join-Path $work "contitle.exe"
[IO.File]::WriteAllText($titleSrc, @'
using System;
using System.Runtime.InteropServices;
using System.Text;
class ConTitle {
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool FreeConsole();
    [DllImport("kernel32.dll", SetLastError = true)] static extern bool AttachConsole(uint pid);
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern uint GetConsoleTitleW(StringBuilder buf, uint size);
    static int Main(string[] args) {
        FreeConsole();
        if (!AttachConsole(uint.Parse(args[0]))) { return 2; }
        var sb = new StringBuilder(4096);
        GetConsoleTitleW(sb, (uint)sb.Capacity);
        FreeConsole();
        System.IO.File.WriteAllText(args[1], sb.ToString());
        return 0;
    }
}
'@)
& $csc /nologo /platform:x64 "/out:$titleExe" $titleSrc 2>&1 | Out-Null
if (-not (Test-Path $titleExe)) { Write-Host "could not build the console title reader"; exit 2 }

# ── the pane program ─────────────────────────────────────────────────────────
# A script file: an inline -Command would have its `$` expanded on the way in.
$paneScript = Join-Path $work "pane.ps1"
[IO.File]::WriteAllText($paneScript, @'
param([string]$Work, [int]$Changes)
$e = [char]27; $b = [char]7; $spin = '|/-\'
Start-Sleep 4
for ($i = 1; $i -le $Changes; $i++) {
    [Console]::Out.Write("$e]2;TTL_$i$b"); [Console]::Out.Flush()
    [IO.File]::WriteAllText((Join-Path $Work "emit_$i.txt"), [DateTime]::UtcNow.Ticks.ToString())
    for ($k = 0; $k -lt 60; $k++) {
        [Console]::Out.Write("`r" + $spin[$k % 4] + " $k   "); [Console]::Out.Flush()
        Start-Sleep -Milliseconds 100
    }
}
Start-Sleep 900
'@)

$defaultBefore = (& $Binary ls 2>&1 | Out-String)
$client = $null
try {
    P new-session -d -s $SESS -x 100 -y 30 pwsh -NoLogo -NoProfile -File $paneScript -Work $work -Changes $Changes | Out-Null
    $sw = [Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt 15000) {
        & $Binary -L $NS has-session -t $SESS 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) { break }
        Start-Sleep -Milliseconds 200
    }
    P set -g status off | Out-Null
    P set -g allow-set-title on | Out-Null
    P set -g set-titles on | Out-Null
    P set -g set-titles-string '#{pane_title}' | Out-Null

    # A real client needs a real console: its own cmd.exe, hidden.
    $launch = Join-Path $work "attach.cmd"
    [IO.File]::WriteAllText($launch, "@echo off`r`nset PSMUX_SESSION=`r`nset TMUX=`r`nset TMUX_PANE=`r`n`"$Binary`" -L $NS attach -t $SESS`r`n")
    $launcher = Start-Process cmd.exe -ArgumentList "/c", $launch -PassThru -WindowStyle Hidden
    $clientPid = 0
    while ($sw.ElapsedMilliseconds -lt 25000 -and $clientPid -eq 0) {
        Start-Sleep -Milliseconds 200
        $found = @(Get-CimInstance Win32_Process -Filter "ParentProcessId=$($launcher.Id)" -EA SilentlyContinue |
            Where-Object { $_.Name -eq $exeName } | Select-Object -ExpandProperty ProcessId)
        if ($found.Count -ge 1) { $clientPid = [int]$found[0] }
    }
    $client = [pscustomobject]@{ Launcher = $launcher; ClientPid = $clientPid }
    if ($clientPid -eq 0) { Write-Host "the client did not start"; exit 2 }

    # Each title's first sighting at the client, as UTC ticks.
    $seen = @{}
    $titleFile = Join-Path $work "title.txt"
    $deadline = [Diagnostics.Stopwatch]::StartNew()
    while ($deadline.ElapsedMilliseconds -lt (8000 + 6500 * $Changes) -and $seen.Count -lt $Changes) {
        & $titleExe $clientPid $titleFile | Out-Null
        $title = (Get-Content $titleFile -Raw -EA SilentlyContinue)
        if ($title -match '^TTL_(\d+)$' -and -not $seen.ContainsKey([int]$Matches[1])) {
            $seen[[int]$Matches[1]] = [DateTime]::UtcNow.Ticks
        }
        Start-Sleep -Milliseconds 100
    }

    for ($i = 1; $i -le $Changes; $i++) {
        $emitFile = Join-Path $work "emit_$i.txt"
        if (-not (Test-Path $emitFile)) { Write-Fail "the pane never set TTL_$i"; continue }
        $emitted = [long](Get-Content $emitFile -Raw)
        if (-not $seen.ContainsKey($i)) { Write-Fail "TTL_$i never reached the client's title while the pane printed"; continue }
        $lag = [int](($seen[$i] - $emitted) / 10000)
        if ($lag -le $MaxLagMs) { Write-Pass "TTL_$i reached the client's title in $lag ms" }
        else { Write-Fail "TTL_$i reached the client's title after $lag ms (limit $MaxLagMs)" }
    }
}
finally {
    if ($client) {
        if ($client.ClientPid -gt 0) { Stop-Process -Id $client.ClientPid -Force -EA SilentlyContinue }
        if (-not $client.Launcher.HasExited) { Stop-Process -Id $client.Launcher.Id -Force -EA SilentlyContinue }
    }
    & $Binary -L $NS kill-server 2>&1 | Out-Null
    $defaultAfter = (& $Binary ls 2>&1 | Out-String)
    if ($defaultAfter -ne $defaultBefore) { Write-Fail "the default namespace's session list changed during the run" }
    Remove-Item -LiteralPath $work -Recurse -Force -EA SilentlyContinue
}

Write-Host "`n  Passed: $script:Passed  Failed: $script:Failed"
exit $(if ($script:Failed -gt 0) { 1 } else { 0 })
