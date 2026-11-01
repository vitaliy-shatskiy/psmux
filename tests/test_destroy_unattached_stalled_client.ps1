# destroy-unattached and a client that stops reading (the PC froze, the
# terminal hung): the server's writer drops the connection after its 5 s
# write timeout, which used to count as the last detach and end the session
# while the client process was alive and about to reconnect.
#
# Each case runs a hand made client in its own process (AUTH, PERSISTENT,
# client-attach, then it reads or it does not), so the test can end that
# process and watch what the server does.
#
# Runs only under a unique -L namespace (AGENTS.md); the default namespace
# session list is compared before and after.
param([string]$Psmux = '')
$ErrorActionPreference = 'Continue'
if (-not $Psmux) {
    $Psmux = (Get-Command psmux -ErrorAction SilentlyContinue).Source
    if (-not $Psmux) { Write-Host 'psmux not found'; exit 2 }
}
$script:pass = 0
$script:fail = 0
function Pass([string]$m) { Write-Host "  [PASS] $m" -ForegroundColor Green; $script:pass++ }
function Fail([string]$m) { Write-Host "  [FAIL] $m" -ForegroundColor Red; $script:fail++ }

$clientScript = Join-Path ([IO.Path]::GetTempPath()) ("psmux_du_client_" + [guid]::NewGuid().ToString('N') + '.ps1')
@'
param([int]$Port, [string]$Key, [ValidateSet('stall', 'drain')][string]$Mode)
$c = [Net.Sockets.TcpClient]::new()
$c.ReceiveBufferSize = 4096
$c.Connect('127.0.0.1', $Port)
$st = $c.GetStream()
$w = [IO.StreamWriter]::new($st); $w.NewLine = "`n"; $w.AutoFlush = $true
$w.WriteLine("AUTH $Key")
$buf = [byte[]]::new(65536)
$st.ReadTimeout = 3000
$null = $st.Read($buf, 0, $buf.Length)
$w.WriteLine('PERSISTENT')
$w.WriteLine('client-attach features=0x0')
if ($Mode -eq 'stall') { Start-Sleep -Seconds 600 }
$st.ReadTimeout = 200
while ($true) { try { if ($st.Read($buf, 0, $buf.Length) -le 0) { break } } catch { } }
'@ | Set-Content -LiteralPath $clientScript -Encoding UTF8

$defBefore = (& $Psmux ls 2>&1 | Out-String).Trim()

function New-Ns { 'testdu' + [guid]::NewGuid().ToString('N').Substring(0, 8) }
function Ns([string]$ns, [string[]]$a) { (& $Psmux -L $ns @a 2>&1 | Out-String).Trim() }
function Alive([string]$ns) { & $Psmux -L $ns has-session -t t 2>$null | Out-Null; $LASTEXITCODE -eq 0 }
function Clients([string]$ns) { @((Ns $ns @('list-clients')) -split "`n" | Where-Object { $_.Trim() }).Count }

function Start-Session([string]$ns, [string]$du) {
    # A pane that floods output, so a client that stops reading fills its
    # socket and the server's write times out.
    Ns $ns @('new-session', '-d', '-s', 't', '--', 'pwsh', '-NoProfile', '-Command', 'while(1){Get-Date -Format o; Start-Sleep -Milliseconds 5}') | Out-Null
    Start-Sleep -Seconds 3
    Ns $ns @('set', '-g', 'destroy-unattached', $du) | Out-Null
}
function Start-Client([string]$ns, [string]$mode) {
    $base = Join-Path $HOME ".psmux\${ns}__t"
    $port = (Get-Content -LiteralPath "$base.port" -Raw).Trim()
    $key = (Get-Content -LiteralPath "$base.key" -Raw).Trim()
    $p = Start-Process pwsh -ArgumentList @('-NoProfile', '-File', $clientScript, '-Port', $port, '-Key', $key, '-Mode', $mode) -WindowStyle Hidden -PassThru
    Start-Sleep -Milliseconds 1500
    $p
}
# Wait until the server has dropped every client (the write timeout fired).
function Wait-Dropped([string]$ns, [int]$want, [int]$sec = 25) {
    $end = (Get-Date).AddSeconds($sec)
    while ((Get-Date) -lt $end) {
        if (-not (Alive $ns)) { return 'gone' }
        if ((Clients $ns) -le $want) { return 'dropped' }
        Start-Sleep -Milliseconds 500
    }
    'timeout'
}
function Wait-Gone([string]$ns, [int]$sec) {
    $end = (Get-Date).AddSeconds($sec)
    while ((Get-Date) -lt $end) { if (-not (Alive $ns)) { return $true }; Start-Sleep -Milliseconds 250 }
    $false
}
function Stop-Client($p) { if ($p -and -not $p.HasExited) { Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue; $p.WaitForExit(5000) | Out-Null } }
function Cleanup([string]$ns, $procs) { foreach ($p in $procs) { Stop-Client $p }; & $Psmux -L $ns kill-server 2>$null | Out-Null }

Write-Host "psmux: $Psmux"

Write-Host "`n[1,3] last client stalls, its process alive; then it exits"
$ns = New-Ns; Start-Session $ns 'on'
$a = Start-Client $ns 'stall'
$r = Wait-Dropped $ns 0
Start-Sleep -Seconds 2
if ($r -eq 'dropped' -and (Alive $ns)) { Pass "session kept after the stalled client was dropped" } else { Fail "session gone after the stall drop ($r)" }
Stop-Client $a
if (Wait-Gone $ns 4) { Pass "session destroyed once the waiting client exited" } else { Fail "session lingers after the waiting client exited" }
Cleanup $ns @($a)

Write-Host "`n[2] stalled client's process alive, a client reconnects"
$ns = New-Ns; Start-Session $ns 'on'
$a = Start-Client $ns 'stall'
$r = Wait-Dropped $ns 0
$b = Start-Client $ns 'drain'
Stop-Client $a
Start-Sleep -Seconds 3
if ($r -eq 'dropped' -and (Alive $ns) -and (Clients $ns) -eq 1) { Pass "reconnect ended the wait, session kept" } else { Fail "session lost around the reconnect ($r)" }
Stop-Client $b
if (Wait-Gone $ns 4) { Pass "session destroyed when the reconnected client exited" } else { Fail "session lingers after the reconnected client exited" }
Cleanup $ns @($a, $b)

Write-Host "`n[4] client closes with no stall"
$ns = New-Ns; Start-Session $ns 'on'
$a = Start-Client $ns 'drain'
if ((Clients $ns) -eq 1) { Pass "client attached" } else { Fail "client did not attach" }
Stop-Client $a
if (Wait-Gone $ns 3) { Pass "session destroyed within 3 s of the client exit" } else { Fail "session kept after the client exited" }
Cleanup $ns @($a)

Write-Host "`n[5] detach-client"
$ns = New-Ns; Start-Session $ns 'on'
$a = Start-Client $ns 'drain'
Ns $ns @('detach-client') | Out-Null
if (Wait-Gone $ns 3) { Pass "session destroyed after detach-client" } else { Fail "session kept after detach-client" }
Cleanup $ns @($a)

Write-Host "`n[7] destroy-unattached off"
$ns = New-Ns; Start-Session $ns 'off'
$a = Start-Client $ns 'stall'
$null = Wait-Dropped $ns 0
Stop-Client $a
Start-Sleep -Seconds 3
if (Alive $ns) { Pass "session kept with destroy-unattached off" } else { Fail "session destroyed with destroy-unattached off" }
Cleanup $ns @($a)

Write-Host "`n[8] two clients, one stalls"
$ns = New-Ns; Start-Session $ns 'on'
$b = Start-Client $ns 'drain'
$a = Start-Client $ns 'stall'
$r = Wait-Dropped $ns 1
Stop-Client $a
Start-Sleep -Seconds 3
if ($r -eq 'dropped' -and (Alive $ns)) { Pass "session kept while another client stays" } else { Fail "session lost with a client still attached ($r)" }
Cleanup $ns @($a, $b)

Remove-Item -LiteralPath $clientScript -ErrorAction SilentlyContinue
$defAfter = (& $Psmux ls 2>&1 | Out-String).Trim()
if ($defBefore -eq $defAfter) { Pass "default namespace unchanged" } else { Fail "default namespace changed"; Write-Host "BEFORE: $defBefore"; Write-Host "AFTER: $defAfter" }

Write-Host "`nPassed: $script:pass  Failed: $script:fail"
if ($script:fail -gt 0) { exit 1 } else { exit 0 }
