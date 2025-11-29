@echo off
setlocal

set "VERSION=%~1"
if "%VERSION%"=="" (
    set "VERSION=0.0.0"
)

set "EXPORT_DIR=%CD%\export"
if not exist "%EXPORT_DIR%" (
    mkdir "%EXPORT_DIR%"
)

set "EXE_SOURCE=target\release\hang-client.exe"
set "EXPORTED_EXE=Hang %VERSION%.exe"
set "MSI_NAME=Hang %VERSION%.msi"
set "MSI_BUILT=0"
set "PAYLOAD_DIR=%EXPORT_DIR%\payload"

echo Building Hang client (version %VERSION%)...
echo.


cargo build --release -p hang-client
if %errorlevel% neq 0 (
    echo Client build failed!
    exit /b 1
)

powershell -NoProfile -ExecutionPolicy Bypass -File installer\prepare_payload.ps1 -Payload "%PAYLOAD_DIR%" -Version "%VERSION%"
if %errorlevel% neq 0 (
    echo Failed to prepare payload!
    exit /b 1
)

copy /Y "%PAYLOAD_DIR%\Hang.exe" "%EXPORT_DIR%\%EXPORTED_EXE%" >nul
if %errorlevel% neq 0 (
    echo Failed to copy client executable to export folder!
    exit /b 1
)

echo Packaging MSI: %MSI_NAME%
where candle.exe >nul 2>&1
if %errorlevel% neq 0 (
    echo WiX Toolset (candle.exe/light.exe) not found. Skipping MSI packaging.
    goto finish
)

powershell -NoProfile -ExecutionPolicy Bypass -File installer\build_installer.ps1 -Version "%VERSION%" -PayloadDir "%PAYLOAD_DIR%" -Output "%EXPORT_DIR%\%MSI_NAME%"
if %errorlevel% neq 0 (
    echo MSI packaging failed!
    exit /b 1
)
set "MSI_BUILT=1"

goto finish

:finish
echo.
echo ========================================
echo Build Complete!
echo ========================================
echo.
echo Client EXE  : %EXE_SOURCE%
echo Exported EXE: %EXPORT_DIR%\%EXPORTED_EXE%
if "%MSI_BUILT%"=="1" (
    echo Installer   : %EXPORT_DIR%\%MSI_NAME%
) else (
    echo Installer   : (not created - WiX toolset missing)
)
echo.
endlocal
