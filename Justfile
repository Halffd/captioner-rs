set dotenv-load := true

MODELS_DIR := env.get_or("MODELS_DIR", ".models")
DEFAULT_MODEL := "base"

@_default:
    @just --list

# Build the project
build:
    cargo build --release

# Build for development
build-dev:
    cargo build

# Check compilation
check:
    cargo check --all-targets

# Run tests
test:
    cargo test

# Download a Whisper model
download-model model:
    #!/usr/bin/env bash
    mkdir -p {{MODELS_DIR}}
    case {{model}} in
        tiny)   URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin" ;;
        base)   URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin" ;;
        small)  URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin" ;;
        medium) URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin" ;;
        large-v2) URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v2.bin" ;;
        large-v3) URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin" ;;
        *)
            echo "Unknown model: {{model}}"
            echo "Available: tiny, base, small, medium, large-v2, large-v3"
            exit 1
            ;;
    esac
    curl -L -o "{{MODELS_DIR}}/ggml-{{model}}.bin" "$URL"

# Download all common models
download-all:
    just download-model tiny
    just download-model base
    just download-model small

# List available audio devices
list-devices:
    cargo run -- list-devices

# Run with GUI
run-gui model=(DEFAULT_MODEL):
    cargo run -- -g {{model}}

# Run with GUI and Japanese language
run-gui-ja model=(DEFAULT_MODEL):
    cargo run -- -g {{model}} ja

# Run with web UI
run-web model=(DEFAULT_MODEL):
    cargo run -- -w {{model}}

# Run with specific device
run-device model=(DEFAULT_MODEL) device:
    cargo run -- -g {{model}} -i {{device}}

# Clean build artifacts
clean:
    cargo clean

# Update dependencies
update:
    cargo update

# Format code
fmt:
    cargo fmt

# Lint code
clippy:
    cargo clippy --all-targets -- -D warnings

# Build with specific target
build-target target:
    cargo build --release --target {{target}}

# Debug build
debug:
    RUST_LOG=debug cargo run -- -g base --debug