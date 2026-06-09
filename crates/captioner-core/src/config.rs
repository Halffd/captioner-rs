use std::path::{Path, PathBuf};

use chrono::{DateTime, Local};
use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

pub const MODEL_NAMES: &[&str] = &[
    "tiny",
    "base",
    "small",
    "medium",
    "large",
    "large-v2",
    "large-v3",
    "tiny.en",
    "base.en",
    "small.en",
    "medium.en",
    "jlondonobo/whisper-medium-pt",
    "clu-ling/whisper-large-v2-japanese-5k-steps",
    "distil-whisper/distil-medium.en",
    "distil-whisper/distil-small.en",
    "distil-whisper/distil-base",
    "distil-whisper/distil-small",
    "distil-whisper/distil-medium",
    "distil-whisper/distil-large",
    "distil-whisper/distil-large-v2",
    "distil-whisper/distil-large-v3",
    "Systran/faster-distil-medium",
    "Systran/faster-distil-large",
    "Systran/faster-distil-large-v2",
    "Systran/faster-distil-large-v3",
    "japanese-asr/distil-whisper-large-v3-ja-reazonspeech-large",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub model_name: String,
    pub realtime_model: Option<String>,
    pub language: Option<String>,
    pub gui: bool,
    pub web: bool,
    pub web_port: u16,
    pub hotkey: bool,
    pub debug_mode: bool,
    pub test_mode: bool,
    pub realtime: bool,
    pub path: PathBuf,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub recording_scale: f32,
    pub silero_sensitivity: f32,
    pub webrtc_sensitivity: u32,
    pub min_gap_between_recordings: f32,
    pub list_devices: bool,
    pub history: Option<usize>,
    pub transcription_backend: TranscriptionBackend,
    pub python_device: Option<String>,
    pub use_gpu: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionBackend {
    #[default]
    WhisperCpp,
    Python,
    Stub,
}

impl std::fmt::Display for TranscriptionBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WhisperCpp => write!(f, "whisper-cpp"),
            Self::Python => write!(f, "python"),
            Self::Stub => write!(f, "stub"),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = get_data_dir();
        Self {
            model_name: "base".to_string(),
            realtime_model: None,
            language: None,
            gui: false,
            web: false,
            web_port: 5000,
            hotkey: true,
            debug_mode: false,
            test_mode: false,
            realtime: true,
            path: data_dir,
            input_device: None,
            output_device: None,
            recording_scale: 1.25,
            silero_sensitivity: 0.1,
            webrtc_sensitivity: 0,
            min_gap_between_recordings: 0.4,
            list_devices: false,
            history: None,
            transcription_backend: TranscriptionBackend::WhisperCpp,
            python_device: None,
            use_gpu: false,
        }
    }
}

impl Config {
    pub fn realtime_model(&self) -> &str {
        self.realtime_model.as_deref().unwrap_or(&self.model_name)
    }
}

#[derive(Clone, Debug)]
pub struct TranscriptionResult {
    pub text: String,
    pub is_realtime: bool,
    pub timestamp: DateTime<Local>,
}

impl TranscriptionResult {
    pub fn new(text: String, is_realtime: bool) -> Self {
        Self {
            text,
            is_realtime,
            timestamp: Local::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum AppCommand {
    ToggleRecording,
    SetModel(String),
    SetLanguage(Option<String>),
    SetRecordingScale(f32),
    DownloadModel(String),
    Clear,
    Quit,
    IncreaseFontSize,
    DecreaseFontSize,
    IncreaseTransparency,
    DecreaseTransparency,
    IncreaseWindowWidth,
    DecreaseWindowWidth,
    IncreaseWindowHeight,
    DecreaseWindowHeight,
    MoveMonitor,
    ToggleTop,
    ToggleFullscreen,
    ToggleFullwidth,
    ToggleFullheight,
    ShowSettings,
}

#[derive(Debug, Clone)]
pub enum HotkeyEvent {
    ToggleRecording,
    IncreaseFontSize,
    DecreaseFontSize,
    ClearText,
    IncreaseTransparency,
    DecreaseTransparency,
    IncreaseWindowHeight,
    DecreaseWindowHeight,
    DecreaseWindowWidth,
    IncreaseWindowWidth,
    MoveMonitor,
    ToggleTop,
    ShowSettings,
    ToggleFullscreen,
    ToggleFullwidth,
    ToggleFullheight,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Language<'a> {
    English,
    Other(&'a str),
}

pub fn detect_language(lang: Option<&str>) -> Language<'_> {
    match lang {
        Some(l) if l.starts_with("en") => Language::English,
        Some(l) => Language::Other(l),
        None => Language::Other(""),
    }
}

pub fn get_data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "captioner", "captioner")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn get_config_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "captioner", "captioner")
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn get_config_path() -> PathBuf {
    get_config_dir().join("config.toml")
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct TomlConfig {
    model: Option<String>,
    realtime_model: Option<String>,
    language: Option<String>,
    gui: Option<bool>,
    web: Option<bool>,
    web_port: Option<u16>,
    hotkey: Option<bool>,
    debug: Option<bool>,
    test: Option<bool>,
    realtime: Option<bool>,
    path: Option<String>,
    input_device: Option<String>,
    output_device: Option<String>,
    recording_scale: Option<f32>,
    silero_sensitivity: Option<f32>,
    webrtc_sensitivity: Option<u32>,
    min_gap: Option<f32>,
    default_model: Option<String>,
    transcription_backend: Option<TranscriptionBackend>,
    python_device: Option<String>,
    use_gpu: Option<bool>,
}

fn load_toml_config() -> TomlConfig {
    let config_path = get_config_path();
    if !config_path.exists() {
        return TomlConfig::default();
    }

    match std::fs::read_to_string(&config_path) {
        Ok(contents) => match toml::from_str(&contents) {
            Ok(cfg) => {
                info!("Loaded config from {}", config_path.display());
                cfg
            }
            Err(e) => {
                warn!("Failed to parse {}: {}", config_path.display(), e);
                TomlConfig::default()
            }
        },
        Err(e) => {
            warn!("Failed to read {}: {}", config_path.display(), e);
            TomlConfig::default()
        }
    }
}

pub fn ensure_data_dirs() -> PathBuf {
    let data_dir = get_data_dir();
    let logs_dir = data_dir.join("Logs");
    let models_dir = data_dir.join("models");

    if let Err(e) = std::fs::create_dir_all(&logs_dir) {
        warn!("Could not create logs dir: {}", e);
    }
    if let Err(e) = std::fs::create_dir_all(&models_dir) {
        warn!("Could not create models dir: {}", e);
    }

    data_dir
}

pub fn scan_available_models() -> Vec<String> {
    let models_dir = get_data_dir().join("models");
    let mut found = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&models_dir) {
        for entry in entries.flatten() {
            if let Some(name) = entry.file_name().to_str() {
                if name.starts_with("ggml-") && (name.ends_with(".bin") || name.ends_with(".ggml"))
                {
                    let model_name = name
                        .strip_prefix("ggml-")
                        .unwrap_or(name)
                        .trim_end_matches(".bin")
                        .trim_end_matches(".ggml")
                        .to_string();
                    found.push(model_name);
                }
            }
        }
    }

    found.sort();
    found
}

pub fn print_history(limit: usize) {
    let logs_root = get_data_dir().join("Logs");
    if !logs_root.exists() {
        println!("No log directory found at {}", logs_root.display());
        return;
    }

    let mut log_files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    collect_log_files(&logs_root, &mut log_files);
    log_files.sort_by_key(|b| std::cmp::Reverse(b.0));

    let files_to_show: Vec<_> = log_files.into_iter().take(limit).collect();

    if files_to_show.is_empty() {
        println!("No log files found.");
        return;
    }

    for (_, path) in &files_to_show {
        println!("--- {} ---", path.display());
        if let Ok(contents) = std::fs::read_to_string(path) {
            let lines: Vec<&str> = contents.lines().collect();
            let start = lines.len().saturating_sub(50);
            for line in &lines[start..] {
                println!("  {}", line);
            }
        }
        println!();
    }
}

fn collect_log_files(dir: &Path, acc: &mut Vec<(std::time::SystemTime, PathBuf)>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_log_files(&path, acc);
            } else {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if ext == "log" || ext == "txt" {
                    if let Ok(meta) = entry.metadata() {
                        if let Ok(modified) = meta.modified() {
                            acc.push((modified, path));
                        }
                    }
                }
            }
        }
    }
}

fn is_numeric(s: &str) -> bool {
    s.parse::<usize>().is_ok()
}

fn looks_like_lang(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() < 2 || bytes.len() > 5 {
        return false;
    }
    let mut i = 0;
    while i < bytes.len() && i < 3 {
        if !bytes[i].is_ascii_lowercase() {
            return false;
        }
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b'-' {
        i += 1;
        while i < bytes.len() {
            if !bytes[i].is_ascii_uppercase() {
                return false;
            }
            i += 1;
        }
    }
    true
}

fn resolve_model(token: &str, default: &str) -> String {
    if is_numeric(token) {
        let idx: usize = token.parse().unwrap();
        if idx < MODEL_NAMES.len() {
            return MODEL_NAMES[idx].to_string();
        }
        return default.to_string();
    }
    token.to_string()
}

#[derive(Parser, Debug)]
#[command(name = "captioner")]
#[command(about = "Real-time speech-to-text captioner", long_about = None)]
struct Cli {
    #[arg(help = "Whisper model name or index")]
    model: Option<String>,

    #[arg(help = "Model for realtime transcription")]
    realtime_model: Option<String>,

    #[arg(help = "Language code (e.g. en, ja, pt, zh-CN)")]
    language: Option<String>,

    #[arg(short, long, help = "Launch GUI overlay")]
    gui: bool,

    #[arg(short = 'w', long, help = "Launch web UI")]
    web: bool,

    #[arg(long, help = "Disable GUI overlay")]
    no_gui: bool,

    #[arg(long, help = "Disable web UI")]
    no_web: bool,

    #[arg(long, help = "Disable global hotkeys")]
    no_hotkey: bool,

    #[arg(long, help = "Web server port", default_value_t = 0)]
    web_port: u16,

    #[arg(long, help = "Enable debug logging")]
    debug: bool,

    #[arg(long, help = "Enable test mode")]
    test: bool,

    #[arg(short = 'i', long, help = "Audio input device (name or index)")]
    input: Option<String>,

    #[arg(short = 'o', long, help = "Audio output device for monitor capture")]
    output: Option<String>,

    #[arg(
        long = "list-devices",
        short = 'l',
        help = "List audio devices and exit"
    )]
    list_devices: bool,

    #[arg(long = "no-realtime", help = "Disable realtime transcription")]
    no_realtime: bool,

    #[arg(long, help = "Silero VAD sensitivity (0.0-1.0)")]
    silero_sensitivity: Option<f32>,

    #[arg(long, help = "WebRTC VAD sensitivity (0-3)")]
    webrtc_sensitivity: Option<u32>,

    #[arg(long, help = "Minimum gap between recordings in seconds")]
    min_gap: Option<f32>,

    #[arg(long, help = "Recording scale multiplier")]
    recording_scale: Option<f32>,

    #[arg(long, help = "Show history of past N log sessions")]
    history: Option<usize>,

    #[arg(long, help = "Transcription backend: whisper-cpp, python, or stub")]
    backend: Option<String>,

    #[arg(long, help = "Compute device for Python backend (cpu, cuda, auto)")]
    python_device: Option<String>,

    #[arg(long, help = "Generate default config.toml and exit")]
    generate_config: bool,

    #[arg(long, help = "Download a whisper model (e.g. --download-model base)")]
    download_model: Option<String>,

    #[arg(long, help = "List available and downloadable models")]
    list_models: bool,

    #[arg(long, help = "Enable GPU acceleration (requires CUDA)")]
    gpu: bool,
}

pub fn parse_args() -> Config {
    let mut cli = Cli::parse();

    if cli.generate_config {
        generate_default_config();
        std::process::exit(0);
    }

    if cli.list_models {
        crate::download::print_downloadable_models();
        std::process::exit(0);
    }

    if let Some(model) = cli.download_model {
        let downloader = crate::download::ModelDownloader::new();
        info!("Downloading model '{}'...", model);
        downloader.download(&model);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let (dl, total) = downloader.progress();
            if total > 0 {
                let pct = (dl as f64 / total as f64 * 100.0) as u64;
                eprint!(
                    "\r  Downloaded {} / {} MB ({}%)    ",
                    dl / 1_048_576,
                    total / 1_048_576,
                    pct
                );
            }
            match downloader.status() {
                crate::download::DownloadStatus::Completed { .. } => {
                    println!("\nDone!");
                    std::process::exit(0);
                }
                crate::download::DownloadStatus::Failed { error, .. } => {
                    println!("\nFailed: {}", error);
                    std::process::exit(1);
                }
                crate::download::DownloadStatus::Idle => break,
                _ => {}
            }
        }
        std::process::exit(0);
    }

    if let Some(n) = cli.history {
        print_history(n);
        std::process::exit(0);
    }

    let toml_cfg = load_toml_config();
    let data_dir = ensure_data_dirs();

    let cli_model = cli.model.take();
    let default_model = cli_model
        .as_deref()
        .or(toml_cfg.default_model.as_deref())
        .or(toml_cfg.model.as_deref())
        .unwrap_or("base");

    let mut model_name = default_model.to_string();
    let mut realtime_model: Option<String> = None;
    let mut lang: Option<String> = cli.language.clone().or(toml_cfg.language.clone());
    let mut found_pos = 0usize;

    if cli_model.is_some() {
        model_name = resolve_model(cli_model.as_deref().unwrap(), default_model);
        found_pos = 1;
    } else if let Some(ref m) = toml_cfg.model {
        model_name = resolve_model(m.as_str(), default_model);
    }

    if let Some(ref rm) = cli.realtime_model {
        realtime_model = Some(resolve_model(rm, &model_name));
    } else if let Some(ref rm) = toml_cfg.realtime_model {
        realtime_model = Some(rm.clone());
    }

    if lang.is_none() {
        let all_args: Vec<String> = std::env::args().collect();
        let args = &all_args[1..];
        let mut i = 0;
        while i < args.len() {
            let a = &args[i];
            if a.starts_with('-') {
                match a.as_str() {
                    "-g" | "--gui" | "-w" | "--web" | "--no-gui" | "--no-web" | "--no-hotkey"
                    | "--debug" | "--test" | "--list-devices" | "-ld" | "-nrt"
                    | "--no-realtime" | "--generate-config" => {}
                    "-i"
                    | "--input"
                    | "-o"
                    | "--output"
                    | "--silero-sensitivity"
                    | "--webrtc-sensitivity"
                    | "--min-gap"
                    | "--recording-scale"
                    | "--web-port"
                    | "--history"
                    | "--backend"
                    | "--python-device"
                    | "--gpu" => {
                        i += 1;
                    }
                    _ => {}
                }
                i += 1;
                continue;
            }

            if looks_like_lang(a) && lang.is_none() {
                lang = Some(a.clone());
            } else if found_pos == 0 {
                model_name = resolve_model(a, default_model);
                found_pos = 1;
            } else if found_pos == 1 {
                realtime_model = Some(resolve_model(a, &model_name));
                found_pos = 2;
            }
            i += 1;
        }
    }

    if realtime_model.is_none() {
        realtime_model = Some(model_name.clone());
    }

    if let Some(ref l) = lang {
        if l != "en" && model_name.contains(".en") {
            eprintln!(
                "Warning: Model '{}' is English-only but lang={}",
                model_name, l
            );
        }
    }

    if !MODEL_NAMES.contains(&model_name.as_str()) {
        let available = scan_available_models();
        if !available.contains(&model_name) {
            eprintln!("Warning: {} is not a recognized model name", model_name);
            if !available.is_empty() {
                eprintln!("  Available downloaded models: {}", available.join(", "));
            }
        }
    }

    let gui = if cli.no_gui {
        false
    } else {
        cli.gui || toml_cfg.gui.unwrap_or(false)
    };
    let web = if cli.no_web {
        false
    } else {
        cli.web || toml_cfg.web.unwrap_or(false)
    };
    let hotkey = if cli.no_hotkey {
        false
    } else {
        toml_cfg.hotkey.unwrap_or(true)
    };

    let web_port = if cli.web_port != 0 {
        cli.web_port
    } else {
        toml_cfg.web_port.unwrap_or(5000)
    };

    let toml_backend_str = toml_cfg
        .transcription_backend
        .as_ref()
        .map(|b| b.to_string())
        .unwrap_or_default();
    let backend = cli
        .backend
        .as_deref()
        .or(if toml_backend_str.is_empty() {
            None
        } else {
            Some(toml_backend_str.as_str())
        })
        .unwrap_or("whisper-cpp");
    let transcription_backend = match backend {
        "python" => TranscriptionBackend::Python,
        "stub" => TranscriptionBackend::Stub,
        _ => TranscriptionBackend::WhisperCpp,
    };

    let path = if let Some(p) = toml_cfg.path {
        PathBuf::from(p)
    } else {
        data_dir
    };

    Config {
        model_name,
        realtime_model,
        language: lang,
        gui,
        web,
        web_port,
        hotkey,
        debug_mode: cli.debug || toml_cfg.debug.unwrap_or(false),
        test_mode: cli.test || toml_cfg.test.unwrap_or(false),
        realtime: !cli.no_realtime && toml_cfg.realtime.unwrap_or(true),
        path,
        input_device: cli.input.or(toml_cfg.input_device),
        output_device: cli.output.or(toml_cfg.output_device),
        recording_scale: cli
            .recording_scale
            .or(toml_cfg.recording_scale)
            .unwrap_or(1.25),
        silero_sensitivity: cli
            .silero_sensitivity
            .or(toml_cfg.silero_sensitivity)
            .unwrap_or(0.1),
        webrtc_sensitivity: cli
            .webrtc_sensitivity
            .or(toml_cfg.webrtc_sensitivity)
            .unwrap_or(0),
        min_gap_between_recordings: cli.min_gap.or(toml_cfg.min_gap).unwrap_or(0.4),
        list_devices: cli.list_devices,
        history: None,
        transcription_backend,
        python_device: cli.python_device.or(toml_cfg.python_device),
        use_gpu: cli.gpu || toml_cfg.use_gpu.unwrap_or(false),
    }
}

pub fn get_model_path(model_name: &str) -> PathBuf {
    let model_dir = get_data_dir().join("models");

    let filename = format!("ggml-{}.bin", model_name.replace(['/', '\\'], "-"));
    let path = model_dir.join(&filename);

    if path.exists() {
        return path;
    }

    let ggml_filename = format!("ggml-{}.ggml", model_name.replace(['/', '\\'], "-"));
    let ggml_path = model_dir.join(&ggml_filename);
    if ggml_path.exists() {
        return ggml_path;
    }

    path
}

fn generate_default_config() {
    let config_dir = get_config_dir();
    if let Err(e) = std::fs::create_dir_all(&config_dir) {
        eprintln!("Could not create config dir: {}", e);
        return;
    }

    let config_path = get_config_path();
    if config_path.exists() {
        eprintln!("Config already exists at {}", config_path.display());
        return;
    }

    let toml_str = r#"# Captioner configuration
# This file is loaded from $XDG_CONFIG_HOME/captioner/config.toml
# CLI arguments override values set here.

# Default whisper model name (e.g. "base", "small", "large-v3")
model = "base"

# Model for realtime transcription (defaults to model)
# realtime_model = "base"

# Language code (e.g. "en", "ja", "pt", "zh-CN")
# language = "en"

# Launch GUI overlay on start
gui = false

# Launch web UI on start
web = false

# Web server port
web_port = 5000

# Enable global hotkeys
hotkey = true

# Transcription backend: "whisper-cpp", "python", or "stub"
# transcription_backend = "whisper-cpp"

# Debug logging
# debug = false

# Test mode
# test = false

# Realtime transcription
# realtime = true

# Audio input device (name or index)
# input_device = ""

# Audio output device for monitor capture
# output_device = ""

# Recording scale multiplier
# recording_scale = 1.25

# Silero VAD sensitivity (0.0-1.0)
# silero_sensitivity = 0.1

# WebRTC VAD sensitivity (0=Quality, 1=LowBitrate, 2=Aggressive, 3=VeryAggressive)
# webrtc_sensitivity = 0

# Minimum gap between recordings in seconds
# min_gap = 0.4

# Compute device for Python backend (cpu, cuda, auto)
# python_device = "cpu"

# Enable GPU acceleration for whisper.cpp (requires CUDA build)
# use_gpu = false
"#;

    match std::fs::write(&config_path, toml_str) {
        Ok(_) => println!("Generated default config at {}", config_path.display()),
        Err(e) => eprintln!("Failed to write config: {}", e),
    }
}

pub fn list_audio_devices() -> anyhow::Result<Vec<AudioDevice>> {
    let host = cpal::default_host();
    let mut devices = Vec::new();

    for i in 0..host.input_devices()?.count() {
        if let Some(device) = host.input_devices()?.nth(i) {
            if let Ok(name) = device.name() {
                let channels = device
                    .default_input_config()
                    .map(|c| c.channels())
                    .unwrap_or(0);
                let sample_rate = device
                    .default_input_config()
                    .map(|c| c.sample_rate().0)
                    .unwrap_or(44100);

                devices.push(AudioDevice {
                    index: i as u32,
                    name,
                    max_input_channels: channels,
                    sample_rate,
                    is_input: true,
                    max_output_channels: 0,
                    is_output: false,
                });
            }
        }
    }

    for i in 0..host.output_devices()?.count() {
        if let Some(device) = host.output_devices()?.nth(i) {
            if let Ok(name) = device.name() {
                let channels = device
                    .default_output_config()
                    .map(|c| c.channels())
                    .unwrap_or(0);
                let sample_rate = device
                    .default_output_config()
                    .map(|c| c.sample_rate().0)
                    .unwrap_or(44100);

                devices.push(AudioDevice {
                    index: i as u32,
                    name,
                    max_output_channels: channels,
                    sample_rate,
                    is_output: true,
                    max_input_channels: 0,
                    is_input: false,
                });
            }
        }
    }

    Ok(devices)
}

#[derive(Clone, Debug)]
pub struct AudioDevice {
    pub index: u32,
    pub name: String,
    pub max_input_channels: u16,
    pub max_output_channels: u16,
    pub sample_rate: u32,
    pub is_input: bool,
    pub is_output: bool,
}
