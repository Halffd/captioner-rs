#!/usr/bin/env bash
set -e

MODEL_DIR="${CAPTIONER_MODELS_DIR:-$HOME/.cache/captioner/models}"
MODEL_NAME="${1:-base}"
BASE_URL="https://huggingface.co/ggerganov/whisper.cpp/resolve/main"

mkdir -p "$MODEL_DIR"

declare -A MODEL_URLS
MODEL_URLS["tiny"]="ggml-tiny.bin"
MODEL_URLS["base"]="ggml-base.bin"
MODEL_URLS["small"]="ggml-small.bin"
MODEL_URLS["medium"]="ggml-medium.bin"
MODEL_URLS["large-v2"]="ggml-large-v2.bin"
MODEL_URLS["large-v3"]="ggml-large-v3.bin"
MODEL_URLS["tiny.en"]="ggml-tiny.en.bin"
MODEL_URLS["base.en"]="ggml-base.en.bin"
MODEL_URLS["small.en"]="ggml-small.en.bin"
MODEL_URLS["medium.en"]="ggml-medium.en.bin"

if [[ -z "${MODEL_URLS[$MODEL_NAME]}" ]]; then
    echo "Unknown model: $MODEL_NAME"
    echo "Available models: ${!MODEL_URLS[@]}"
    exit 1
fi

FILENAME="${MODEL_URLS[$MODEL_NAME]}"
DEST="$MODEL_DIR/$FILENAME"

if [[ -f "$DEST" ]]; then
    echo "Model already exists at $DEST"
    exit 0
fi

echo "Downloading ggml-$MODEL_NAME to $DEST..."
curl -L -o "$DEST" "$BASE_URL/$FILENAME"

echo "Download complete: $DEST"
echo "File size: $(du -h "$DEST" | cut -f1)"