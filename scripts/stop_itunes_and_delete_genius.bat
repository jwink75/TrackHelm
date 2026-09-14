@echo off
setlocal enabledelayedexpansion

title iTunes ^& Genius Database Cleaner

echo ========================================================
echo        iTunes Stopper ^& Genius Database Cleaner
echo ========================================================
echo.

:: ---------------------------------------------------------
:: 1. Stop iTunes and related processes
:: ---------------------------------------------------------
echo [1/2] Terminating iTunes processes...

set "FOUND_ANY=0"
for %%P in (iTunes.exe AppleMobileDeviceProcess.exe AppleMobileDeviceHelper.exe AppleMobileDeviceService.exe distnoted.exe iTunesHelper.exe iPodService.exe APSDaemon.exe) do (
    tasklist /FI "IMAGENAME eq %%P" 2>nul | find /I "%%P" >nul
    if !errorlevel! equ 0 (
        echo   - Stopping %%P...
        taskkill /F /T /IM %%P >nul 2>&1
        set "FOUND_ANY=1"
    )
)

if "!FOUND_ANY!"=="0" (
    echo   - No running iTunes processes detected.
) else (
    echo   - Process termination signals sent.
)

:: Wait 2 seconds to ensure file locks are released by the OS
echo   - Waiting for OS file handles to release...
ping 127.0.0.1 -n 3 >nul

echo.
:: ---------------------------------------------------------
:: 2. Delete iTunes Library Genius.itdb
:: ---------------------------------------------------------
set "TARGET_FILE=F:\Music\iTunes\iTunes Library Genius.itdb"

echo [2/2] Checking target file:
echo       "%TARGET_FILE%"
echo.

if not exist "%TARGET_FILE%" (
    echo [INFO] The file does not exist or has already been removed.
) else (
    :: Clear potential Read-Only, System, or Hidden attributes
    attrib -r -s -h "%TARGET_FILE%" >nul 2>&1
    
    echo Attempting deletion...
    del /f /q "%TARGET_FILE%" >nul 2>&1

    :: Retry if file lock lingered
    if exist "%TARGET_FILE%" (
        echo [WAIT] File still appears locked. Retrying in 2 seconds...
        ping 127.0.0.1 -n 3 >nul
        del /f /q "%TARGET_FILE%" >nul 2>&1
    )

    if exist "%TARGET_FILE%" (
        echo.
        echo [ERROR] Could not delete the file:
        echo         "%TARGET_FILE%"
        echo.
        echo Possible reasons:
        echo   1. A background process is still holding a lock.
        echo   2. Administrator privileges may be required.
        echo      -^> Try right-clicking this batch file and choosing "Run as administrator".
    ) else (
        echo.
        echo [SUCCESS] "iTunes Library Genius.itdb" was successfully deleted!
    )
)

echo.
echo ========================================================
:: Support silent execution if passed /q or /silent
if /I "%~1"=="/silent" goto :eof
if /I "%~1"=="-silent" goto :eof
if /I "%~1"=="/q" goto :eof
if /I "%~1"=="-q" goto :eof

pause
