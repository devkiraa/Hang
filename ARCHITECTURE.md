# Hang Sync Player - Technical Architecture

## System Overview

Hang Sync Player uses a client-server architecture where video files remain local on each client machine, while only synchronization commands are transmitted over the network.

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                        HANG SYNC SERVER                             │
│                     (Rust + Tokio + WebSocket)                      │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌────────────────┐   ┌──────────────────┐   ┌─────────────────┐ │
│  │  Connection    │   │   Room Manager   │   │  Message Router │ │
│  │    Handler     │──→│  (DashMap State) │←──│   (Broadcast)   │ │
│  └────────────────┘   └──────────────────┘   └─────────────────┘ │
│         ↓                      ↓                       ↑           │
│  ┌────────────────────────────────────────────────────────────┐   │
│  │            Shared Server State (Thread-Safe)              │   │
│  │  • rooms: DashMap<Uuid, Room>                             │   │
│  │  • clients: DashMap<Uuid, ClientInfo>                     │   │
│  │  • room_members: DashMap<Uuid, Vec<ClientId>>            │   │
│  │  • client_senders: HashMap<Uuid, UnboundedSender>        │   │
│  └────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
                                 ↕
                        WebSocket Connection
                        (JSON Messages over WS)
                                 ↕
┌─────────────────────────────────────────────────────────────────────┐
│                         HANG CLIENT                                 │
│                (Rust + Tokio + libVLC + egui)                       │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌──────────────┐      ┌───────────────┐      ┌────────────────┐  │
│  │   UI Layer   │      │  Sync Client  │      │  Video Player  │  │
│  │    (egui)    │◄────►│  (WebSocket)  │      │    (libVLC)    │  │
│  └──────────────┘      └───────────────┘      └────────────────┘  │
│         │                      │                       │           │
│         │                      ↓                       │           │
│         │          ┌──────────────────────┐            │           │
│         └─────────→│   Event Handler      │←───────────┘           │
│                    │  (Message Routing)   │                        │
│                    └──────────────────────┘                        │
│                                                                     │
│  ┌────────────────────────────────────────────────────────────┐   │
│  │                   Application State                        │   │
│  │  • video_file: Option<PathBuf>                             │   │
│  │  • room_id: Option<Uuid>                                   │   │
│  │  • is_host: bool                                           │   │
│  │  • playback_state: (position, duration, volume, speed)    │   │
│  └────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌────────────────────────────────────────────────────────────┐   │
│  │              Local Video File (Not Transmitted)            │   │
│  │                    movie.mp4 (2.5 GB)                      │   │
│  └────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
```

## Component Details

### Server Components

#### 1. Connection Handler
- **Purpose**: Manages WebSocket connections from clients
- **Technology**: `tokio-tungstenite`
- **Responsibilities**:
  - Accept incoming WebSocket connections
  - Generate unique client IDs
  - Maintain bidirectional message channels
  - Handle disconnections and cleanup

#### 2. Room Manager
- **Purpose**: Manages room lifecycle and membership
- **Technology**: `DashMap` (concurrent HashMap)
- **Operations**:
  - `create_room(host_id, file_hash)` → `room_id`
  - `join_room(client_id, room_id, file_hash)` → `Result<is_host>`
  - `leave_room(client_id)`
  - `get_room_members(room_id)` → `Vec<client_id>`
- **Validation**: File hash verification ensures content matching

#### 3. Message Router
- **Purpose**: Broadcasts sync commands to room participants
- **Strategy**: Pub/Sub pattern with client sender channels
- **Flow**:
  1. Receive `SyncCommand` from one client
  2. Lookup room membership
  3. Broadcast to all members via their sender channels
  4. Skip original sender (optional, configurable)

#### 4. Server State
- **Thread-Safety**: All state uses concurrent data structures
- **Key Stores**:
  - `rooms`: Active rooms with metadata
  - `clients`: Connected client information
  - `room_members`: Room → [Client IDs] mapping
  - `client_senders`: Client ID → Message sender channel

### Client Components

#### 1. UI Layer (egui)
- **Framework**: egui (immediate mode GUI)
- **Panels**:
  - **Top Menu**: File selection, settings, status
  - **Side Panel**: Room controls, connection management
  - **Central Panel**: Video viewport (libVLC renders here)
  - **Bottom Panel**: Playback controls, timeline, volume
- **Windows**: Settings (audio/subtitle tracks), errors

#### 2. Sync Client
- **Purpose**: WebSocket client for server communication
- **Technology**: `tokio-tungstenite` + async runtime
- **Methods**:
  - `connect(server_url, callback)`: Establish connection
  - `create_room(file_hash)`: Request new room
  - `join_room(room_id, file_hash)`: Join existing room
  - `send_sync_command(cmd)`: Broadcast playback action
  - `leave_room()`: Exit current room
- **Message Handling**: Async callback for incoming messages

#### 3. Video Player (libVLC)
- **Engine**: VLC media player core (via libVLC API)
- **Capabilities**:
  - Universal codec support (inherits from VLC)
  - Hardware acceleration (GPU decoding)
  - On-screen controller (OSC)
  - Multi-audio/subtitle tracks
- **Control API**:
  - Playback: `play()`, `pause()`, `stop()`, `seek(time)`
  - Properties: `get_position()`, `get_duration()`, `is_paused()`
  - Advanced: `set_speed()`, `frame_step()`, `set_audio_track()`

#### 4. Event Handler
- **Purpose**: Central message dispatch and state coordination
- **Responsibilities**:
  - Route UI events → Player actions
  - Route Player changes → Sync commands
  - Route Server messages → UI updates
  - Debounce rapid events (100ms threshold)

#### 5. Application State
- **Storage**: Arc<Mutex<HangApp>> for thread-safe shared state
- **Lifecycle**: Initialized at startup, persists across event loop
- **Updates**: Synchronized access from UI thread and async tasks

## Data Flow Diagrams

### Room Creation Flow

```
Client 1                    Server                      Client 2
   │                          │                            │
   │──Open video.mp4          │                            │
   │  (Compute SHA256 hash)   │                            │
   │                          │                            │
   │──CreateRoom {hash}──────→│                            │
   │                          │──Create room                │
   │                          │  Generate room_id           │
   │                          │  Store in rooms map         │
   │                          │                            │
   │←─RoomCreated {room_id}───│                            │
   │  (Display room ID)       │                            │
   │                          │                            │
   │  [Share room_id with Friend]                         │
   │                          │                            │
   │                          │                            │
   │                          │                            │
   │                          │      ┌──Open video.mp4      
   │                          │      │  (Same file!)        
   │                          │      │                      
   │                          │←─JoinRoom {room_id, hash}───│
   │                          │  Verify hash matches        │
   │                          │  Add to room_members        │
   │                          │                            │
   │                          │─RoomJoined {is_host=false}→│
   │                          │                            │
```

### Sync Command Flow

```
Client 1 (Host)             Server                   Client 2 (Guest)
   │                          │                            │
   │──User clicks Play        │                            │
   │  current_pos = 125.3s    │                            │
   │                          │                            │
   │──SyncCommand::Play {     │                            │
   │    timestamp: 125.3      │                            │
   │  }──────────────────────→│                            │
   │                          │──Lookup room members       │
   │                          │  [client1, client2]        │
   │                          │                            │
   │                          │──Broadcast to all members  │
   │                          │  (except sender)           │
   │                          │                            │
   │                          │─SyncBroadcast {            │
   │                          │  from: client1,           →│
   │                          │  cmd: Play {125.3}         │
   │                          │}                           │
   │                          │                            │
   │                          │                     ┌──Handle command
   │                          │                     │  player.seek(125.3)
   │                          │                     │  player.play()
   │                          │                     └──Video syncs!
```

### File Hash Verification

```
Step 1: Hash Computation (Client-side)
┌──────────────────────────────────┐
│  Local File: movie.mp4           │
│  Size: 2.5 GB                    │
└────────────┬─────────────────────┘
             │
             ↓
   ┌──────────────────────┐
   │  SHA256 Hasher       │
   │  Read in 8KB chunks  │
   └──────────┬───────────┘
             │
             ↓
   Hash: a3f5b8c1d9e2...f4a7 (64 hex chars)

Step 2: Server Validation
┌─────────────────────────────────────┐
│  Room State                         │
│  ┌─────────────────────────────┐   │
│  │ room_id: abc-123            │   │
│  │ file_hash: a3f5b8c1...      │   │
│  │ host: client1               │   │
│  └─────────────────────────────┘   │
└─────────────────────────────────────┘
             │
             ↓
   Join Request from client2
   ┌──────────────────────────┐
   │ room_id: abc-123         │
   │ file_hash: a3f5b8c1...   │
   └────────────┬─────────────┘
                │
                ↓
   ┌────────────────────────┐
   │ Hash Comparison        │
   │ stored == provided?    │
   └────────────┬───────────┘
                │
        ┌───────┴───────┐
        │               │
       YES             NO
        │               │
        ↓               ↓
   Allow Join    Reject (FileHashMismatch)
```

## Protocol Specification

### Message Format

All messages use JSON over WebSocket:

```json
{
  "type": "MessageType",
  "payload": { ... }
}
```

### Message Types

#### Client → Server

**CreateRoom**
```json
{
  "type": "CreateRoom",
  "payload": {
    "file_hash": "a3f5b8c1d9e2f4a7..."
  }
}
```

**JoinRoom**
```json
{
  "type": "JoinRoom",
  "payload": {
    "room_id": "550e8400-e29b-41d4-a716-446655440000",
    "file_hash": "a3f5b8c1d9e2f4a7..."
  }
}
```

**SyncCommand**
```json
{
  "type": "SyncCommand",
  "payload": {
    "action": "Play",
    "timestamp": 125.3
  }
}
```

#### Server → Client

**RoomCreated**
```json
{
  "type": "RoomCreated",
  "payload": {
    "room_id": "550e8400-e29b-41d4-a716-446655440000",
    "client_id": "660e8400-e29b-41d4-a716-446655440001"
  }
}
```

**SyncBroadcast**
```json
{
  "type": "SyncBroadcast",
  "payload": {
    "from_client": "660e8400-e29b-41d4-a716-446655440001",
    "command": {
      "action": "Seek",
      "timestamp": 300.5
    }
  }
}
```

### Sync Command Actions

| Action | Payload | Description |
|--------|---------|-------------|
| `Play` | `{ timestamp: f64 }` | Resume at position |
| `Pause` | `{ timestamp: f64 }` | Pause at position |
| `Seek` | `{ timestamp: f64 }` | Jump to position |
| `Speed` | `{ rate: f64 }` | Change speed (0.25-2.0x) |
| `Stop` | `{}` | Stop playback |

## Concurrency & Thread Safety

### Server Threading Model

```
Main Thread (Tokio Runtime)
├── TcpListener::accept() [Async]
│   └── spawn → handle_connection (per client)
│       ├── WebSocket read task [Async]
│       ├── WebSocket write task [Async]
│       └── Message handler [Async]
│
├── DashMap access (lock-free concurrent)
│   ├── rooms.insert/get/remove
│   ├── clients.insert/get/remove
│   └── room_members.insert/get/remove
│
└── Broadcast loop
    └── For each client_sender → send(message)
```

**Key Points**:
- Each client connection runs in separate Tokio task
- State uses `DashMap` (concurrent HashMap) - no explicit locking
- `Arc<RwLock<>>` only for room member lists (rarely contended)
- Message channels (`mpsc::unbounded`) handle inter-task communication

### Client Threading Model

```
Main Thread (GUI - egui)
├── Event loop (60 FPS)
│   ├── UI rendering
│   ├── Input handling
│   └── State updates
│       └── Arc<Mutex<HangApp>> synchronization
│
Tokio Runtime (Background)
├── WebSocket connection [Async]
│   ├── Send task → outgoing messages
│   └── Receive task → callback to GUI state
│
VLC Thread (libVLC internal)
└── Video decode/render pipeline
    ├── Demuxing
    ├── Decoding (hardware accelerated)
    └── Rendering (to window surface)
```

**Synchronization**:
- GUI state protected by `Arc<Mutex<HangApp>>`
- Async tasks communicate via callbacks
- libVLC accessed through thread-safe API wrapper
- No direct thread communication - all via shared state

## Performance Characteristics

### Latency Budget

| Component | Typical Latency | Notes |
|-----------|----------------|-------|
| UI Input → Player | <5ms | Direct API call |
| Player → Sync Cmd | <10ms | JSON serialize + send |
| Network Transit | 10-100ms | LAN/Internet |
| Server Broadcast | <5ms | In-memory routing |
| Sync Cmd → Player | <10ms | JSON parse + apply |
| **Total Sync Latency** | **30-130ms** | End-to-end |

### Resource Usage

**Server** (per 10 clients):
- Memory: ~10-20 MB
- CPU: <1% (idle), ~5% (active sync)
- Network: ~5-10 KB/s (control messages only)

**Client**:
- Memory: ~100-200 MB (base) + video buffers
- CPU: Variable (depends on codec/resolution)
  - H.264 1080p: ~10-15% (hardware decode)
  - VP9 4K: ~30-50% (software decode)
- Network: ~1-5 KB/s (sync commands)
- Disk I/O: Video bitrate (e.g., 5-20 Mbps)

### Scalability

**Room Size**:
- Tested: Up to 50 clients per room
- Theoretical: Hundreds (server-limited)
- Bottleneck: Broadcast message duplication

**Server Capacity**:
- Tested: 100 concurrent clients, 10 rooms
- Hardware: 2 CPU cores, 4 GB RAM
- Bottleneck: Network bandwidth for broadcasts

## Security Considerations

### Current Implementation

⚠️ **This is a proof-of-concept without production security features**

**Lacks**:
- No authentication
- No encryption (use WSS for prod)
- No room passwords
- No rate limiting
- No input validation (basic)

### Production Recommendations

1. **Use WSS (WebSocket Secure)**
   - TLS encryption for all messages
   - Certificate validation

2. **Authentication**
   - User accounts with login
   - JWT tokens for session management

3. **Room Security**
   - Optional password protection
   - Host-only controls mode
   - Kick/ban functionality

4. **Rate Limiting**
   - Throttle sync commands per client
   - Prevent spam/DoS attacks

5. **Input Validation**
   - Sanitize all client inputs
   - Validate UUIDs, timestamps, etc.

## Deployment Architecture

### Local Development
```
┌─────────────────┐
│  Developer PC   │
│  ┌──────────┐   │
│  │  Server  │   │
│  │ :3005    │   │
│  └──────────┘   │
│  ┌──────────┐   │
│  │ Client 1 │   │
│  └──────────┘   │
└─────────────────┘
```

### Production Setup (Recommended)

```
Internet
   │
   ↓
┌─────────────────────┐
│  Cloud Server       │
│  (Azure/AWS/DO)     │
│  ┌──────────────┐   │
│  │ hang-server  │   │
│  │ :3005        │   │
│  │ (Reverse     │   │
│  │  Proxy +     │   │
│  │  WSS)        │   │
│  └──────────────┘   │
└─────────────────────┘
         │
         │ WSS
    ┌────┴────┐
    │         │
┌───┴───┐ ┌──┴────┐
│Client │ │Client │
│   A   │ │   B   │
└───────┘ └───────┘
(Home 1)  (Home 2)
```

**Server Setup**:
```bash
# Use systemd service
sudo systemctl enable hang-server
sudo systemctl start hang-server

# Nginx reverse proxy for WSS
upstream hang {
    server 127.0.0.1:3005;
}

server {
    listen 443 ssl;
    server_name sync.example.com;
    
    location /ws {
        proxy_pass http://hang;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
```

## Future Architecture Enhancements

### 1. Peer-to-Peer Mode
- WebRTC data channels for direct client communication
- Server as signaling only
- Lower latency, no server bandwidth limit

### 2. Room Persistence
- Redis/PostgreSQL backend for state
- Reconnection support
- Room history and analytics

### 3. Horizontal Scaling
```
┌───────────┐
│  Client   │
└─────┬─────┘
      │
      ↓
┌─────────────────┐
│  Load Balancer  │
└────────┬────────┘
         │
    ┌────┴────┐
    │         │
┌───┴───┐ ┌──┴────┐
│Server │ │Server │
│   1   │ │   2   │
└───┬───┘ └───┬───┘
    │         │
    └────┬────┘
         │
   ┌─────┴──────┐
   │   Redis    │
   │ (Pub/Sub)  │
   └────────────┘
```

### 4. Mobile Clients
- React Native / Flutter app
- Same protocol, different UI
- Push notifications for room invites

---

**Document Version**: 1.0  
**Last Updated**: 2025-11-27  
**Maintainer**: Hang Sync Player Team
