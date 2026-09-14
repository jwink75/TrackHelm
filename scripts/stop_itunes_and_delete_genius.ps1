# iTunes Killer and Genius Database Cleaner
[CmdletBinding()]
param(
    [switch]$Silent
)

$host.UI.RawUI.WindowTitle = "iTunes & Genius Database Cleaner"

Write-Host "========================================================" -ForegroundColor Cyan
Write-Host "       iTunes Stopper & Genius Database Cleaner" -ForegroundColor Cyan
Write-Host "========================================================`n" -ForegroundColor Cyan

# 1. Stop iTunes processes
Write-Host "[1/2] Terminating iTunes processes..." -ForegroundColor Yellow

$targetProcesses = @(
    "iTunes",
    "AppleMobileDeviceProcess",
    "AppleMobileDeviceHelper",
    "AppleMobileDeviceService",
    "distnoted",
    "iTunesHelper",
    "iPodService",
    "APSDaemon"
)

$matched = Get-Process -ErrorAction SilentlyContinue | Where-Object {
    $targetProcesses -contains $_.ProcessName -or
    ($_.Path -and ($_.Path -like "*AppleInc.iTunes*" -or $_.Path -like "*\iTunes\*"))
}

if ($matched) {
    foreach ($p in $matched) {
        Write-Host "  - Stopping $($p.ProcessName) (PID $($p.Id))..." -ForegroundColor DarkYellow
        try {
            Stop-Process -Id $p.Id -Force -ErrorAction Stop
        } catch {
            Write-Warning "Could not stop PID $($p.Id): $($_.Exception.Message)"
        }
    }
} else {
    Write-Host "  - No running iTunes processes found." -ForegroundColor DarkGray
}

Write-Host "  - Waiting for OS file handles to release..." -ForegroundColor DarkGray
Start-Sleep -Seconds 2

# 2. Delete the Genius database
$targetFile = "F:\Music\iTunes\iTunes Library Genius.itdb"
Write-Host "`n[2/2] Checking target file:" -ForegroundColor Yellow
Write-Host "      $targetFile`n"

if (-not (Test-Path $targetFile)) {
    Write-Host "[INFO] The file does not exist or has already been removed." -ForegroundColor Cyan
} else {
    try {
        # Clear attributes if read-only/hidden
        Set-ItemProperty -Path $targetFile -Name Attributes -Value Normal -ErrorAction SilentlyContinue
        Remove-Item -Path $targetFile -Force -ErrorAction Stop
        Write-Host "[SUCCESS] 'iTunes Library Genius.itdb' was successfully deleted!" -ForegroundColor Green
    } catch {
        Write-Host "[WAIT] File lock lingered, retrying in 2 seconds..." -ForegroundColor DarkYellow
        Start-Sleep -Seconds 2
        try {
            Remove-Item -Path $targetFile -Force -ErrorAction Stop
            Write-Host "[SUCCESS] 'iTunes Library Genius.itdb' was successfully deleted on retry!" -ForegroundColor Green
        } catch {
            Write-Host "`n[ERROR] Failed to delete file: $($_.Exception.Message)" -ForegroundColor Red
            Write-Host "Try running PowerShell as Administrator if an Apple background service is locking it." -ForegroundColor Yellow
        }
    }
}

Write-Host "`n========================================================" -ForegroundColor Cyan

if (-not $Silent) {
    Write-Host "Press any key to close this window..."
    $null = $host.UI.RawUI.ReadKey("NoEcho,IncludeKeyDown")
}
