# Hang

A desktop video player with synchronized room-based viewing for Windows.

## Features

### Full-Featured Video Player
- **Universal Format Support**: MP4, MKV, AVI, MOV, WebM, FLV, WMV, and more
- **Complete Playback Controls**: Play, pause, stop, seek, volume, speed adjustment (0.25x - 2.0x)
- **Multi-Track Support**: 
  - Multiple audio track selection
  - Subtitle track selection with multiple formats
- **Advanced Navigation**: Frame-by-frame stepping (forward/backward)
- **Display Modes**: Fullscreen and windowed playback
- **On-Screen Controls**: Integrated OSC (On-Screen Controller)

### Synchronized Room-Based Viewing
- **Room Management**: Create or join rooms using unique Room IDs
- **File Verification**: SHA256 hash-based file matching ensures all participants have identical videos
- **Real-Time Sync**: Millisecond-precision synchronization across all clients
- **Synchronized Actions**:
  - Play/Pause commands sync instantly
  - Seek operations propagate to all participants
  - Playback speed changes sync across the room
- **Flexible Roles**: Host/Guest system with democratic control

### Technical Highlights
- **Rust-Based**: High performance, memory safety, and low latency
- **libVLC Backend**: VLC's battle-tested playback core with wide codec coverage
- **WebSocket Protocol**: Real-time communication with efficient message passing
- **Modern UI**: Clean egui-based interface with responsive controls
- **Cross-Platform Foundation**: Built on portable technologies (currently Windows, easily extensible)

## Architecture

```
┌─────────────────────────────────────────────────────┐
│                     Hang Server                      │
│  (WebSocket Server - Room Management & Sync Relay)  │
└─────────────────────────────────────────────────────┘
                         ↕ WebSocket
┌─────────────────────────────────────────────────────┐
│                   Hang Client (Desktop)              │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────┐│
│  │  egui UI     │→ │ Sync Client  │→ │   Player   ││
│  │  (Controls)  │  │  (WebSocket) │  │  (libVLC)  ││
│  └──────────────┘  └──────────────┘  └────────────┘│
└─────────────────────────────────────────────────────┘
```

### Components

1. **Server** (`server/`)
   - WebSocket server for room management
   - Handles client connections and message routing
   - Maintains room state and member lists
   - Broadcasts sync commands to room participants

2. **Client** (`client/`)
   - Desktop application with video player
   - WebSocket client for server communication
   - Video playback via libVLC
   - Rich UI with egui framework

## Prerequisites

### Required Dependencies

1. **Rust Toolchain**
   - Install from https://rustup.rs/
   - Minimum version: 1.70+

2. **VLC media player** (libVLC runtime)
    - Download VLC: https://www.videolan.org/vlc/
    - Install the 64-bit Windows build (default path `C:\Program Files\VideoLAN\VLC`)
    - Ensure `libvlc.dll` is discoverable by either:
       - Adding the VLC install directory to `PATH`, or
       - Setting `LIBVLC_PATH` to the absolute path of `libvlc.dll`

3. **Visual Studio Build Tools** (Windows)
   - Download from https://visualstudio.microsoft.com/downloads/
   - Install "Desktop development with C++" workload
   - Required for linking native libraries

## Building

### 1. Clone or Extract Project

```powershell
cd D:\Repository\Hang\hang-sync-player
```

### 2. Build Server

```powershell
cd server
cargo build --release
```

The compiled server will be at: `target\release\hang-server.exe`

### 3. Build Client

**Important**: Ensure VLC (libVLC) is installed and `libvlc.dll` can be located before building.

```powershell
cd ..\client
cargo build --release
```

The compiled client will be at: `target\release\hang-client.exe`

### Troubleshooting Build Issues

**libvlc not found:**
```powershell
# Point to libvlc.dll directly if it's installed elsewhere
$env:LIBVLC_PATH = "C:\Program Files\VideoLAN\VLC\libvlc.dll"
cargo build --release
```

**Linking errors:**
- Ensure Visual Studio Build Tools are installed
- Verify PATH includes the VLC install directory
- Try running from "x64 Native Tools Command Prompt"

## Running

### Start the Server

```powershell
cd server
cargo run --release
```

Or run the compiled executable:
```powershell
.\target\release\hang-server.exe
```

Server will listen on `ws://0.0.0.0:3005` by default.

### Start the Client

```powershell
cd client
cargo run --release
```

Or run the compiled executable:
```powershell
.\target\release\hang-client.exe
```

## Usage Guide

### Solo Viewing

1. Launch the client
2. Click "Open Video" to select a video file
3. Use playback controls to watch your video

### Synchronized Room Viewing

#### Creating a Room (Host)

1. Launch the client
2. Open a video file
3. Ensure "Server URL" points to your server (default: `ws://localhost:3005`)
4. Click "Create Room"
5. Share the generated Room ID with participants

#### Joining a Room (Guest)

1. Launch the client
2. Open the **exact same video file** as the host
3. Enter the Room ID provided by the host
4. Click "Join Room"
5. Playback will automatically sync with other participants

### Controls

**Playback:**
- `▶/⏸`: Play/Pause
- `⏹`: Stop
- `⏮/⏭`: Frame step backward/forward
- Timeline slider: Seek to specific position

**Volume & Speed:**
- Volume slider (🔊): Adjust audio level (0-100)
- Speed slider: Change playback speed (0.25x - 2.0x)

**Display:**
- `⛶`: Toggle fullscreen
- `⚙ Settings`: Open settings panel for audio/subtitle tracks

**Room Controls:**
- "Create Room": Start a new synchronized session
- "Join Room": Enter an existing room
- "Leave Room": Exit current room
- "Enable Sync" checkbox: Toggle automatic synchronization

### Settings Panel

Access via `⚙ Settings` button:

- **Audio Tracks**: Select between available audio streams
- **Subtitle Tracks**: Enable/disable and select subtitle tracks

## Protocol Details

### Message Types

**Client → Server:**
- `CreateRoom`: Request new room creation
- `JoinRoom`: Join existing room with file hash verification
- `LeaveRoom`: Exit current room
- `SyncCommand`: Send playback command to room

**Server → Client:**
- `RoomCreated`: Confirmation with Room ID
- `RoomJoined`: Success with role (host/guest)
- `SyncBroadcast`: Relayed command from another client
- `FileHashMismatch`: File verification failed
- `RoomNotFound`: Invalid Room ID
- `Error`: General error message

### Sync Commands

All sync commands include timestamp for precise synchronization:

- `Play { timestamp }`: Resume playback at position
- `Pause { timestamp }`: Pause at position
- `Seek { timestamp }`: Jump to position
- `Speed { rate }`: Change playback speed
- `Stop`: Stop playback

### File Verification

Files are verified using SHA256 hash:
- Hash computed on file selection
- Sent with room join request
- Server validates hash matches room's expected hash
- Prevents sync issues from different video files

## Configuration

### Server Configuration

Edit environment variables or modify `server/src/main.rs`:

```rust
let addr = "0.0.0.0:3005"; // Change bind address/port
```

### Client Configuration

Default server URL can be changed in UI or modify `client/src/ui.rs`:

```rust
server_url: "ws://localhost:3005".to_string(), // Default server
```

## Development

### Project Structure

```
hang-sync-player/
├── Cargo.toml          # Workspace configuration
├── server/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs     # Server entry point
│       ├── protocol.rs # Shared message types
│       └── state.rs    # Room & client state management
└── client/
    ├── Cargo.toml
    └── src/
        ├── main.rs     # Client entry point
      ├── player.rs   # libVLC loader
        ├── sync.rs     # WebSocket client
        ├── ui.rs       # egui interface
        ├── protocol.rs # Message types
        └── utils.rs    # Helpers (hashing, formatting)
```

### Running in Development

Terminal 1 (Server):
```powershell
cd server
cargo run
```

Terminal 2 (Client 1):
```powershell
cd client
cargo run
```

Terminal 3 (Client 2 - for testing):
```powershell
cd client
cargo run
```

### Logging

Set `RUST_LOG` environment variable for detailed logs:

```powershell
$env:RUST_LOG = "debug"
cargo run
```

Levels: `error`, `warn`, `info`, `debug`, `trace`

## Performance Tips

1. **Local Playback**: Use local files, avoid network drives
2. **Codec Support**: Use widely supported codecs (H.264, AAC)
3. **Network**: Stable connection required for smooth sync
4. **Hardware**: Hardware acceleration handled automatically by libVLC

## Troubleshooting

### "File Hash Mismatch" Error
- Ensure all participants have **identical** video files
- Same file name doesn't guarantee same content
- Re-download or copy files to match exactly

### Sync Lag/Desync
- Check network latency between clients and server
- Reduce playback speed if persistent issues
- Disable "Enable Sync" temporarily to test local playback

### Video Won't Load
- Verify VLC is installed and accessible
- Check file path has no special characters
- Try different video file/format
- Check VLC can play the file standalone

### Server Connection Failed
- Verify server is running
- Check firewall allows port 3005
- For remote servers, use correct IP address
- Test with `ws://127.0.0.1:3005` for local

## Known Limitations

- Windows primary target (Linux/macOS possible with minor changes)
- VLC dependency required (not bundled)
- No built-in chat/communication features
- Room limit not enforced (dependent on server resources)

## Future Enhancements

- [ ] Chat system integrated into UI
- [ ] Room passwords/authentication
- [ ] Persistent room state (reconnection support)
- [ ] Playlist management with queue sync
- [ ] Video filters and effects sync
- [ ] Mobile client support
- [ ] Self-contained releases with bundled VLC

## License

MIT License - See LICENSE file for details

## Contributing

Contributions welcome! Please:
1. Fork the repository
2. Create feature branch
3. Test thoroughly
4. Submit pull request with clear description

## Support

For issues, questions, or feature requests, please open an issue on the project repository.

---

**Built with ❤️ using Rust, libVLC, egui, and tokio**
