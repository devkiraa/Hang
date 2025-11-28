@echo off
echo Building Hang Sync Player...
echo.

echo [1/2] Building Server...
cd server
cargo build --release
if %errorlevel% neq 0 (
    echo Server build failed!
    pause
    exit /b 1
)
cd ..

echo.
echo [2/2] Building Client...
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
echo Server: server\target\release\hang-server.exe
echo Client: client\target\release\hang-client.exe
echo.
pause
