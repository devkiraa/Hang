@echo off
echo Starting Hang Server...
cd server
start "Hang Server" cargo run --release
echo Server started in new window
timeout /t 2 /nobreak > nul
cd ..
