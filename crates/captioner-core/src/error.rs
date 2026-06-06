use thiserror::Error;

#[derive(Error, Debug)]
pub enum CaptionerError {
    #[error("Audio device error: {0}")]
    AudioDevice(String),

    #[error("Audio capture error: {0}")]
    AudioCapture(String),

    #[error("Whisper error: {0}")]
    Whisper(String),

    #[error("VAD error: {0}")]
    Vad(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Tokio broadcast error: {0}")]
    Broadcast(#[from] tokio::sync::broadcast::error::RecvError),

    #[error("Channel send error")]
    ChannelSend,
}