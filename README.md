# NetShare

> Share your mouse, keyboard, audio, and files across PCs on your LAN — no KVM switch required.

NetShare is a Rust application that lets you control multiple PCs from a single keyboard and mouse, while also sharing audio, files, and clipboard content over your local network.

---

## Features

| Feature | Details |
|---------|---------|
| **Mouse & Keyboard** | Control remote PCs using the server's input devices |
| **Audio** | Stream desktop audio and microphone between machines |
| **File Transfer** | Send files and folders with integrity verification and resume support |
| **Clipboard Sync** | Automatically sync text and images between machines |
| **Multi-client** | Connect up to 9 client PCs simultaneously |
| **TLS Encryption** | All TCP connections are TLS-encrypted |
| **Pairing Code** | 6-character code prevents unauthorized connections |

---

## Architecture

```
Server (main PC)          Client (secondary PC)
─────────────────         ─────────────────────
Input Capture ──TCP:9000──▶ Input Inject
Audio Sink    ◀─UDP:9001── Audio Capture
Mic Capture   ──UDP:9002──▶ Virtual Mic
File Receiver ◀─TCP:9003── File Sender
File Sender   ──TCP:9003──▶ File Receiver
Clipboard     ◀─TCP:9004──▶ Clipboard
```

- **Server** — the machine whose keyboard and mouse you want to share
- **Client** — a secondary machine that receives input and shares audio

All connections are **direct LAN** (no cloud, no relay).

---

## Requirements

### Windows
- Windows 10/11
- [VB-Cable](https://vb-audio.com/Cable/) (for audio sharing — free)

### Linux
- Ubuntu 24.04+ or any distro with PipeWire/PulseAudio
- `evdev` access (usually requires being in the `input` group)

### Build Requirements
- Rust 1.75+ (`rustup install stable`)
- CMake 3.10+ (for the Opus audio codec)
- On Windows: MSVC Build Tools

---

## Quick Start

### 1. Build

```bash
git clone https://github.com/MCTEEKUNG/Move.git
cd Move/netshare
cargo build --release
```

Binaries are in `target/release/`:
- `netshare-gui` — graphical interface (recommended)
- `netshare-server` — headless server
- `netshare-client` — headless client

### 2. GUI Usage

Run `netshare-gui` on **both machines**.

**On the server PC:**
1. Click **Start Server**
2. Note the **pairing code** (e.g. `1A2B3C`) shown in the GUI

**On the client PC:**
1. Enter the server's IP address and port (e.g. `192.168.1.10:9000`)
2. Enter the pairing code shown on the server
3. Click **Connect**

Once connected, move your mouse off the edge of the server screen or press `Ctrl+Shift+Alt+1` to activate the first client slot.

### 3. CLI Usage

**Server:**
```bash
netshare-server                    # bind 0.0.0.0:9000
netshare-server 0.0.0.0:9000       # explicit address
```

**Client:**
```bash
netshare-client 192.168.1.10:9000 <pairing-code>
```

---

## Hotkeys (Server)

| Hotkey | Action |
|--------|--------|
| `Ctrl+Shift+Alt+1` … `+9` | Switch active client to slot 1–9 |
| `Scroll Lock` | Cycle to the next connected client |

---

## File Transfer

Drop files onto the GUI window to send them to the active client (server→client) or to the server (client→server).

- The receiving side shows an **Accept / Reject** dialog before writing anything to disk
- Received files are saved to `~/Downloads/NetShare/`
- Transfers resume automatically if the connection drops mid-way
- Every file is verified with **CRC32** (per chunk) and **SHA-256** (whole file)

---

## Security

| Mechanism | Details |
|-----------|---------|
| **TLS** | All TCP channels (:9000, :9003, :9004) use TLS with a self-signed certificate |
| **Pairing code** | Derived from the server's TLS certificate fingerprint; must match to connect |
| **Path sanitization** | Received files are sandboxed to the receive folder — no path traversal |
| **File accept prompt** | User must explicitly accept every incoming file transfer |

> **Note:** The TLS certificate is self-signed. Security relies on the pairing code being verified out-of-band (user confirms the 6-character code displayed on both machines).

---

## Configuration

| Setting | Default | Notes |
|---------|---------|-------|
| Control port | `9000` | TCP |
| Client→Server audio | `9001` | UDP |
| Server→Client mic | `9002` | UDP |
| File transfer | `9003` | TCP |
| Clipboard sync | `9004` | TCP |
| Receive folder | `~/Downloads/NetShare` | Configurable |
| Chunk size | 256 KB | Fixed |
| Audio codec | Opus 128 kbps, 48 kHz stereo | Fixed |

---

## Project Structure

```
netshare/
├── crates/
│   ├── netshare-core/     # Shared types, protocol, framing, TLS utilities
│   ├── netshare-server/   # Server binary + library
│   ├── netshare-client/   # Client binary + library
│   └── netshare-gui/      # egui GUI (depends on server + client libs)
└── vendor/
    └── audiopus_sys/      # Patched for CMake 3.31+ compatibility
```

---

## Roadmap

- [x] Phase 1 — Mouse & keyboard sharing (Windows LL Hooks + SendInput)
- [x] Phase 2 — Audio sharing (CPAL + Opus, VB-Cable on Windows)
- [x] Phase 3 — File transfer & clipboard sync
- [x] Phase 4 — GUI (egui + tray icon + mDNS auto-discovery)
- [x] Phase 5 — TLS encryption + pairing code + file accept prompt
- [ ] Phase 6 — Installer (`.msi` for Windows, `.deb` / `.AppImage` for Linux)

---

## License

MIT
