# Hang - Quick Start

## Prerequisites Setup

### 1. Install Rust
```powershell
# Download and run from: https://rustup.rs/
# Or use winget:
winget install Rustlang.Rustup
```

### 2. Install VLC (Required for video playback)

**Option A: VideoLAN Installer (Recommended)**
1. Download the 64-bit installer from https://www.videolan.org/vlc/
2. Run the installer and keep the default destination `C:\Program Files\VideoLAN\VLC`
3. Add the install folder to PATH so `libvlc.dll` can be discovered:
```powershell
$env:Path += ";C:\Program Files\VideoLAN\VLC"
[Environment]::SetEnvironmentVariable("Path", $env:Path, [EnvironmentVariableTarget]::User)
```

**Option B: Chocolatey**
```powershell
choco install vlc
```

### 3. Install Visual Studio Build Tools
```powershell
# Download from: https://visualstudio.microsoft.com/downloads/
# Or use winget:
winget install Microsoft.VisualStudio.2022.BuildTools
# Select "Desktop development with C++" workload
```

## Building

```powershell
cd D:\Repository\Hang\hang-sync-player
.\build.bat
```

## Running

### Quick Start (Single Machine Test)

**Terminal 1 - Start Server:**
```powershell
.\run_server.bat
```

**Terminal 2 - Start Client:**
```powershell
.\run_client.bat
```

### Multi-User Setup

1. **Server Machine:**
   ```powershell
   cd server
   cargo run --release
   # Note the IP address
   ```

2. **Each Client Machine:**
   ```powershell
   cd client
   cargo run --release
   # In UI, set Server URL to: ws://SERVER_IP:3005
   ```

## Usage Flow

1. **Open Video**: Click "Open Video" and select a video file
2. **Create Room**: Click "Create Room" button
3. **Share Room ID**: Copy the generated UUID and share with friends
4. **Others Join**: 
   - They open the **same video file**
   - Enter the Room ID
   - Click "Join Room"
5. **Watch Together**: All playback actions sync automatically!

## Troubleshooting

### libvlc.dll not found
```powershell
# Verify VLC installation
"C:\Program Files\VideoLAN\VLC\vlc.exe" --version

# If not found, add to PATH manually or set:
$env:LIBVLC_PATH = "C:\Program Files\VideoLAN\VLC\libvlc.dll"
```

### Build Errors
```powershell
# Clean and rebuild
cd server
cargo clean
cargo build --release

cd ..\client
cargo clean
cargo build --release
```

### Connection Failed
- Ensure server is running first
- Check firewall allows port 3005
- For remote servers, use IP address instead of localhost

## Helpful VLC Hotkeys (Standalone Player)

- `Space`: Play/Pause
- `Ctrl + → / ←`: Seek ±1 minute
- `Shift + → / ←`: Seek ±3 seconds
- `=` / `-`: Increase/Decrease playback speed
- `Ctrl + Up/Down`: Volume ±5%
- `F`: Toggle fullscreen

## File Hash Requirement

All participants must have **byte-identical** video files:
- Same source file copied to each machine
- Do NOT re-encode or convert
- Filename can differ, but content must match

## Performance

- Tested with groups up to 10+ clients
- Sub-100ms sync latency on LAN
- Recommended: Stable internet connection (1+ Mbps)
- Video stays local - only control commands sent over network

## Next Steps

- See full README.md for detailed documentation
- Check protocol.rs for message format details
- Customize UI in client/src/ui.rs
- Extend server logic in server/src/main.rs

Enjoy watching together! 🎬
