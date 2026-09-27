# M1 性能基线：在夹具仓库上跑探针，汇总成 markdown 表格（T1.12 第 4 条）。
#
# 为什么要这么绕：探针只负责量"服务层耗时"，而**内存与 CPU 必须从进程外部取样**
# （Windows 上没有 portable 的 getrusage）。因此这里用 Start-Process 起探针，
# 轮询 Process.WorkingSet64 记录峰值，再读取 TotalProcessorTime 得到 CPU 时间。
#
# 用法：
#   pwsh scripts/perf/run-baseline.ps1            # 默认跑 target/perf 下的夹具
#   pwsh scripts/perf/run-baseline.ps1 -WatchSeconds 10
param(
    [int]$WatchSeconds = 0,
    [string]$PerfRoot = (Join-Path (Resolve-Path "$PSScriptRoot\..\..") 'target\perf'),
    [string]$Probe = (Join-Path (Resolve-Path "$PSScriptRoot\..\..") 'target\release\examples\perf_probe.exe')
)

if (-not (Test-Path $Probe)) {
    Write-Error "找不到探针：$Probe（先跑 cargo build --release -p forgedesk-services --example perf_probe）"
    exit 1
}

function Measure-Repo {
    param([string]$Name, [string]$Path, [int]$WatchSeconds)

    $arguments = @($Path)
    if ($WatchSeconds -gt 0) { $arguments += @('--watch-seconds', "$WatchSeconds") }

    $process = Start-Process -FilePath $Probe -ArgumentList $arguments -NoNewWindow -PassThru `
        -RedirectStandardOutput (Join-Path $env:TEMP "perf-$Name.out") `
        -RedirectStandardError (Join-Path $env:TEMP "perf-$Name.err")

    $peakBytes = 0
    while (-not $process.HasExited) {
        try {
            $process.Refresh()
            if ($process.WorkingSet64 -gt $peakBytes) { $peakBytes = $process.WorkingSet64 }
        } catch {
            # 进程刚好退出：忽略
        }
        Start-Sleep -Milliseconds 40
    }

    $cpuMs = [math]::Round($process.TotalProcessorTime.TotalMilliseconds, 0)
    $json = (Get-Content (Join-Path $env:TEMP "perf-$Name.out") -Raw -ErrorAction SilentlyContinue)
    $cpuDeltaMs = $null
    if ($WatchSeconds -gt 0) { $cpuDeltaMs = $cpuMs }

    [pscustomobject]@{
        Name       = $Name
        Repo       = $Path
        Json       = $json
        PeakMb     = [math]::Round($peakBytes / 1MB, 1)
        CpuMs      = $cpuMs
        WatchCpuMs = $cpuDeltaMs
    }
}

$fixtures = Get-ChildItem -Path $PerfRoot -Directory | Sort-Object Name
$rows = @()
foreach ($fixture in $fixtures) {
    Write-Host "== $($fixture.Name)"
    $measured = Measure-Repo -Name $fixture.Name -Path $fixture.FullName -WatchSeconds $WatchSeconds
    Write-Host "   $($measured.Json) 峰值内存 $($measured.PeakMb)MB CPU $($measured.CpuMs)ms"
    $rows += $measured
}

# 真实仓库（本仓库）也量一遍：夹具再好也不代表真实的树形状
$self = (Resolve-Path "$PSScriptRoot\..\..").Path
Write-Host "== forgedesk (self)"
$measuredSelf = Measure-Repo -Name 'forgedesk' -Path $self -WatchSeconds $WatchSeconds
Write-Host "   $($measuredSelf.Json) 峰值内存 $($measuredSelf.PeakMb)MB CPU $($measuredSelf.CpuMs)ms"
$rows += $measuredSelf

$outFile = Join-Path $PerfRoot 'baseline.json'
$rows | ConvertTo-Json -Depth 4 | Set-Content -Path $outFile -Encoding utf8
Write-Host "已写入 $outFile"
