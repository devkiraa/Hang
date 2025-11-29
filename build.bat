@echo off
setlocal

set VERSION=%1
if "%VERSION%"=="" (
    set VERSION=0.0.0
)

echo Building Hang client (version %VERSION%)...
echo.

cargo build --release -p hang-client
if %errorlevel% neq 0 (
    echo Client build failed!
    exit /b 1
)

set MSI_NAME=Hang %VERSION%.msi
echo Packaging MSI: %MSI_NAME%
where candle.exe >nul 2>&1
if %errorlevel% neq 0 (
    echo WiX Toolset (candle.exe/light.exe) not found. Skipping MSI packaging.
    goto finish
)

powershell -NoProfile -ExecutionPolicy Bypass -File installer\build_installer.ps1 -Version "%VERSION%" -Output "%MSI_NAME%"
if %errorlevel% neq 0 (
    echo MSI packaging failed!
    exit /b 1
)

goto finish

:finish
echo.
echo ========================================
echo Build Complete!
echo ========================================
echo.
echo Client EXE : target\release\hang-client.exe
if exist "%MSI_NAME%" (
    echo Installer  : %MSI_NAME%
) else (
    echo Installer  : (not created - WiX toolset missing)
)
echo.
endlocal
