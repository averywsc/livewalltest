<#
Measures CPU, GPU and memory of one app for a number of seconds.
Examples:
  .\tools\measure.ps1 -Process livewall -Label playing-1080p
  .\tools\measure.ps1 -Process livewall -Label paused          (pause it first)
  .\tools\measure.ps1 -Process wallpaper64,wallpaper32,webwallpaper32 -Label wallpaper-engine
CPU is a percentage of the whole processor (like Task Manager's total). GPU is the busiest
engine of the process (3D, video decode, copy), like Task Manager's GPU column.
#>
param(
    [Parameter(Mandatory = $true)][string[]]$Process,
    [int]$Seconds = 60,
    [string]$Label = "run"
)
$ErrorActionPreference = "Stop"
$cores = [Environment]::ProcessorCount
$rows = @()
$prevCpu = @{}
$prevT = Get-Date

function Get-Pids { (Get-Process -Name $Process -ErrorAction SilentlyContinue).Id }

if (-not (Get-Pids)) { throw "No running process named: $($Process -join ', ')" }
Write-Host "Measuring $($Process -join ', ') for $Seconds seconds ($Label)..."

for ($i = 0; $i -lt $Seconds; $i++) {
    $pids = Get-Pids
    $procs = Get-Process -Id $pids -ErrorAction SilentlyContinue
    $now = Get-Date
    $dt = ($now - $prevT).TotalSeconds
    $cpuDelta = 0.0
    foreach ($p in $procs) {
        $t = $p.TotalProcessorTime.TotalSeconds
        if ($prevCpu.ContainsKey($p.Id)) { $cpuDelta += [math]::Max(0, $t - $prevCpu[$p.Id]) }
        $prevCpu[$p.Id] = $t
    }
    $prevT = $now
    $cpu = if ($i -gt 0 -and $dt -gt 0) { 100.0 * $cpuDelta / ($dt * $cores) } else { 0.0 }

    $gpu = 0.0
    try {
        $samples = (Get-Counter -Counter "\GPU Engine(*)\Utilization Percentage" -ErrorAction Stop).CounterSamples
        $byType = @{}
        foreach ($s in $samples) {
            foreach ($id in $pids) {
                if ($s.InstanceName -like "pid_${id}_*") {
                    $type = ($s.InstanceName -split "engtype_")[-1]
                    $byType[$type] = [double]$byType[$type] + $s.CookedValue
                }
            }
        }
        if ($byType.Count) { $gpu = ($byType.Values | Measure-Object -Maximum).Maximum }
    } catch { $gpu = [double]::NaN }

    $ws = ($procs | Measure-Object WorkingSet64 -Sum).Sum / 1MB
    $priv = ($procs | Measure-Object PrivateMemorySize64 -Sum).Sum / 1MB
    $rows += [pscustomobject]@{ Second = $i; CpuPercent = [math]::Round($cpu, 2); GpuPercent = [math]::Round($gpu, 2); WorkingSetMB = [math]::Round($ws, 1); PrivateMB = [math]::Round($priv, 1) }
    if ($i -lt $Seconds - 1) { Start-Sleep -Milliseconds 800 }
}

$valid = $rows | Select-Object -Skip 1
$summary = [pscustomobject]@{
    Label        = $Label
    Seconds      = $Seconds
    CpuAvg       = [math]::Round(($valid | Measure-Object CpuPercent -Average).Average, 2)
    CpuMax       = ($valid | Measure-Object CpuPercent -Maximum).Maximum
    GpuAvg       = [math]::Round(($valid | Where-Object { -not [double]::IsNaN($_.GpuPercent) } | Measure-Object GpuPercent -Average).Average, 2)
    GpuMax       = ($valid | Measure-Object GpuPercent -Maximum).Maximum
    WorkingSetMB = [math]::Round(($valid | Measure-Object WorkingSetMB -Average).Average, 1)
    PrivateMB    = [math]::Round(($valid | Measure-Object PrivateMB -Average).Average, 1)
}
$csv = "measure-$Label.csv"
$rows | Export-Csv $csv -NoTypeInformation
$summary | Format-List
Write-Host "Per-second data saved to $csv"
