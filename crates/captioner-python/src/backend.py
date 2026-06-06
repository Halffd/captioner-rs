#!/usr/bin/env python3
"""Captioner Python STT Backend.

Uses faster-whisper (same engine as RealtimeSTT) with WebRTC VAD
and Silero VAD. Outputs JSON lines to stdout for the Rust host.

Communication protocol (one JSON object per line on stdout):
  {"type": "transcription", "text": "..."}  - final transcription
  {"type": "realtime", "text": "..."}       - realtime partial
  {"type": "status", "text": "..."}         - status message
  {"type": "error", "text": "..."}          - error message
"""

import argparse
import json
import os
import sys
import threading
import time
import queue
import select
import numpy as np

os.environ["KMP_DUPLICATE_LIB_OK"] = "TRUE"
os.environ["TQDM_DISABLE"] = "1"


def send_message(msg_type, text):
    try:
        line = json.dumps({"type": msg_type, "text": text}, ensure_ascii=False)
        sys.stdout.write(line + "\n")
        sys.stdout.flush()
    except Exception:
        pass


def read_stdin_commands(cmd_queue):
    """Read commands from stdin in a background thread."""
    while True:
        try:
            if select.select([sys.stdin], [], [], 0.1) == ([sys.stdin], [], []):
                line = sys.stdin.readline().strip()
                if line:
                    cmd_queue.put(line)
                elif line == "" and not sys.stdin.isatty():
                    break
        except Exception:
            break


def get_min_length_of_recording(lang, model_name, recording_scale):
    conditions = {
        ("en", "tiny.en"): 1.5,
        ("en", "tiny"): 1.75,
        ("en", "base.en"): 2,
        ("en", "base"): 2.25,
        ("en", "small.en"): 2.8,
        ("en", "small"): 2.85,
        ("en", "medium.en"): 3.8,
        ("en", "medium"): 3.85,
        ("en", "large"): 4.5,
        ("en", "large-v2"): 5,
        ("en", "large-v3"): 8,
        ("", "tiny"): 2,
        ("", "base"): 3,
        ("", "small"): 4,
        ("", "medium"): 5,
        ("", "large"): 7,
        ("", "large-v2"): 9,
        ("", "large-v3"): 12,
    }
    key = (lang, model_name)
    return conditions.get(key, 1) * recording_scale


def resolve_audio_device(device_name):
    try:
        import pyaudio
        p = pyaudio.PyAudio()
        if device_name == "default" or device_name == "":
            info = p.get_default_input_device_info()
            idx = info["index"]
            p.terminate()
            return idx
        try:
            idx = int(device_name)
            p.terminate()
            return idx
        except ValueError:
            pass
        for i in range(p.get_device_count()):
            info = p.get_device_info_by_index(i)
            if device_name.lower() in info["name"].lower() and info["maxInputChannels"] > 0:
                p.terminate()
                return info["index"]
        p.terminate()
    except Exception as e:
        send_message("error", f"Failed to resolve audio device: {e}")
    return None


class AudioRecorder:
    def __init__(self, input_device_index=None, sample_rate=16000, buffer_size=512):
        self.sample_rate = sample_rate
        self.buffer_size = buffer_size
        self.input_device_index = input_device_index
        self.audio_queue = queue.Queue()
        self._stop_event = threading.Event()
        self._thread = None
        self._pyaudio = None
        self._stream = None

    def start(self):
        import pyaudio
        self._stop_event.clear()
        self._pyaudio = pyaudio.PyAudio()
        try:
            self._stream = self._pyaudio.open(
                format=pyaudio.paInt16,
                channels=1,
                rate=self.sample_rate,
                input=True,
                frames_per_buffer=self.buffer_size,
                input_device_index=self.input_device_index,
            )
        except Exception as e:
            send_message("error", f"Failed to open audio stream: {e}")
            return False

        self._thread = threading.Thread(target=self._record_loop, daemon=True)
        self._thread.start()
        return True

    def _record_loop(self):
        while not self._stop_event.is_set():
            try:
                data = self._stream.read(self.buffer_size, exception_on_overflow=False)
                audio = np.frombuffer(data, dtype=np.int16).astype(np.float32) / 32768.0
                self.audio_queue.put(audio)
            except Exception:
                break

    def stop(self):
        self._stop_event.set()
        if self._stream:
            try:
                self._stream.stop_stream()
                self._stream.close()
            except Exception:
                pass
        if self._pyaudio:
            try:
                self._pyaudio.terminate()
            except Exception:
                pass
        if self._thread:
            self._thread.join(timeout=2)

    def read_chunk(self, timeout=0.1):
        try:
            return self.audio_queue.get(timeout=timeout)
        except queue.Empty:
            return None


class VoiceActivityDetector:
    def __init__(self, webrtc_sensitivity=0, silero_sensitivity=0.1):
        self.webrtc_sensitivity = webrtc_sensitivity
        self.silero_sensitivity = silero_sensitivity
        self._webrtc_vad = None
        self._silero_model = None
        self._silero_ready = False

    def init_webrtc(self):
        try:
            import webrtcvad
            self._webrtc_vad = webrtcvad.Vad(self.webrtc_sensitivity)
        except ImportError:
            send_message("error", "webrtcvad not installed: pip install webrtcvad")

    def init_silero(self):
        try:
            import torch
            self._silero_model, _ = torch.hub.load(
                repo_or_dir="snakers4/silero-vad",
                model="silero_vad",
                trust_repo=True,
            )
            self._silero_ready = True
        except Exception as e:
            send_message("status", f"Silero VAD not available: {e}")

    def is_speech_webrtc(self, audio_chunk_int16):
        if self._webrtc_vad is None:
            return True
        try:
            return self._webrtc_vad.is_speech(audio_chunk_int16.tobytes(), 16000)
        except Exception:
            return True

    def is_speech_silero(self, audio_float32):
        if not self._silero_ready:
            return True
        try:
            prob = self._silero_model(torch.from_numpy(audio_float32), 16000).item()
            threshold = 1.0 - self.silero_sensitivity
            return prob > threshold
        except Exception:
            return True


def main():
    parser = argparse.ArgumentParser(description="Captioner Python STT Backend")
    parser.add_argument("--model", default="base")
    parser.add_argument("--realtime-model", default=None)
    parser.add_argument("--language", default="auto")
    parser.add_argument("--webrtc-sensitivity", type=int, default=0)
    parser.add_argument("--silero-sensitivity", type=float, default=0.1)
    parser.add_argument("--min-gap", type=float, default=0.4)
    parser.add_argument("--recording-scale", type=float, default=1.25)
    parser.add_argument("--device", default="cpu")
    parser.add_argument("--input-device", default="default")
    parser.add_argument("--enable-realtime", action="store_true", default=True)
    parser.add_argument("--no-realtime", dest="enable_realtime", action="store_false")

    args = parser.parse_args()

    language = args.language if args.language != "auto" else None
    lang_key = "en" if language and language.startswith("en") else ""

    send_message("status", f"Loading model: {args.model}")

    try:
        from faster_whisper import WhisperModel
    except ImportError:
        send_message("error", "faster-whisper not installed. Run: pip install faster-whisper")
        sys.exit(1)

    try:
        model = WhisperModel(args.model, device=args.device, compute_type="default")
        send_message("status", f"Model loaded: {args.model}")
    except Exception as e:
        send_message("error", f"Failed to load model: {e}")
        sys.exit(1)

    realtime_model = None
    if args.enable_realtime and args.realtime_model and args.realtime_model != args.model:
        try:
            realtime_model = WhisperModel(args.realtime_model, device=args.device, compute_type="default")
            send_message("status", f"Realtime model loaded: {args.realtime_model}")
        except Exception as e:
            send_message("error", f"Failed to load realtime model: {e}")
            realtime_model = None

    device_index = resolve_audio_device(args.input_device)
    if device_index is not None:
        send_message("status", f"Audio device index: {device_index}")

recorder = AudioRecorder(input_device_index=device_index)
if not recorder.start():
    send_message("error", "Failed to start audio recording")
    sys.exit(1)

send_message("status", "Audio recording started")

cmd_queue = queue.Queue()
cmd_thread = threading.Thread(target=read_stdin_commands, args=(cmd_queue,), daemon=True)
cmd_thread.start()

paused = False

    vad = VoiceActivityDetector(args.webrtc_sensitivity, args.silero_sensitivity)
    vad.init_webrtc()
    vad.init_silero()

    min_length = get_min_length_of_recording(lang_key, args.model, args.recording_scale) / 3
    post_speech_silence = 0.16 / args.recording_scale
    min_gap = args.min_gap

    CHUNKS_PER_SECOND = 16000 / 512
    silence_chunks_threshold = int(post_speech_silence * CHUNKS_PER_SECOND)
    min_recording_chunks = int(min_length * CHUNKS_PER_SECOND)
    gap_chunks = int(min_gap * CHUNKS_PER_SECOND)
    max_buffer_chunks = int(60 * CHUNKS_PER_SECOND)
    realtime_chunks = int(0.2 * CHUNKS_PER_SECOND)

    send_message("status", f"VAD config: webrtc={args.webrtc_sensitivity}, silero={args.silero_sensitivity}")
    send_message("status", f"Timing: min_length={min_length:.2f}s, post_silence={post_speech_silence:.3f}s, min_gap={min_gap:.2f}s")

    buffer = []
    silence_chunks = 0
    gap_counter = 0
    is_recording_segment = False
    realtime_counter = 0

    send_message("status", "Listening...")

try:
    while True:
        while not cmd_queue.empty():
            cmd = cmd_queue.get_nowait()
            if cmd == "quit":
                send_message("status", "Received quit command")
                raise KeyboardInterrupt
            elif cmd == "toggle_recording":
                paused = not paused
                send_message("status", f"Recording {'paused' if paused else 'resumed'}")
            elif cmd == "clear":
                buffer.clear()
                is_recording_segment = False
                silence_chunks = 0
                gap_counter = 0
                send_message("status", "Buffer cleared")

        if paused:
            time.sleep(0.1)
            continue

        chunk = recorder.read_chunk(timeout=0.1)
            if chunk is None:
                continue

            chunk_int16 = (chunk * 32768).astype(np.int16)
            has_speech_webrtc = vad.is_speech_webrtc(chunk_int16)
            has_speech_silero = vad.is_speech_silero(chunk)
            has_speech = has_speech_webrtc and has_speech_silero

            if has_speech:
                buffer.append(chunk)
                silence_chunks = 0
                gap_counter = 0
                if not is_recording_segment:
                    is_recording_segment = True
                    send_message("status", "Speech detected")
            elif is_recording_segment:
                buffer.append(chunk)
                silence_chunks += 1

                if silence_chunks >= silence_chunks_threshold and len(buffer) >= min_recording_chunks:
                    audio = np.concatenate(buffer)
                    buffer.clear()
                    is_recording_segment = False
                    silence_chunks = 0

                    send_message("status", "Transcribing...")
                    try:
                        segments, info = model.transcribe(
                            audio,
                            language=language,
                            beam_size=5,
                            vad_filter=False,
                        )
                        text = " ".join(s.text.strip() for s in segments).strip()
                        if text:
                            send_message("transcription", text)
                    except Exception as e:
                        send_message("error", f"Transcription error: {e}")

                    gap_counter = 0
                    continue

                if len(buffer) >= max_buffer_chunks:
                    audio = np.concatenate(buffer)
                    buffer.clear()
                    is_recording_segment = False
                    silence_chunks = 0

                    send_message("status", "Buffer full (60s), transcribing...")
                    try:
                        segments, info = model.transcribe(
                            audio,
                            language=language,
                            beam_size=5,
                            vad_filter=False,
                        )
                        text = " ".join(s.text.strip() for s in segments).strip()
                        if text:
                            send_message("transcription", text)
                    except Exception as e:
                        send_message("error", f"Transcription error: {e}")
            else:
                gap_counter += 1
                if is_recording_segment and gap_counter > gap_chunks:
                    if len(buffer) >= min_recording_chunks:
                        audio = np.concatenate(buffer)
                        buffer.clear()
                        is_recording_segment = False
                        silence_chunks = 0
                        gap_counter = 0

                        try:
                            segments, info = model.transcribe(
                                audio,
                                language=language,
                                beam_size=5,
                                vad_filter=False,
                            )
                            text = " ".join(s.text.strip() for s in segments).strip()
                            if text:
                                send_message("transcription", text)
                        except Exception as e:
                            send_message("error", f"Transcription error: {e}")
                    else:
                        buffer.clear()
                        is_recording_segment = False
                        silence_chunks = 0
                        gap_counter = 0

            if args.enable_realtime and realtime_model is not None and is_recording_segment:
                realtime_counter += 1
                if realtime_counter >= realtime_chunks and len(buffer) >= min_recording_chunks:
                    realtime_counter = 0
                    audio = np.concatenate(buffer)
                    try:
                        segments, info = realtime_model.transcribe(
                            audio,
                            language=language,
                            beam_size=3,
                            vad_filter=False,
                        )
                        text = " ".join(s.text.strip() for s in segments).strip()
                        if text:
                            send_message("realtime", text)
                    except Exception:
                        pass

    except KeyboardInterrupt:
        pass
    finally:
        recorder.stop()
        send_message("status", "Stopped")


if __name__ == "__main__":
    main()
