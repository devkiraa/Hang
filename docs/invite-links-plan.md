# Invite Links & Deep Link Handling

## Goals
- Allow hosts to share a single link that encodes room ID and optional passcode.
- When recipients click the link, the Hang desktop client should launch (or focus if already running), show room details, and streamline joining.
- Maintain compatibility with manual room entry and ensure passcode protection remains optional but secure.

## Link Formats
1. **Custom Protocol (Preferred)**
   - `hang://join?room=123-456&code=ABCD`
   - Windows installer registers the `hang` protocol to invoke `hang-client.exe --protocol-url <link>`.

2. **Web Fallback**
   - `https://letshang.onrender.com/join?room=123-456&code=ABCD`
   - Landing page detects missing client and offers download; if Hang is installed, JavaScript can redirect to the custom protocol.

## Room Model Changes
- Server stores `Room.passcode: Option<StringHash>` (bcrypt or argon2 hash).
- `CreateRoom` message accepts `passcode: Option<String>`.
- `RoomCreated` / `RoomJoined` responses echo the room ID and boolean `passcode_enabled` to the host.
- `JoinRoom` message includes `passcode: Option<String>`; server validates hash comparison.
- Add new `RoomInvite` helper endpoint (optional) that returns share-ready link text.

## Client UX Updates
- Room dialog gains a passcode field when creating a room (optional).
- After creating a room, host sees "Copy Invite Link"; clicking copies `hang://` URL with `room` + `code` (if set).
- When receiving an invite deep link:
  1. Show a modal: "Join Hang Room 123-456" + passcode prefilled (masked) if provided.
  2. Display host message: "Load the same video file: <filename hash or placeholder>".
  3. Provide primary actions: `Open Video…`, `Join Room` (enabled once hash computed), `Cancel`.

## Deep-Link Handling Architecture
- **Single Instance**: ensure one Hang client runs at a time (use a mutex + `single-instance` crate or a TCP loopback listener). Subsequent `hang://` invocations send payload to the running instance.
- **Startup Args**: parse `--protocol-url` CLI arg and enqueue invite details into a `PendingInvite` channel consumed by the Hang UI.
- **When Running**: listener receives the invite string and forwards to UI via async channel (e.g., `tokio::sync::mpsc`).
- UI listens for pending invites and triggers the modal.

## Security Considerations
- Passcodes never broadcast to other clients; only host retains them locally.
- Server compares passcode hashes using constant-time comparison to mitigate timing attacks.
- Rate-limit failed join attempts per client ID/IP.
- Optionally, expire invites when room closes.

## Implementation Phases
1. **Protocol & Server**: add passcode fields, validation, and optional `/join` HTTP endpoint that emits invite info.
2. **Client UI**: passcode support in create/join dialogs, invite link generator, and modal for pending invites.
3. **Custom Protocol**: register `hang://` via build/installer script, add CLI parsing, single-instance message passing, and UI hook.
4. **Docs & Release**: update README/Quickstart with usage instructions and environment considerations.
