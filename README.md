# Captioner-RS

A fast, native Rust implementation of real-time speech-to-text captioning, rewritten from
the Python original ([captioner-py](https://github.com/nicat-as/captioner-py)) in Rust.

## Features

- **Real-time speech recognition** using Whisper via [whisper-rs](https://github.com/tazz4843/whisper-rs)
- **Native overlay GUI** using [egui](https://github.com/emilk/egui) with transparent, frameless, always-on-top display
- **Web interface** with real-time SSE streaming using [axum](https://github.com/tokio-rs/axum)
- **Global hotkeys** for recording toggle, zoom, transparency, and window control via [rdev](https://github.com/a1expert/rdev)
- **Voice activity detection** using WebRTC VAD to filter silence
- **Japanese furigana support** via [lindera](https://github.com/linderaNLP/lindera)
- **Duplicate text filtering** with simple TF-IDF cosine similarity
- **Daily-rotating log files** with year/month/day nested directory structure
- **Audio device selection** by name substring or index
- **Multiple Whisper model support**: tiny, base, small, medium, large-v2, large-v3, Distil-Whisper variants

## Architecture

```
captioner-rs/
├── Cargo.toml                      # Workspace
├── Justfile                        # Build automation
├── README.md
└── crates/
    ├── captioner/                  # Binary - wires everything together
    ├── captioner-core/            # Core library: config, audio, STT, VAD
    ├── captioner-gui/             # egui overlay window
    ├── captioner-web/             # axum web server
    ├── captioner-input/           # Global hotkey listener
    └── captioner-log/             # Daily-rotating log files
```

### Data Flow

```
Audio Capture (cpal)
    ↓ f32 samples (16kHz mono)
Voice Activity Detection (webrtc-vad)
    ↓ speech chunks
Whisper STT (whisper-rs)
    ↓ TranscriptionResult (broadcast channel)
    ├→ GUI (egui overlay)
    ├→ Web Server (SSE) → browser
    └→ Log file
```

## Building

### Prerequisites

- Rust 1.75+ (with Cargo)
- C++ compiler (for compiling whisper.cpp from source)
- ~2GB RAM for larger models

### Build

```bash
# Clone and enter directory
cd captioner-rs

# Build release
cargo build --release
```

Building whisper-rs from source (whisper.cpp) can take 10+ minutes on first build.

### Download Models

GGML format models from [huggingface](https://huggingface.co/ggerganov/whisper.cpp/tree/main):

```bash
# Using just (recommended)
just download-model base

# Or manually
mkdir -p ~/.cache/captioner/models
curl -L -o ~/.cache/captioner/models/ggml-base.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin
```

### Usage

```bash
# GUI mode (default)
cargo run --release -- -g base

# GUI with Japanese language
cargo run --release -- -g base ja

# Web interface (http://localhost:5000)
cargo run --release -- -w base

# List audio devices
cargo run --release -- --list-devices

# Specific input device
cargo run --release -- -g base -i "USB Microphone"
```

### Command-Line Options

```
Usage: captioner [OPTIONS] [MODEL] [REALTIME_MODEL] [LANGUAGE]

Args:
  MODEL           Whisper model name or index [default: base]
  REALTIME_MODEL  Model for realtime transcription [default: MODEL]
  LANGUAGE        Language code (e.g. en, ja, pt) [format: xx or xx-XX]

Options:
  -g, --gui              Launch GUI overlay
  -w, --web              Launch web UI
  --debug                Enable debug logging
  --test                 Enable test mode
  -i, --input <DEVICE>   Audio input device (name or index)
  -o, --output <DEVICE>  Audio output device for monitor capture
  --list-devices         List audio devices and exit
  -nrt, --no-realtime    Disable realtime transcription
```

### GUI Hotkeys

| Key | Action |
|-----|--------|
| `+` / `=` | Zoom in (increase font size) |
| `-` | Zoom out (decrease font size) |
| `9` | More transparent |
| `0` | Less transparent |
| `X` | Clear text |
| `Alt+Arrow` | Move window |
| `B` | Toggle top/bottom position |
| `Space` | Move to other monitor |
| `F` | Toggle fullscreen |
| `W` | Full width |
| `H` | Full height |
| `T` | Settings dialog |
| `Ctrl+Scroll` | Zoom |
| `Ctrl+Alt+Up/Down/Left/Right` | Resize window |

### Global Hotkeys (work even when app not focused)

| Key | Action |
|-----|--------|
| `Ctrl+Enter` | Toggle recording |
| `Ctrl+Shift+Backspace` | Show settings |
| `Ctrl+Shift+=` | Increase font size |
| `Ctrl+Shift+-` | Decrease font size |
| `Ctrl+Alt+X` | Clear text |
| `Ctrl+PageUp` | Increase transparency |
| `Ctrl+PageDown` | Decrease transparency |
| `Ctrl+Alt+Arrow` | Resize window |

## Keyboard Shortcuts from Python Version (to be verified)

The original Python app had extensive keyboard shortcuts. The Rust version maps most of them:

- `Ctrl+Shift+=` → font increase
- `Ctrl+Shift+-` → font decrease
- `Ctrl+Alt+X` → clear
- `Ctrl+PageUp/PageDown` → transparency
- `Ctrl+Alt+Arrows` → window resize
- `Ctrl+Alt+Minus/Equal` → zoom (same as font)

## Status

**Under development.** The core architecture and modules are implemented but this has not been compiled successfully in this environment due to read-only filesystem constraints. Expected remaining work:

1. Fix any API mismatches revealed by `cargo build`
2. Verify whisper-rs 0.16 API compatibility
3. Verify cpal 0.15 audio stream building API
4. Verify eframe 0.29 API for overlay window
5. Test end-to-end with actual hardware

## Model Files

 whisper.cpp uses GGML format models (.bin files). Download from:
- https://huggingface.co/ggerganov/whisper.cpp (GGML models)
- Models: `ggml-tiny.bin`, `ggml-base.bin`, `ggml-small.bin`, `ggml-medium.bin`, `ggml-large-v2.bin`, `ggml-large-v3.bin`, etc.

Default model directory: `~/.cache/captioner/models/` or `./models/`

## Dependencies

| Purpose | Crate |
|---------|-------|
| STT | `whisper-rs` 0.16 (whisper.cpp bindings) |
| Audio capture | `cpal` 0.15 |
| GUI | `eframe`/`egui` 0.29 |
| Web server | `axum` 0.7, `tower-http` |
| Global hotkeys | `rdev` |
| VAD | `webrtc-vad` 0.4 |
| CLI | `clap` 4 |
| Japanese tokenization | `lindera` 0.27 |
| Async runtime | `tokio` 1 |

## Comparison with Python Version

| Feature | Python | Rust |
|---------|--------|------|
| STT | faster-whisper (CTranslate2) | whisper-rs (whisper.cpp) |
| GUI | PyQt5 | egui/eframe |
| Web | Flask | axum |
| Hotkeys | pynput | rdev |
| Audio | pyaudio | cpal |
| VAD | Silero + webrtc-vad | webrtc-vad |
| Performance | Good | Better (native) |
| Binary size | N/A (Python) | ~50-100MB |

## License

MIT