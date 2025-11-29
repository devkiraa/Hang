@echo off
echo Building Hang Client...
echo.

cd client
cargo build --release
if %errorlevel% neq 0 (
    echo Client build failed!
    pause
    exit /b 1
)
cd ..

echo.
echo ========================================
echo Build Complete!
echo ========================================
echo.
echo Client: client\target\release\hang-client.exe
echo.
pause
