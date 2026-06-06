use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::{info, warn, error, debug};

use super::config::{get_model_path, Config, TranscriptionResult};
use super::error::CaptionerError;
use super::vad::VoiceActivityDetector;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContextParameters, WhisperContext};

#[derive(Clone, Debug)]
pub enum SttCommand {
    SetModel(String),
    SetLanguage(Option<String>),
}

#[derive(Clone)]
pub struct SttHandle {
    pub transcription_tx: broadcast::Sender<TranscriptionResult>,
    pub stop_flag: Arc<AtomicBool>,
    pub recording_enabled: Arc<AtomicBool>,
    pub command_tx: mpsc::Sender<SttCommand>,
}

impl SttHandle {
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

    pub fn set_model(&self, model_name: String) -> Result<(), mpsc::error::TrySendError<SttCommand>> {
        self.command_tx.try_send(SttCommand::SetModel(model_name))
    }

    pub fn set_language(&self, language: Option<String>) -> Result<(), mpsc::error::TrySendError<SttCommand>> {
        self.command_tx.try_send(SttCommand::SetLanguage(language))
    }
}

pub struct SttEngine {
    config: Config,
    transcription_tx: broadcast::Sender<TranscriptionResult>,
    audio_rx: mpsc::Receiver<Vec<f32>>,
    stop_flag: Arc<AtomicBool>,
    recording_enabled: Arc<AtomicBool>,
    command_rx: mpsc::Receiver<SttCommand>,
    command_tx: mpsc::Sender<SttCommand>,
    ctx: Option<WhisperContext>,
}

const MIN_RECORDING_SAMPLES: usize = 16000;
const SILENCE_CHUNKS_THRESHOLD: usize = 20;
const MAX_BUFFER_SAMPLES: usize = 16000 * 60;

impl SttEngine {
    pub fn new(
        config: Config,
        audio_rx: mpsc::Receiver<Vec<f32>>,
        transcription_tx: broadcast::Sender<TranscriptionResult>,
    ) -> Self {
        let stop_flag = Arc::new(AtomicBool::new(false));
        let recording_enabled = Arc::new(AtomicBool::new(true));
        let (command_tx, command_rx) = mpsc::channel::<SttCommand>(4);

        Self {
            config,
            transcription_tx,
            audio_rx,
            stop_flag,
            recording_enabled,
            command_rx,
            command_tx,
            ctx: None,
        }
    }

    pub fn handle(&self) -> SttHandle {
        SttHandle {
            transcription_tx: self.transcription_tx.clone(),
            stop_flag: self.stop_flag.clone(),
            recording_enabled: self.recording_enabled.clone(),
            command_tx: self.command_tx.clone(),
        }
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    fn load_model(&mut self) -> Result<(), CaptionerError> {
        let model_path = get_model_path(&self.config.model_name);

        if !model_path.exists() {
            return Err(CaptionerError::Whisper(format!(
                "Model file not found: {}. Download it first.",
                model_path.display()
            )));
        }

        let mut params = WhisperContextParameters::new();
        params.use_gpu = false;

        let ctx = WhisperContext::new_with_params(&model_path, params)
            .map_err(|e| CaptionerError::Whisper(format!("Failed to load model: {:?}", e)))?;

        info!("Whisper model loaded: {}", self.config.model_name);
        self.ctx = Some(ctx);
        Ok(())
    }

    fn reload_model(&mut self, model_name: &str) -> Result<(), CaptionerError> {
        let model_path = get_model_path(model_name);

        if !model_path.exists() {
            return Err(CaptionerError::Whisper(format!(
                "Model file not found: {}. Download it first.",
                model_path.display()
            )));
        }

        let mut params = WhisperContextParameters::new();
        params.use_gpu = false;

        let ctx = WhisperContext::new_with_params(&model_path, params)
            .map_err(|e| CaptionerError::Whisper(format!("Failed to load model: {:?}", e)))?;

        info!("Whisper model reloaded: {} -> {}", self.config.model_name, model_name);
        self.config.model_name = model_name.to_string();
        self.ctx = Some(ctx);
        Ok(())
    }

    fn transcribe(&mut self, audio: &[f32]) -> Result<String, CaptionerError> {
        let ctx = self.ctx.as_ref().ok_or_else(|| {
            CaptionerError::Whisper("Model not loaded".to_string())
        })?;

        let mut state = ctx.create_state()
            .map_err(|e| CaptionerError::Whisper(format!("Failed to create state: {:?}", e)))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy {
            best_of: 0,
        });

        params.set_n_threads(4);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_print_special(false);
        params.set_single_segment(false);
        params.set_no_timestamps(true);
        params.set_translate(false);
        params.set_suppress_blank(true);

        if let Some(ref lang) = self.config.language {
            params.set_language(Some(lang.as_str()));
        }

        state.full(params, audio)
            .map_err(|e| CaptionerError::Whisper(format!("Transcription failed: {:?}", e)))?;

        let n_segments = state.full_n_segments();
        let mut text = String::new();

        for i in 0..n_segments {
            if let Some(segment) = state.get_segment(i) {
                if let Ok(s) = segment.to_str() {
                    let trimmed = s.trim();
                    if !trimmed.is_empty() {
                        if !text.is_empty() {
                            text.push(' ');
                        }
                        text.push_str(trimmed);
                    }
                }
            }
        }

        Ok(text)
    }

    pub async fn run(&mut self) {
        if let Err(e) = self.load_model() {
            error!("Failed to load STT model: {}", e);
            info!("Running in stub mode - no real transcription");

            loop {
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                        if self.stop_flag.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                    Some(_) = self.audio_rx.recv() => {}
                    Some(cmd) = self.command_rx.recv() => {
                        match cmd {
                            SttCommand::SetModel(model_name) => {
                                match self.reload_model(&model_name) {
                                    Ok(()) => {
                                        info!("Model loaded successfully, exiting stub mode");
                                        break;
                                    }
                                    Err(e) => {
                                        error!("Failed to load model '{}': {}", model_name, e);
                                    }
                                }
                            }
                            SttCommand::SetLanguage(lang) => {
                                self.config.language = lang;
                                info!("Language updated (in stub mode)");
                            }
                        }
                    }
                }
            }

            if self.ctx.is_none() {
                return;
            }
        }

        info!("STT engine running with model: {}", self.config.model_name);

        let mut vad = VoiceActivityDetector::new(self.config.webrtc_sensitivity);
        let mut recording_buffer: Vec<f32> = Vec::with_capacity(MAX_BUFFER_SAMPLES);
        let mut silence_chunks = 0usize;
        let mut is_recording_segment = false;

        loop {
            if self.stop_flag.load(Ordering::SeqCst) {
                if is_recording_segment && recording_buffer.len() >= MIN_RECORDING_SAMPLES {
                    self.flush_transcription(&mut recording_buffer);
                }
                break;
            }

            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                    if is_recording_segment {
                        silence_chunks += 1;
                        if silence_chunks >= SILENCE_CHUNKS_THRESHOLD
                            && recording_buffer.len() >= MIN_RECORDING_SAMPLES
                        {
                            debug!("Silence timeout, transcribing {} samples", recording_buffer.len());
                            self.flush_transcription(&mut recording_buffer);
                            is_recording_segment = false;
                            silence_chunks = 0;
                        }
                    }
                }
                Some(audio_chunk) = self.audio_rx.recv() => {
                    if !self.recording_enabled.load(Ordering::SeqCst) {
                        continue;
                    }

                    let is_speech = vad.is_speech(&audio_chunk);

                    if is_speech {
                        recording_buffer.extend_from_slice(&audio_chunk);
                        silence_chunks = 0;
                        is_recording_segment = true;
                    } else if is_recording_segment {
                        recording_buffer.extend_from_slice(&audio_chunk);
                        silence_chunks += 1;
                    }

                    if recording_buffer.len() >= MAX_BUFFER_SAMPLES {
                        debug!("Buffer full (60s), transcribing {} samples", recording_buffer.len());
                        self.flush_transcription(&mut recording_buffer);
                        is_recording_segment = false;
                        silence_chunks = 0;
                    }
                }
                Some(cmd) = self.command_rx.recv() => {
                    if is_recording_segment && recording_buffer.len() >= MIN_RECORDING_SAMPLES {
                        self.flush_transcription(&mut recording_buffer);
                        is_recording_segment = false;
                        silence_chunks = 0;
                    }
                    match cmd {
                        SttCommand::SetModel(model_name) => {
                            match self.reload_model(&model_name) {
                                Ok(()) => info!("Switched to model: {}", model_name),
                                Err(e) => error!("Failed to switch model to '{}': {}", model_name, e),
                            }
                        }
                        SttCommand::SetLanguage(lang) => {
                            self.config.language = lang;
                            info!("Language changed to: {:?}", self.config.language);
                        }
                    }
                }
            }
        }

        info!("STT engine stopped");
    }

    fn flush_transcription(&mut self, buffer: &mut Vec<f32>) {
        if buffer.is_empty() {
            return;
        }

        let audio = std::mem::take(buffer);

        match self.transcribe(&audio) {
            Ok(text) => {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    info!("Transcription: {}", trimmed);
                    let result = TranscriptionResult::new(trimmed.to_string(), false);
                    let _ = self.transcription_tx.send(result);
                }
            }
            Err(e) => {
                warn!("Transcription error: {}", e);
            }
        }
    }
}

pub fn validate_model_name(name: &str) -> bool {
    super::config::MODEL_NAMES.contains(&name)
}

pub fn list_models() -> Vec<&'static str> {
    super::config::MODEL_NAMES.to_vec()
}

pub fn model_default() -> &'static str {
    "base"
}
