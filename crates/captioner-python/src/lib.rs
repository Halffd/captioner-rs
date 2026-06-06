use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use captioner_core::config::Config;
use captioner_core::TranscriptionResult;

const PYTHON_SCRIPT: &str = include_str!("backend.py");

pub struct PythonSttEngine {
    config: Config,
    transcription_tx: broadcast::Sender<TranscriptionResult>,
    stop_flag: Arc<AtomicBool>,
    recording_enabled: Arc<AtomicBool>,
}

impl PythonSttEngine {
    pub fn new(
        config: Config,
        transcription_tx: broadcast::Sender<TranscriptionResult>,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let recording_enabled = Arc::new(AtomicBool::new(true));

        Self {
            config,
            transcription_tx,
            stop_flag,
            recording_enabled,
        }
    }

    pub fn handle(&self) -> PythonSttHandle {
        PythonSttHandle {
            stop_flag: self.stop_flag.clone(),
            recording_enabled: self.recording_enabled.clone(),
            command_tx: None,
        }
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    pub async fn run(&mut self) {
        let python_bin = find_python();
        if python_bin.is_none() {
            error!("Python not found. Install Python 3.9+ with: pip install faster-whisper webrtcvad torch torchaudio pyaudio numpy");
            self.run_stub_loop().await;
            return;
        }

        let script_path = write_script_to_temp();

        let model_name = &self.config.model_name;
        let language = self.config.language.as_deref().unwrap_or("");
        let realtime_model = self.config.realtime_model.as_deref().unwrap_or(model_name);
        let webrtc_sensitivity = self.config.webrtc_sensitivity;
        let silero_sensitivity = self.config.silero_sensitivity;
        let min_gap = self.config.min_gap_between_recordings;
        let recording_scale = self.config.recording_scale;
        let input_device = self.config.input_device.as_deref().unwrap_or("");
        let realtime = self.config.realtime;
        let device = self.config.python_device.as_deref().unwrap_or("cpu");

        info!("Starting Python STT backend: model={}, realtime_model={}, lang={:?}, device={}",
            model_name, realtime_model, self.config.language, device);

        let mut child = match Command::new(python_bin.unwrap())
            .arg(&script_path)
            .arg("--model")
            .arg(model_name)
            .arg("--realtime-model")
            .arg(realtime_model)
            .arg("--language")
            .arg(if language.is_empty() { "auto" } else { language })
            .arg("--webrtc-sensitivity")
            .arg(webrtc_sensitivity.to_string())
            .arg("--silero-sensitivity")
            .arg(silero_sensitivity.to_string())
            .arg("--min-gap")
            .arg(min_gap.to_string())
            .arg("--recording-scale")
            .arg(recording_scale.to_string())
            .arg("--device")
            .arg(device)
            .arg("--input-device")
            .arg(if input_device.is_empty() { "default" } else { input_device })
            .arg(if realtime { "--enable-realtime" } else { "--no-realtime" })
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to spawn Python process: {}", e);
                self.run_stub_loop().await;
                return;
            }
        };

        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");

        let mut stdin_writer = StdinWriter::new(stdin);
        stdin_writer.send("start").await;

        let stderr_reader = BufReader::new(stderr);
        let stderr_stop = self.stop_flag.clone();
        tokio::spawn(async move {
            let mut lines = stderr_reader.lines();
            loop {
                if stderr_stop.load(Ordering::SeqCst) {
                    break;
                }
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if !line.is_empty() {
                            warn!("[python] {}", line);
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        });

        let mut reader = BufReader::new(stdout).lines();
        let recording = self.recording_enabled.clone();
        let tx = self.transcription_tx.clone();
        let stop = self.stop_flag.clone();

        info!("Python STT backend started, reading transcription output...");

        loop {
            if stop.load(Ordering::SeqCst) {
                break;
            }

            tokio::select! {
                line = reader.next_line() => {
                    match line {
                        Ok(Some(line)) => {
                            if line.is_empty() {
                                continue;
                            }
                            if let Ok(msg) = serde_json::from_str::<PythonMessage>(&line) {
                                match msg.msg_type.as_str() {
                                    "transcription" => {
                                        if !recording.load(Ordering::SeqCst) {
                                            continue;
                                        }
                                        let text = msg.text.trim().to_string();
                                        if !text.is_empty() {
                                            debug!("Python transcription: {}", text);
                                            let result = TranscriptionResult::new(text, false);
                                            let _ = tx.send(result);
                                        }
                                    }
                                    "realtime" => {
                                        if !recording.load(Ordering::SeqCst) {
                                            continue;
                                        }
                                        let text = msg.text.trim().to_string();
                                        if !text.is_empty() {
                                            let result = TranscriptionResult::new(text, true);
                                            let _ = tx.send(result);
                                        }
                                    }
                                    "status" => {
                                        info!("[python] {}", msg.text);
                                    }
                                    "error" => {
                                        error!("[python] {}", msg.text);
                                    }
                                    _ => {
                                        debug!("[python] unknown message: {}", line);
                                    }
                                }
                            } else {
                                debug!("[python] raw: {}", line);
                            }
                        }
                        Ok(None) => {
                            info!("Python process stdout closed");
                            break;
                        }
                        Err(e) => {
                            error!("Error reading Python output: {}", e);
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                }
            }
        }

        let _ = stdin_writer.send("quit").await;
        drop(stdin_writer);

        match tokio::time::timeout(
            std::time::Duration::from_secs(3),
            child.wait(),
        ).await {
            Ok(Ok(status)) => info!("Python process exited: {}", status),
            Ok(Err(e)) => warn!("Error waiting for Python process: {}", e),
            Err(_) => {
                warn!("Python process didn't exit in 3s, killing...");
                let _ = child.kill().await;
            }
        }

        let _ = std::fs::remove_file(&script_path);

        info!("Python STT backend stopped");
    }

    async fn run_stub_loop(&self) {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            if self.stop_flag.load(Ordering::SeqCst) {
                break;
            }
        }
    }
}

pub struct PythonSttHandle {
    pub stop_flag: Arc<AtomicBool>,
    pub recording_enabled: Arc<AtomicBool>,
    pub command_tx: Option<tokio::sync::mpsc::Sender<String>>,
}

impl PythonSttHandle {
    pub fn toggle_recording(&self) -> bool {
        let current = self.recording_enabled.load(Ordering::SeqCst);
        self.recording_enabled.store(!current, Ordering::SeqCst);
        !current
    }

    pub fn is_recording(&self) -> bool {
        self.recording_enabled.load(Ordering::SeqCst)
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }
}

#[derive(serde::Deserialize)]
struct PythonMessage {
    #[serde(rename = "type")]
    msg_type: String,
    text: String,
}

struct StdinWriter {
    stdin: ChildStdin,
}

impl StdinWriter {
    fn new(stdin: ChildStdin) -> Self {
        Self { stdin }
    }

    async fn send(&mut self, cmd: &str) {
        let line = format!("{}\n", cmd);
        let _ = self.stdin.write_all(line.as_bytes()).await;
        let _ = self.stdin.flush().await;
    }
}

fn find_python() -> Option<String> {
    for cmd in &["python3", "python"] {
        if which_exists(cmd) {
            return Some(cmd.to_string());
        }
    }
    None
}

pub fn check_python_deps() -> Vec<String> {
    let mut missing = Vec::new();
    let required = ["faster_whisper", "webrtcvad", "numpy"];

    if let Some(python) = find_python() {
        for module in &required {
            let status = std::process::Command::new(&python)
                .args(["-c", &format!("import {}", module)])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !status.map(|s| s.success()).unwrap_or(false) {
                missing.push(module.to_string());
            }
        }

        let status = std::process::Command::new(&python)
            .args(["-c", "import pyaudio"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if !status.map(|s| s.success()).unwrap_or(false) {
            missing.push("pyaudio".to_string());
        }

        let status = std::process::Command::new(&python)
            .args(["-c", "import torch"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        if !status.map(|s| s.success()).unwrap_or(false) {
            missing.push("torch (optional, for Silero VAD)".to_string());
        }
    } else {
        missing.push("python3".to_string());
    }

    missing
}

fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn write_script_to_temp() -> PathBuf {
    let dir = std::env::temp_dir().join("captioner");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("captioner_backend.py");
    let _ = std::fs::write(&path, PYTHON_SCRIPT);
    path
}
