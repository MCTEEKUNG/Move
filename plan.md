# Project Plan: NetShare — Network Device Sharing Application

> แอพพลิเคชันสำหรับแชร์ Mouse, Keyboard, Audio Output และ Microphone ข้ามพีซีผ่านเครือข่าย  
> รองรับ: **Windows 11** และ **Ubuntu Linux 24.04+**

---

## 1. ภาพรวมของโปรเจค (Project Overview)

NetShare คือแอพพลิเคชันแบบ Server-Client ที่ช่วยให้ผู้ใช้สามารถใช้งานอุปกรณ์อินพุต/เอาต์พุตจากเครื่องหลัก (Server) บนเครื่องรอง (Client) ผ่านเครือข่าย LAN โดยไม่ต้องใช้ KVM Switch ทางกายภาพ

### อุปกรณ์ที่รองรับ
| อุปกรณ์ | ทิศทาง | หมายเหตุ |
|---------|--------|---------|
| Mouse | Server → Client | ควบคุม cursor และ click บน Client |
| Keyboard | Server → Client | พิมพ์และ shortcut บน Client |
| Audio Output (หูฟัง/ลำโพง) | Server ↔ Client | ส่งเสียงออกจาก Client มาเล่นที่ Server หรือกลับกัน |
| Microphone | Server ↔ Client | ส่งสัญญาณ mic ข้ามเครื่อง |
| File Sharing | Server ↔ Client | ส่ง/รับไฟล์และโฟลเดอร์ระหว่างเครื่อง |
| Clipboard Sync | Server ↔ Client | sync ข้อความและรูปภาพที่ Copy/Paste ข้ามเครื่อง (port แยก ป้องกัน HOL blocking) |

---

## 2. สถาปัตยกรรมระบบ (System Architecture)

```
┌─────────────────────────────────┐         ┌─────────────────────────────────┐
│         SERVER (Primary PC)      │         │         CLIENT (Secondary PC)    │
│                                 │         │                                 │
│  ┌──────────────┐               │  TCP    │  ┌──────────────┐               │
│  │ Input Capture│──────────────────────────▶│ Input Inject  │               │
│  │ (Mouse/KB)   │               │  :9000  │  │ (uinput/API) │               │
│  └──────────────┘               │         │  └──────────────┘               │
│                                 │         │                                 │
│  ┌──────────────┐               │  UDP    │  ┌──────────────┐               │
│  │ Audio Sink   │◀─────────────────────────│  Audio Capture│               │
│  │ (Playback)   │               │  :9001  │  │ (Virtual Dev)│               │
│  └──────────────┘               │         │  └──────────────┘               │
│                                 │         │                                 │
│  ┌──────────────┐               │  UDP    │  ┌──────────────┐               │
│  │ Mic Capture  │──────────────────────────▶│ Virtual Mic   │               │
│  └──────────────┘               │  :9002  │  └──────────────┘               │
│                                 │         │                                 │
│  ┌──────────────┐               │  TCP    │  ┌──────────────┐               │
│  │ File Manager │◀────────────────────────▶│ File Manager  │               │
│  │ (Send/Recv)  │               │  :9003  │  │ (Send/Recv)  │               │
│  └──────────────┘               │         │  └──────────────┘               │
│                                 │         │                                 │
│  ┌──────────────┐               │  TCP    │  ┌──────────────┐               │
│  │  Clipboard   │◀────────────────────────▶│  Clipboard    │               │
│  │  Sync        │               │  :9004  │  │  Sync        │               │
│  └──────────────┘               │         │  └──────────────┘               │
│                                 │         │                                 │
│  ┌──────────────┐               │         │  ┌──────────────┐               │
│  │  GUI / Tray  │               │         │  ┌──────────────┐               │
│  │   App        │               │         │  │  GUI / Tray  │               │
│  └──────────────┘               │         │  └──────────────┘               │
└─────────────────────────────────┘         └─────────────────────────────────┘
```

### โหมดการทำงาน
- **Server Mode** — เครื่องที่มีอุปกรณ์จริงต่ออยู่ (เช่น เมาส์, คีย์บอร์ด)
- **Client Mode** — เครื่องปลายทางที่รับ input และส่ง/รับ audio
- รองรับ 1 Server : N Clients (Multi-Client)

### Active Client & Focus Switching
Server จะมีแนวคิด **"Active Client"** — input ทั้งหมดจะส่งไปยัง Client ที่ active เพียงเครื่องเดียวในแต่ละขณะ

| โหมด | พฤติกรรม | วิธีเปิดใช้ |
|------|---------|-----------|
| **Exclusive (default)** | input ไปยัง active client เท่านั้น | ค่าเริ่มต้น |
| **Broadcast (opt-in)** | input ส่งไปทุก client พร้อมกัน | เปิดใน Advanced Settings |

**Hotkey switching:** `Ctrl+Shift+Alt+[1-9]` เพื่อ switch ไปยัง Client ที่ต้องการ หรือ `Scroll Lock` เพื่อ cycle ไปเรื่อย ๆ Server จะแสดง indicator บน Tray/GUI ว่า active client คือเครื่องไหน Broadcast mode จะมี visual warning สีแดงเพื่อป้องกัน confusion

---

## 3. เทคโนโลยีที่เลือกใช้ (Tech Stack)

| ส่วน | เทคโนโลยี | เหตุผล |
|------|-----------|--------|
| ภาษาหลัก | **Rust** | Performance สูง, Memory safe, Cross-platform |
| GUI | **egui** + **eframe** | Pure Rust, immediate-mode, binary เล็ก, ควบคุมได้ทุกอย่างใน Rust |
| Networking | **tokio** (async runtime) | Async I/O ประสิทธิภาพสูง |
| Protocol | TCP (Control) + UDP (Audio) | Reliable input, Low-latency audio |
| Audio | **CPAL** (Cross-Platform Audio Library) | รองรับทั้ง Windows WASAPI และ Linux PipeWire/PulseAudio |
| Input Capture (Windows) | **LL Hooks** (WH_MOUSE_LL / WH_KEYBOARD_LL) | Global capture ก่อน OS ส่งให้ app ใด ๆ, suppress event บน Server |
| Input Inject (Windows) | **SendInput** | Inject mouse/keyboard event บน Client |
| Input Capture (Linux) | **evdev** (exclusive grab) | Grab device ระดับ kernel, ไม่มี process อื่นเห็น event |
| Input Inject (Linux) | **uinput** | สร้าง virtual device และ inject event |
| Audio Codec | **Opus** | Low latency, High quality, Open source |
| Encryption (Optional) | **TLS / DTLS** | ความปลอดภัยในเครือข่าย |

---

## 4. โครงสร้างโปรเจค (Project Structure)

```
netshare/
├── Cargo.toml                  # Rust workspace manifest
├── plan.md                     # ไฟล์นี้
│
├── crates/
│   ├── netshare-core/          # Logic กลาง, Protocol, Types
│   │   ├── src/
│   │   │   ├── protocol.rs     # Packet format, serialization
│   │   │   ├── input.rs        # Input event types
│   │   │   └── audio.rs        # Audio frame types
│   │   └── Cargo.toml
│   │
│   ├── netshare-server/        # Server binary
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── input_capture/  # แยก platform
│   │   │   │   ├── windows.rs
│   │   │   │   └── linux.rs
│   │   │   ├── audio_capture.rs
│   │   │   └── network.rs
│   │   └── Cargo.toml
│   │
│   ├── netshare-client/        # Client binary
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── input_inject/   # แยก platform
│   │   │   │   ├── windows.rs
│   │   │   │   └── linux.rs
│   │   │   ├── audio_sink.rs
│   │   │   └── network.rs
│   │   └── Cargo.toml
│   │
│   └── netshare-gui/           # GUI App — Monolithic: depend on netshare-server + netshare-client crates โดยตรง (1 binary, mode เลือกได้จาก UI)
│       ├── src/
│       │   ├── main.rs         # เลือก Server/Client mode จาก args หรือ GUI startup screen
│       │   ├── app.rs          # egui app loop
│       │   └── tray.rs         # tray-icon integration
│       └── Cargo.toml
│
└── docs/
    └── protocol.md             # เอกสาร Protocol specification
```

---

## 5. Network Protocol

### Control Channel (TCP :9000)
ใช้ **bincode v2** สำหรับ serialization — compact binary format, zero-copy friendly, pure Rust

```
Packet Header (8 bytes):
┌─────────┬─────────┬──────────┬──────────────┐
│ Type(1) │ Seq (2) │ Flags(1) │ Length (4)   │  ... Payload
└─────────┴─────────┴──────────┴──────────────┘

Flags byte (bitmask):
  0x01 = Compressed
  0x02 = Broadcast (sent to all clients)
  *(TLS ทำงานที่ transport layer ครอบทั้ง TCP stream อยู่แล้ว — ไม่มี per-packet flag)*

Packet Types:
  0x01 = Hello          (Client → Server)
  0xF1 = Hello Response (Server → Client)
  0x02 = Mouse Move Event
  0x03 = Mouse Click Event
  0x04 = Keyboard Event
  0x05 = Scroll Event
  0x10 = Audio Config
  0x11 = Heartbeat
  0x12 = Active Client Changed   (server broadcast → ทุก client: active เปลี่ยนเป็น slot ไหน)
  0xFF = Disconnect

Hello Payload (Client → Server):
┌────────────────┬─────────────────────┐
│ Version: u16   │ ClientName: string  │
└────────────────┴─────────────────────┘

0xF1 Hello Response (Server → Client):
┌───────────────┬─────────────────────┐
│ Slot: u8      │ ServerName: string  │   ← Slot 1-9 สำหรับ Ctrl+Shift+Alt+[slot]
└───────────────┴─────────────────────┘

Active Client Changed Payload (Server → All Clients):
┌──────────────────┐
│ ActiveSlot: u8   │   ← 0 = ไม่มี active client (Server mode), 1-9 = slot ที่ active
└──────────────────┘
```

### File Transfer Channel (TCP :9003)
ใช้ **chunked transfer** เพื่อรองรับไฟล์ขนาดใหญ่ พร้อม integrity check

```
File Transfer Packet Types:
  0x20 = File List Request / Response    (ขอดูรายการไฟล์)
  0x21 = File Send Request               (ขอส่งไฟล์)
  0x22 = File Send Accept / Reject       (ปลายทางตอบรับ)
  0x23 = File Chunk                      (ส่งข้อมูลเป็น chunk ขนาด 256KB)
  0x24 = File Chunk ACK                  (ยืนยัน chunk)
  0x25 = File Transfer Complete          (ส่งครบแล้ว)
  0x26 = File Transfer Cancel            (ยกเลิก)
  0x29 = File Transfer Resume Request   (receiver บอก sender ให้เริ่มส่งใหม่จาก chunk index ที่ระบุ)

*(0x27, 0x28 ย้ายไป Clipboard Channel :9004 แล้ว — ป้องกัน HOL blocking ขณะ transfer ไฟล์)*

File Chunk Format:
┌──────────┬───────────┬──────────┬───────────────┬──────────────────┐
│ FileID(4)│ ChunkIdx(4)│ Total(4) │  CRC32 (4)    │  Data (≤256KB)   │
└──────────┴───────────┴──────────┴───────────────┴──────────────────┘

Resume Request Payload:
┌──────────┬─────────────┐
│ FileID(4)│ FromChunk(4)│   ← receiver ส่งหลัง reconnect เพื่อบอกว่า "ขอจาก chunk นี้เป็นต้นไป"
└──────────┴─────────────┘
```

### Audio Channel (UDP :9001-9002)
```
Audio Packet:
┌──────────┬───────────┬──────────────────┐
│ Seq (4)  │ TS (8)    │ Opus Frame (var) │
└──────────┴───────────┴──────────────────┘

- Sample Rate: 48000 Hz
- Channels: Stereo (2ch)
- Frame Size: 10ms (480 samples)
- Codec: Opus @ 128kbps
```

### Clipboard Channel (TCP :9004)
Connection แยกจาก File Transfer เพื่อป้องกัน Head-of-Line blocking — clipboard ส่งได้ทุกเวลาแม้กำลัง transfer ไฟล์อยู่

```
Clipboard Packet Types:
  0x27 = Clipboard Text Sync    (payload: UTF-8 string)
  0x28 = Clipboard Image Sync   (payload: PNG bytes, จำกัด ≤ 10MB)
  0x2A = Clipboard File Ref     (payload: filename + size — แนะนำให้ส่งเป็น File Transfer แทน)

Clipboard Text Payload:
┌─────────────┬────────────────────┐
│ Length (4)  │  UTF-8 Text (var)  │
└─────────────┴────────────────────┘
```

---

## 6. แผนการพัฒนา (Development Phases)

> **หมายเหตุ:** Phase 1-3 ใช้ minimal CLI + config file (TOML) สำหรับ test และ iterate ก่อน — GUI จะครอบ feature ทั้งหมดพร้อมกันใน Phase 4

### Phase 1 — Core Input Sharing (สัปดาห์ที่ 1-3)
- [ ] ออกแบบ Protocol และ Packet format
- [ ] สร้าง `netshare-core` crate (types, serialization)
- [ ] Implement TCP connection manager (server/client)
- [ ] **Windows**: Mouse/Keyboard capture ด้วย **LL Hooks** (WH_MOUSE_LL / WH_KEYBOARD_LL) — global, suppress event ไม่ให้ Server OS ได้รับ
- [ ] **Windows**: Mouse/Keyboard injection ด้วย SendInput บน Client
- [ ] **Linux**: Mouse/Keyboard capture ด้วย **evdev exclusive grab** — ไม่มี process อื่นเห็น event
- [ ] **Linux**: Mouse/Keyboard injection ด้วย **uinput** บน Client
- [ ] Implement **Active Client** concept: track active client, suppress/forward logic
- [ ] Implement **Hotkey switching**: `Ctrl+Shift+Alt+[1-9]` และ `Scroll Lock` cycle
- [ ] ทดสอบ Input latency (เป้าหมาย < 5ms บน LAN)

### Phase 2 — Audio Sharing (สัปดาห์ที่ 4-6)
- [ ] Setup CPAL สำหรับ audio capture/playback
- [ ] Integrate Opus encoder/decoder
- [ ] Implement UDP audio streaming
- [ ] **Windows**: ใช้ **VB-Cable** เป็น virtual audio device (detect อัตโนมัติ, แจ้ง user ถ้าไม่มี) — *(custom WDM driver เป็น stretch goal ใน Phase 5)*
- [ ] **Linux**: PipeWire/PulseAudio virtual sink/source
- [ ] ทดสอบ Audio latency (เป้าหมาย < 30ms บน LAN)
- [ ] Jitter buffer สำหรับ Audio stability

### Phase 3 — File Sharing (สัปดาห์ที่ 7-8)
- [ ] ออกแบบ File Transfer Protocol (Chunked TCP)
- [ ] Implement file sender: chunk splitter, CRC32 checksum, progress tracking
- [ ] Implement file receiver: reassemble chunks, verify checksum, resume ถ้า connection หาย
- [ ] รองรับการส่งทั้ง ไฟล์เดี่ยว และ โฟลเดอร์ (recursive)
- [ ] **Clipboard Sync** — ข้อความและรูปภาพที่ Copy บนเครื่องหนึ่ง ให้ Paste ได้บนอีกเครื่อง
- [ ] Resume interrupted transfer (ส่งต่อได้ถ้า connection ขาด)
- [ ] ทดสอบความเร็ว (เป้าหมาย ≥ 80-100 MB/s บน LAN 1Gbps)

### Phase 4 — GUI และ UX (สัปดาห์ที่ 9-10)
- [ ] System Tray icon (Windows + Linux)
- [ ] หน้า Settings: เลือก Client IP, Port, Audio device, Receive folder
- [ ] หน้า Monitor: แสดง Connection status, Latency, Active Client indicator
- [ ] **Active Client indicator** — แสดงชื่อเครื่องที่ active บน Tray tooltip และ status bar
- [ ] **Broadcast mode toggle** — อยู่ใน Advanced Settings พร้อม visual warning สีแดงเมื่อเปิด
- [ ] **Drag & Drop** ไฟล์จาก File Explorer ไปยัง GUI แล้วส่งไปอีกเครื่อง
- [ ] Progress bar และ transfer queue สำหรับ file transfer
- [ ] Auto-discovery ผ่าน mDNS (หาเครื่อง Client ในวงเดียวกัน)

### Phase 5 — Security & Polish (สัปดาห์ที่ 11-12)
- [ ] TLS สำหรับ Control channel และ File Transfer channel
- [ ] DTLS สำหรับ Audio channel
- [ ] Pairing code (ป้องกันการเชื่อมต่อจากเครื่องแปลกหน้า)
- [ ] File transfer permission prompt — ปลายทางต้องกด Accept ก่อนรับไฟล์
- [ ] Sandbox รับไฟล์ไปยัง folder ที่กำหนด (ป้องกัน path traversal attack)
- [ ] *(Stretch)* Custom WDM virtual audio driver สำหรับ Windows เพื่อไม่ต้องพึ่ง VB-Cable
- [ ] ทำ Installer: `.msi` สำหรับ Windows, `.deb` / `.AppImage` สำหรับ Linux
- [ ] เขียน Documentation และ README


---

## 7. File Sharing — รายละเอียดเพิ่มเติม

### ฟีเจอร์หลัก
| ฟีเจอร์ | รายละเอียด |
|--------|-----------|
| Send File/Folder | ส่งได้ทั้งไฟล์เดี่ยวและโฟลเดอร์ทั้งหมด (recursive) |
| Clipboard Sync | ข้อความ, รูปภาพ, และไฟล์ที่ Copy จะ sync ไปยังทุก Client |
| Drag & Drop | ลากไฟล์จาก File Explorer แล้วปล่อยใน GUI เพื่อส่ง |
| Transfer Queue | ส่งคิวหลายไฟล์พร้อมกัน แสดง progress แต่ละไฟล์ |
| Resume Transfer | ถ้า connection หลุดกลางทาง สามารถ resume ได้โดยไม่ต้องเริ่มใหม่ |
| Receive Folder | กำหนด folder ที่รับไฟล์เองได้ใน Settings |
| Permission Prompt | ปลายทางเห็น popup "X ต้องการส่งไฟล์ Y ขนาด Z" ให้กด Accept/Reject |

### File Transfer State Machine
```
[IDLE] → (Send Request) → [REQUESTING]
  → (Accept)  → [TRANSFERRING] → (All chunks ACK) → [COMPLETE]
  → (Reject)  → [REJECTED]
  → (Timeout) → [FAILED]
  → (Cancel)  → [CANCELLED]

[TRANSFERRING] → (Connection lost) → [PAUSED] → (Reconnect) → [RESUMING] → [TRANSFERRING]
```

### ความปลอดภัยของ File Transfer
- ไฟล์ที่รับจะถูกบันทึกใน **sandboxed folder** เท่านั้น (ป้องกัน path traversal เช่น `../../etc/passwd`)
- TLS encryption บน TCP :9003 ทุก transfer
- ตรวจสอบ CRC32 ทุก chunk และ SHA-256 ของไฟล์ทั้งหมดเมื่อรับครบ
- ไม่อนุญาต execute ไฟล์ที่รับมาโดยอัตโนมัติ

---

## 8. ความท้าทายทางเทคนิค (Technical Challenges)

### Input
| ความท้าทาย | แนวทางแก้ไข |
|------------|-------------|
| Capture input เมื่อ app ไม่ได้ focused | **LL Hooks** (Windows) และ **evdev exclusive grab** (Linux) — จับได้ทุก input ไม่ว่า app ไหนจะ active |
| **LL Hooks ต้องการ dedicated message loop thread** | Windows LL Hooks ต้องติดตั้งบน thread ที่รัน `GetMessage`/`DispatchMessage` loop และต้อง process ภายใน **300ms** หรือ Windows จะ bypass hook โดยอัตโนมัติ — tokio thread pool ไม่เหมาะ ต้องสร้าง **dedicated OS thread** แยกสำหรับ hook แล้วส่ง event ผ่าน `mpsc channel` ไปยัง tokio runtime |
| Keyboard shortcut ชน (เช่น Alt+Tab, Win key) | Intercept ใน LL Hook ก่อน OS ประมวลผล แล้ว suppress ไม่ส่งต่อ, forward ไป Client แทน |
| Hotkey switching ชนกับ AltGr keyboard (ยุโรป) | `Ctrl+Alt` = `AltGr` บน layout ยุโรป — ใช้ `Ctrl+Shift+Alt+[1-9]` เป็น default แทน และ allow user customize hotkey ได้ |
| Broadcast mode ส่ง input ไปหลายเครื่องพร้อมกัน | Fan-out ใน input dispatch loop, แสดง warning indicator สีแดงชัดเจน |
| Input latency บน Wireless LAN | Priority queue, ลด packet overhead |

### Audio
| ความท้าทาย | แนวทางแก้ไข |
|------------|-------------|
| Audio latency สูง | Jitter buffer แบบ adaptive + Opus low-delay mode |
| Packet loss ทำให้เสียงกระตุก | FEC (Forward Error Correction) ใน Opus |
| Windows ไม่มี Virtual Audio Device built-in | **Phase 2:** ใช้ VB-Cable (detect + แจ้ง user ถ้าไม่มี) / **Phase 5 stretch:** custom WDM driver เพื่อ bundle ใน installer ได้เลย |
| Echo/Feedback เมื่อแชร์ Mic + Speaker | ใช้ **`webrtc-audio-processing`** crate (WebRTC AEC module) — signal processing ซับซ้อน ไม่ควร implement เอง |

### File Sharing
| ความท้าทาย | แนวทางแก้ไข |
|------------|-------------|
| ส่งไฟล์ขนาดใหญ่ (หลาย GB) | Chunked transfer (256KB/chunk) + resume support |
| Connection หลุดกลางการส่ง | บันทึก offset ล่าสุด, reconnect แล้วส่งต่อจาก chunk ที่ค้างไว้ |
| ไฟล์เสียหายระหว่างส่ง | CRC32 ต่อ chunk + SHA-256 ของทั้งไฟล์ หากไม่ผ่านให้ re-request chunk นั้น |
| Clipboard sync รูปภาพขนาดใหญ่ | จำกัดขนาด clipboard image sync ที่ 10MB, ใหญ่กว่านั้นแนะนำให้ส่งเป็นไฟล์แทน |
| Path traversal attack | Sanitize path ทุก chunk, บันทึกเฉพาะใน receive folder ที่กำหนด |

---

## 9. Non-Goals (สิ่งที่ยังไม่ทำในโปรเจคนี้)

- ❌ Video/Screen sharing (ใช้ RDP, Parsec, Sunshine แทน)
- ❌ รองรับ macOS (อาจเพิ่มในอนาคต)
- ❌ USB Device forwarding (เช่น Webcam, Flash drive)
- ❌ รองรับ Wireless display (Miracast, AirPlay)
- ❌ Cloud-based file storage หรือ sync (ไม่ใช่ Dropbox/OneDrive replacement)
- ❌ File transfer ผ่าน Internet / WAN (รองรับเฉพาะ LAN)

---

## 10. โครงสร้างไฟล์เพิ่มเติมสำหรับ File Sharing

```
netshare/
└── crates/
    ├── netshare-core/
    │   └── src/
    │       └── file_transfer.rs    # Chunk types, FileID, State machine types
    │
    ├── netshare-server/
    │   └── src/
    │       ├── file_sender.rs      # ส่งไฟล์, สร้าง chunks, track ACK
    │       └── clipboard_server.rs # Capture + serve clipboard บน Server (:9004)
    │
    ├── netshare-client/
    │   └── src/
    │       ├── file_receiver.rs    # รับ chunks, reassemble, verify checksum
    │       └── clipboard_client.rs # Receive + inject clipboard บน Client (:9004)
    │
    └── netshare-gui/               # Monolithic binary: depend on server + client crates
        └── src/
            ├── main.rs             # เลือก mode (Server/Client) จาก CLI args หรือ startup screen
            ├── app.rs              # egui app loop
            └── tray.rs             # tray-icon integration
```

---

## 11. Dependencies สำคัญ (Cargo)

```toml
[dependencies]
# Networking
tokio = { version = "1", features = ["full"] }

# Serialization
serde = { version = "1", features = ["derive"] }
bincode = "2"

# Audio
cpal = "0.15"
opus = "0.3"            # ⚠️ Windows build ต้องการ libopus C library: ติดตั้งผ่าน vcpkg (`vcpkg install opus`) หรือ MSVC + prebuilt binary — ต้องระบุใน CI/CD และ CONTRIBUTING.md
webrtc-audio-processing = "0.3"   # AEC (Acoustic Echo Cancellation) — WebRTC module

# Input (Linux)
evdev = "0.12"

# Input (Windows)
windows = { version = "0.58", features = [
    "Win32_UI_WindowsAndMessaging",      # SetWindowsHookEx, WH_MOUSE_LL, WH_KEYBOARD_LL (Server capture)
    "Win32_UI_Input_KeyboardAndMouse",   # SendInput (Client inject)
] }

# GUI
egui = "0.28"
eframe = "0.28"
tray-icon = "0.14"      # System tray icon + menu (Windows + Linux) — eframe ไม่มี built-in

# mDNS Discovery
mdns-sd = "0.11"

# File Transfer / Hashing
sha2 = "0.10"           # SHA-256 สำหรับ verify ไฟล์ทั้งหมด
crc32fast = "1.4"       # CRC32 สำหรับ verify แต่ละ chunk
tokio-util = "0.7"      # Codec / framing สำหรับ TCP stream

# Clipboard
arboard = "3"           # Cross-platform clipboard (text + image)

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"
```

---

## 12. เกณฑ์ความสำเร็จ (Success Criteria)

| เกณฑ์ | เป้าหมาย |
|-------|---------|
| Input latency (LAN 1Gbps) | < 5 ms |
| Audio latency (LAN 1Gbps) | < 30 ms |
| Audio quality | ≥ 128 kbps Opus, 48kHz Stereo |
| File transfer speed (LAN 1Gbps) | ≥ 80-100 MB/s |
| File integrity | SHA-256 ผ่าน 100% ทุก transfer |
| Resume after disconnect | ส่งต่อได้ทุกครั้งโดยไม่เสียข้อมูล |
| Clipboard sync latency | < 100 ms |
| Stability | ทำงานต่อเนื่อง 8+ ชั่วโมงโดยไม่ crash |
| CPU Usage (Idle) | < 2% บนเครื่อง Server |
| รองรับ OS | Windows 11, Ubuntu 24.04+ |
| Security | มี Pairing + TLS + File Accept Prompt |

---

*สร้างเมื่อ: 2026-04-08 | อัปเดตล่าสุด: 2026-04-08 | เวอร์ชัน: 0.6 (Review R4: 0xF1 Hello Response, bincode commit, tray-icon dep, monolithic GUI arch, libopus build note)*