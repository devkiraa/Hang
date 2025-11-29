# Troubleshooting Guide

## Build Issues

### Error: "libvlc.dll not found" or "VLC runtime missing"

**Cause**: VLC (libVLC) is not installed or the DLL cannot be located at build/runtime.

**Solutions**:

1. **Install VLC**:
   ```powershell
   # Download the 64-bit build from https://www.videolan.org/vlc/
   # Accept the default path C:\Program Files\VideoLAN\VLC
   ```

2. **Add VLC to PATH** (so `libvlc.dll` can be discovered automatically):
   ```powershell
   $env:Path += ";C:\Program Files\VideoLAN\VLC"
   [Environment]::SetEnvironmentVariable("Path", $env:Path, [EnvironmentVariableTarget]::User)
   ```

3. **Set LIBVLC_PATH** (override path if VLC lives elsewhere):
   ```powershell
   $env:LIBVLC_PATH = "C:\Program Files\VideoLAN\VLC\libvlc.dll"
   cargo build --release
   ```

4. **Verify Installation**:
   ```powershell
   "C:\Program Files\VideoLAN\VLC\vlc.exe" --version
   # Or simply run: vlc --version (if PATH updated)
   ```

### Error: "link.exe not found" or MSVC linking errors

**Cause**: Visual Studio Build Tools not installed

**Solution**:
```powershell
# Install Visual Studio Build Tools
winget install Microsoft.VisualStudio.2022.BuildTools

# Or download from:
# https://visualstudio.microsoft.com/downloads/

# Select "Desktop development with C++" workload
```

### Error: "cargo: command not found"

**Cause**: Rust not installed or not in PATH

**Solution**:
```powershell
# Install Rust
winget install Rustlang.Rustup

# Or download from: https://rustup.rs/

# Restart terminal after installation
```

### Error: Multiple VLC installations found

**Cause**: Several VLC copies (Microsoft Store, standalone, portable) confuse dynamic loading.

**Solution**:
```powershell
# Find all VLC executables / libvlc.dll copies
where.exe vlc
Get-ChildItem -Recurse -Filter libvlc.dll -Path "C:\Program Files", "C:\Program Files (x86)"

# Remove/rename unwanted installations so only one remains

# Clear manual override
$env:LIBVLC_PATH = ""
# Point to the desired instance explicitly (optional)
$env:LIBVLC_PATH = "C:\Program Files\VideoLAN\VLC\libvlc.dll"
```

### Build succeeds but "error while loading shared libraries" at runtime

**Cause**: Runtime can't locate `libvlc.dll` (or its companion `plugins` folder).

**Solutions**:

1. **Copy DLL next to the executable** (quick test):
   ```powershell
   Copy-Item "C:\Program Files\VideoLAN\VLC\libvlc.dll" "client\target\release\"
   Copy-Item "C:\Program Files\VideoLAN\VLC\libvlccore.dll" "client\target\release\"
   ```

2. **Add VLC to PATH** (recommended):
   ```powershell
   [Environment]::SetEnvironmentVariable(
       "Path",
       $env:Path + ";C:\Program Files\VideoLAN\VLC",
       [EnvironmentVariableTarget]::User
   )
   ```

## Runtime Issues

### Client connects but "Connection Failed" immediately

**Symptoms**: Client shows "Connection Failed" after clicking Create/Join Room

**Causes & Solutions**:

1. **Server not running**:
   ```powershell
   # Terminal 1 - Start server FIRST
   cd server
   cargo run --release
   
   # Should see: "Hang Server listening on ws://0.0.0.0:3005"
   ```

2. **Wrong server URL**:
   - Default is `ws://localhost:3005`
   - For remote servers: `ws://SERVER_IP:3005`
   - Must use `ws://` not `http://`

3. **Firewall blocking port 3005**:
   ```powershell
   # Windows Firewall - allow port 3005
   netsh advfirewall firewall add rule name="Hang" dir=in action=allow protocol=TCP localport=3005
   ```

4. **Server crashed/exited**:
   - Check server terminal for error messages
   - Look for panic traces or connection errors

### "File Hash Mismatch" when joining room

**Cause**: Video files are different between host and guest

**What doesn't work**:
- ❌ Same movie but different files
- ❌ Re-encoded versions
- ❌ Different quality/resolution
- ❌ Downloaded from different sources

**What works**:
- ✅ Exact copy of same file
- ✅ Shared via USB/network share
- ✅ Same torrent download
- ✅ Same cloud storage link

**Verification**:
```powershell
# Check hash manually
$hash = Get-FileHash "movie.mp4" -Algorithm SHA256
$hash.Hash

# Both clients should show identical hash
```

**Temporary workaround** (NOT recommended):
- Modify server to skip hash check (removes sync guarantee)

### Video won't play / Black screen

**Symptoms**: UI works but video area is black

**Causes & Solutions**:

1. **Codec not supported**:
   ```powershell
   # Test file in standalone VLC
   "C:\Program Files\VideoLAN\VLC\vlc.exe" "C:\path\to\video.mp4"
   
   # If VLC can't play it, the client can't either
   # Convert video to H.264/AAC (MP4)
   ```

2. **File path has special characters**:
   ```powershell
   # Move file to simple path
   # Bad: C:\Users\John O'Brien\Videos\movie [1080p].mkv
   # Good: C:\Videos\movie.mp4
   ```

3. **File locked by another program**:
   ```powershell
   # Close other video players
   # Stop antivirus scans temporarily
   ```

4. **GPU driver issues**:
   ```powershell
   # Update graphics drivers
   # In VLC standalone: Tools → Preferences → Input/Codecs → Hardware-accelerated decoding
   #   Try toggling between Automatic / Direct3D11 / Disabled and retest
   ```

### Sync lag / Constant desync

**Symptoms**: Players drift apart over time, frequent resync needed

**Causes & Solutions**:

1. **High network latency**:
   ```powershell
   # Test ping to server
   ping SERVER_IP
   
   # Should be <100ms for good sync
   # >200ms will cause noticeable lag
   ```

2. **Different playback performance**:
   - One client dropping frames (weak CPU/GPU)
   - Different codec implementations
   
   **Solution**: Use simpler video format (H.264 1080p or lower)

3. **Sync threshold too loose**:
   - Edit `ui.rs` line ~320:
   ```rust
   if now.duration_since(*last_sync).as_millis() < 100 {
       // Change 100 to 50 for tighter sync (more network traffic)
   ```

4. **Clock drift**:
   ```powershell
   # Sync system clocks
   w32tm /resync
   ```

### "Room Not Found" error

**Causes & Solutions**:

1. **Typo in Room ID**:
   - Room IDs are UUIDs: `550e8400-e29b-41d4-a716-446655440000`
   - Must match exactly (case-insensitive)
   - Copy-paste to avoid errors

2. **Server restarted**:
   - Rooms are in-memory only
   - Server restart = all rooms deleted
   - Create new room after restart

3. **Host left room**:
   - When last member leaves, room auto-deletes
   - Create new room

### Audio/Subtitle tracks not showing

**Symptoms**: Settings panel shows no tracks or incorrect tracks

**Causes & Solutions**:

1. **Tracks not yet loaded**:
   - Wait 2-3 seconds after opening file
   - Close and reopen Settings panel

2. **Container format issue**:
   ```powershell
   # Test with VLC directly
   "C:\Program Files\VideoLAN\VLC\vlc.exe" --intf dummy --play-and-exit "video.mkv"
   # While running the GUI, open Playback → Audio Track / Subtitle Track to verify entries
   # If VLC exposes the tracks, the client should too
   ```

3. **External subtitles**:
   - Currently only embedded tracks supported
   - Merge subtitles into video file:
   ```bash
   ffmpeg -i video.mp4 -i subs.srt -c copy -c:s mov_text output.mp4
   ```

## Performance Issues

### High CPU usage

**Causes & Solutions**:

1. **Hardware decoding disabled**:
   - Open standalone VLC → `Tools → Preferences → Input / Codecs`
   - Set **Hardware-accelerated decoding** to `Automatic` (or `Direct3D11 video acceleration`), then restart the Hang client so libVLC picks up the change.
   - If your GPU struggles, set it to `Disable` and rely on CPU decoding.

2. **High resolution video**:
   - 4K videos use significantly more CPU/GPU.
   - Try 1080p or 720p versions when testing sync or on lower-spec hardware.

3. **Too many concurrent clients**:
   - Each client decodes locally; running several instances on one PC multiplies CPU/GPU demand.
   - Limit to a handful of instances per machine.

### UI lag / Stuttering

**Symptoms**: Controls respond slowly, video playback smooth

**Cause**: UI thread blocked

**Solutions**:

1. **Reduce repaint rate**:
   ```rust
   // In ui.rs, comment out:
   // ctx.request_repaint();
   
   // Or add throttle:
   ctx.request_repaint_after(Duration::from_millis(33)); // 30 FPS
   ```

2. **Disable sync temporarily**:
   - Uncheck "Enable Sync" in UI
   - Test if issue persists

### Memory leak / Growing memory usage

**Symptoms**: Memory usage increases over time

**Current Status**: Not a known issue in current implementation

**If occurs**:
1. Check for unclosed file handles
2. Monitor with Task Manager
3. Report with reproduction steps

## Network Issues

### "Connection Refused" to remote server

**Causes & Solutions**:

1. **Firewall blocking**:
   ```powershell
   # On server machine
   netsh advfirewall firewall add rule name="Hang" dir=in action=allow protocol=TCP localport=3005
   ```

2. **Server bound to localhost only**:
   - Edit `server/src/main.rs`
   - Change `"127.0.0.1:3005"` → `"0.0.0.0:3005"`
   - Restart server

3. **Router not forwarding port**:
   - Access router admin panel
   - Forward port 3005 to server machine
   - Or use port triggering

4. **ISP blocking custom ports**:
   - Try different port (e.g., 8080, 3000)
   - Update server and client code

### Connection drops randomly

**Symptoms**: "Connection lost" messages, auto-disconnect

**Causes & Solutions**:

1. **Idle timeout**:
   - Add WebSocket keepalive/ping
   - Currently not implemented (future feature)

2. **Network instability**:
   - Use wired connection instead of WiFi
   - Check router logs for disconnects

3. **Server overload**:
   - Monitor server CPU/memory
   - Limit room sizes or total clients

## Logging & Debugging

### Enable verbose logging

```powershell
# Server
$env:RUST_LOG = "hang_server=debug,tokio=info"
cd server
cargo run --release

# Client
$env:RUST_LOG = "hang_client=debug"
$env:VLC_VERBOSE = "2"   # Optional: increase libVLC logging
cd client
cargo run --release
```

### Capture network traffic

```powershell
# Install Wireshark
winget install WiresharkFoundation.Wireshark

# Filter: tcp.port == 3005
# Look for WebSocket handshake and JSON messages
```

### Test server independently

```powershell
# Use wscat (Node.js tool)
npm install -g wscat
wscat -c ws://localhost:3005

# Send test message:
{"type":"CreateRoom","payload":{"file_hash":"test123"}}
```

### Debug VLC issues

```powershell
# Run VLC with verbose logging
"C:\Program Files\VideoLAN\VLC\vlc.exe" -vvv video.mp4

# Look for codec errors, missing plugins, etc.
```

## Getting Help

### Before reporting issues

1. ✅ Check this troubleshooting guide
2. ✅ Review README.md and QUICKSTART.md
3. ✅ Enable verbose logging (RUST_LOG=debug)
4. ✅ Test with different video file
5. ✅ Try on different machine/network

### When reporting issues

Include:
- **Environment**: Windows version, Rust version (`rustc --version`)
- **Logs**: Full error messages and stack traces
- **Steps to reproduce**: Exact sequence to trigger issue
- **Video info**: Codec, resolution, container format (`ffprobe video.mp4`)
- **Network**: Local (same machine) or remote, server logs

### Useful commands for debugging

```powershell
# System info
systeminfo | findstr /C:"OS"
rustc --version
cargo --version

# VLC info
"C:\Program Files\VideoLAN\VLC\vlc.exe" --version
where.exe vlc

# Video info (requires ffmpeg)
ffprobe -v error -show_format -show_streams video.mp4

# Network info
netstat -an | findstr 3005
Test-NetConnection -ComputerName SERVER_IP -Port 3005
```

## Known Limitations (Not bugs)

1. **No reconnection**: Disconnect = must rejoin room
2. **No room persistence**: Server restart = all rooms lost
3. **No user accounts**: Anyone can create/join rooms
4. **No file transfer**: Must share video out-of-band
5. **Windows only**: Linux/macOS require minor modifications

## Common Workarounds

### Video format compatibility

**Problem**: MKV files sometimes have issues

**Workaround**: Convert to MP4
```bash
ffmpeg -i input.mkv -c copy output.mp4
# Fast remux, no re-encoding
```

### Large file hash computation slow

**Problem**: 10+ GB files take 30+ seconds to hash

**Workaround**: Use smaller clips or be patient (one-time cost)

### Room ID hard to share

**Problem**: UUID too long to type

**Workaround**: 
- Use copy-paste
- Or implement short codes (modify server)
- Or use QR codes (future feature)

---

**Still stuck?** Open an issue with full details: [GitHub Issues](#)

**Want to contribute a fix?** Pull requests welcome! See ARCHITECTURE.md for technical details.
