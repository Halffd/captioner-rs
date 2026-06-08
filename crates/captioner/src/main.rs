use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tokio::runtime::Builder;
use tokio::sync::{broadcast, mpsc};
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use captioner_core::{
    audio::{AudioCapture, OutputCapture},
    config::parse_args,
    stt::SttEngine,
    download::ModelDownloader,
    AppCommand, TranscriptionBackend, TranscriptionResult,
};
use captioner_input::HotkeyListener;
use captioner_gui::CaptionerGUI;
use captioner_log::{SharedLog, LogArgs};
use captioner_python::PythonSttEngine;
use captioner_web::{WebConfig, start_server};

fn main() -> anyhow::Result<()> {
    let config = parse_args();

    let subscriber = FmtSubscriber::builder()
        .with_max_level(if config.debug_mode {
            Level::DEBUG
        } else {
            Level::INFO
        })
        .with_target(false)
        .with_thread_ids(false)
        .with_file(false)
        .with_line_number(false)
        .compact()
        .finish();

    tracing::subscriber::set_global_default(subscriber)?;

    info!("Captioner starting...");
    info!("Config: model={}, language={:?}, gui={}, web={}, web_port={}, hotkey={}, backend={}, input={:?}, output={:?}",
        config.model_name, config.language, config.gui, config.web, config.web_port, config.hotkey,
        config.transcription_backend, config.input_device, config.output_device);

    if config.input_device.is_none() && config.output_device.is_none() {
        captioner_core::audio::print_audio_devices();
    }

    if config.list_devices {
        return Ok(());
    }

    let available_models = captioner_core::config::scan_available_models();
    if !available_models.is_empty() {
        info!("Available models: {}", available_models.join(", "));
    }

    let stop_flag = Arc::new(AtomicBool::new(false));
    let (command_tx, _) = broadcast::channel::<AppCommand>(16);
    let (transcription_tx, _) = broadcast::channel::<TranscriptionResult>(100);
    let (audio_tx, audio_rx) = mpsc::channel::<Vec<f32>>(64);
    let (web_event_tx, _) = broadcast::channel::<String>(100);

    let log_args = LogArgs::from_config(&config);
    let shared_log = Arc::new(SharedLog::new(
        &config.path,
        "Logs",
        config.language.as_deref(),
        &config.model_name,
        if config.test_mode { Some("test") } else { None },
        &log_args,
    ));
    info!("Log directory: {}/Logs", config.path.display());

    let audio_recording_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(true));
    let model_downloader = Arc::new(ModelDownloader::new());

    spawn_audio_capture(
        &config,
        &stop_flag,
        &audio_tx,
        &audio_recording_flag,
    );

    let hotkey_listener = if config.hotkey {
        let listener = HotkeyListener::new();
        let rx = listener.receiver();
        listener.run();
        info!("Global hotkey listener started");
        Some((listener, rx))
    } else {
        info!("Global hotkeys disabled");
        None
    };

    spawn_web_server(
        &config,
        &stop_flag,
        &transcription_tx,
        &command_tx,
        &web_event_tx,
        &model_downloader,
    );

    spawn_log_writer(
        &stop_flag,
        &transcription_tx,
        &shared_log,
    );

    spawn_command_handler(
        &config,
        &stop_flag,
        &command_tx,
        &transcription_tx,
        &audio_recording_flag,
        &shared_log,
        &web_event_tx,
        &model_downloader,
        audio_rx,
    );

    if config.gui {
        let gui_stop = stop_flag.clone();
        let gui_command_tx = command_tx.clone();
        let language = config.language.clone();
        let gui_transcription_rx = transcription_tx.subscribe();

        let hotkey_rx = hotkey_listener.as_ref()
            .map(|(_, rx)| rx.clone())
            .unwrap_or_else(|| crossbeam_channel::bounded(1).1);

    if let Err(e) = CaptionerGUI::run(
        gui_stop,
        gui_command_tx,
        gui_transcription_rx,
        hotkey_rx,
        language,
        model_downloader.clone(),
    ) {
            error!("GUI error: {}", e);
        }
    } else {
        while !stop_flag.load(Ordering::SeqCst) {
            thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    stop_flag.store(true, Ordering::SeqCst);

    if let Some((listener, _)) = hotkey_listener {
        listener.stop();
    }

    info!("Captioner shutdown complete");
    Ok(())
}

fn spawn_audio_capture(
    config: &captioner_core::Config,
    stop_flag: &Arc<AtomicBool>,
    audio_tx: &mpsc::Sender<Vec<f32>>,
    audio_recording_flag: &Arc<AtomicBool>,
) {
    match config.transcription_backend {
        TranscriptionBackend::WhisperCpp => {
            let audio_stop = stop_flag.clone();
            let audio_tx_clone = audio_tx.clone();
            let audio_input = config.input_device.clone();
            let audio_output = config.output_device.clone();
            let rec_flag = audio_recording_flag.clone();

            thread::spawn(move || {
                let mut capture = match AudioCapture::new(audio_input.as_deref()) {
                    Ok(c) => c,
                    Err(e) => {
                        error!("Failed to open audio device: {}", e);
                        return;
                    }
                };

                if let Err(e) = capture.start() {
                    error!("Failed to start audio capture: {}", e);
                    return;
                }

                info!("Audio capture started on device: {}", capture.device_name());

                let mut output_capture: Option<OutputCapture> = None;
                if let Some(ref output_spec) = audio_output {
                    match OutputCapture::new(output_spec) {
                        Ok(mut oc) => {
                            match oc.start() {
                                Ok(()) => {
                                    info!("Output monitor capture started on device: {}", oc.device_name());
                                    output_capture = Some(oc);
                                }
                                Err(e) => {
                                    error!("Failed to start output capture: {}", e);
                                    info!("Continuing with microphone input only");
                                }
                            }
                        }
                        Err(e) => {
                            error!("Failed to open output device: {}", e);
                            info!("Continuing with microphone input only");
                        }
                    }
                }

                while !audio_stop.load(Ordering::SeqCst) {
                    thread::sleep(std::time::Duration::from_millis(10));

                    let recording = rec_flag.load(Ordering::SeqCst);
                    if !recording {
                        continue;
                    }

                    if capture.has_samples() {
                        let mut samples = capture.pull_samples(1600);
                        if let Some(ref mut oc) = output_capture {
                            if oc.has_samples() {
                                let out_samples = oc.pull_samples(1600);
                                merge_audio(&mut samples, &out_samples);
                            }
                        }
                        if !samples.is_empty() && audio_tx_clone.blocking_send(samples).is_err() {
                            break;
                        }
                    } else if let Some(ref mut oc) = output_capture {
                        if oc.has_samples() {
                            let out_samples = oc.pull_samples(1600);
                            if !out_samples.is_empty() && audio_tx_clone.blocking_send(out_samples).is_err() {
                                break;
                            }
                        }
                    }
                }

                if let Some(mut oc) = output_capture {
                    oc.stop();
                }
                capture.stop();
                info!("Audio capture thread stopped");
            });
        }
        TranscriptionBackend::Python | TranscriptionBackend::Stub => {}
    }
}

fn spawn_web_server(
    config: &captioner_core::Config,
    _stop_flag: &Arc<AtomicBool>,
    transcription_tx: &broadcast::Sender<TranscriptionResult>,
    command_tx: &broadcast::Sender<AppCommand>,
    web_event_tx: &broadcast::Sender<String>,
    model_downloader: &Arc<ModelDownloader>,
) {
    if !config.web {
        return;
    }

    let web_transcript = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let web_tx = web_event_tx.clone();
    let mut web_transcription_rx = transcription_tx.subscribe();
    let web_command_tx = command_tx.clone();
    let web_recording = Arc::new(parking_lot::Mutex::new(true));
    let web_model = Arc::new(parking_lot::Mutex::new(config.model_name.clone()));
    let web_lang = Arc::new(parking_lot::Mutex::new(config.language.clone()));
    let web_dl = model_downloader.clone();
    let web_port = config.web_port;

    std::thread::spawn(move || {
        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let transcript_clone = web_transcript.clone();
        let web_tx_clone = web_tx.clone();

        rt.spawn(async move {
            loop {
                match web_transcription_rx.recv().await {
                    Ok(result) => {
                        transcript_clone.lock().push(result.text.clone());
                        let _ = web_tx_clone.send(result.text);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

    let cfg = WebConfig {
        port: web_port,
        transcript: web_transcript,
        event_tx: web_tx,
        command_tx: web_command_tx,
        recording: web_recording,
        current_model: web_model,
        current_language: web_lang,
        model_downloader: web_dl,
    };

        rt.block_on(start_server(cfg));
    });
}

fn spawn_log_writer(
    stop_flag: &Arc<AtomicBool>,
    transcription_tx: &broadcast::Sender<TranscriptionResult>,
    shared_log: &Arc<SharedLog>,
) {
    let log_transcription_rx = transcription_tx.subscribe();
    let log_stop = stop_flag.clone();
    let shared_log_ref = shared_log.clone();
    std::thread::spawn(move || {
        let rt = Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        let mut rx = log_transcription_rx;
        let log_ref = shared_log_ref.clone();
        rt.block_on(async move {
            loop {
                match rx.recv().await {
                    Ok(result) => {
                        log_ref.write_log(&result.text);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
                if log_stop.load(Ordering::SeqCst) {
                    break;
                }
            }
        });
        shared_log_ref.close();
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn_command_handler(
    config: &captioner_core::Config,
    stop_flag: &Arc<AtomicBool>,
    command_tx: &broadcast::Sender<AppCommand>,
    transcription_tx: &broadcast::Sender<TranscriptionResult>,
    audio_recording_flag: &Arc<AtomicBool>,
    shared_log: &Arc<SharedLog>,
    web_event_tx: &broadcast::Sender<String>,
    model_downloader: &Arc<ModelDownloader>,
    audio_rx: mpsc::Receiver<Vec<f32>>,
) {
    match config.transcription_backend {
        TranscriptionBackend::WhisperCpp => {
            let rt = Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            let stt_config = config.clone();
            let stt_engine = SttEngine::new(
                stt_config,
                audio_rx,
                transcription_tx.clone(),
            );
            let stt_handle = stt_engine.handle();
            let mut command_rx = command_tx.subscribe();
            let sf = stop_flag.clone();
            let arf = audio_recording_flag.clone();
        let sl = shared_log.clone();
        let wetx = web_event_tx.clone();
        let dl = model_downloader.clone();

        let engine_stopped = Arc::new(AtomicBool::new(false));

        std::thread::spawn(move || {
            rt.block_on(async move {
                let handle = stt_handle;
                let es = engine_stopped.clone();
                let es2 = engine_stopped.clone();

                tokio::task::spawn_blocking(move || {
                    let mut engine = stt_engine;
                    let rt2 = Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    rt2.block_on(engine.run());
                    es2.store(true, Ordering::SeqCst);
                });

                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    while let Ok(cmd) = command_rx.try_recv() {
                        match cmd {
                            AppCommand::Quit => {
                                sf.store(true, Ordering::SeqCst);
                                handle.stop();
                                return;
                            }
                            AppCommand::ToggleRecording => {
                                let now_recording = handle.toggle_recording();
                                arf.store(now_recording, Ordering::SeqCst);
                                info!("Recording: {}", now_recording);
                            }
                            AppCommand::Clear => {
                                sl.write_log("-- Clear --");
                                let _ = wetx.send("CLEAR".to_string());
                            }
                            AppCommand::SetModel(m) => {
                                match handle.set_model(m.clone()) {
                                    Ok(()) => info!("Model switch requested: {}", m),
                                    Err(e) => error!("Failed to send model switch request: {}", e),
                                }
                            }
                            AppCommand::SetLanguage(l) => {
                                match handle.set_language(l.clone()) {
                                    Ok(()) => info!("Language change requested: {:?}", l),
                                    Err(e) => error!("Failed to send language change request: {}", e),
                                }
                            }
                AppCommand::SetRecordingScale(s) => {
                    info!("Recording scale change requested: {} (not yet implemented)", s);
                }
                AppCommand::DownloadModel(m) => {
                    info!("Model download requested: {}", m);
                    dl.download(&m);
                }
                _ => {}
                        }
                    }
                    if sf.load(Ordering::SeqCst) || es.load(Ordering::SeqCst) {
                        return;
                    }
                }
            });
        });
        }
        TranscriptionBackend::Python => {
            let missing = captioner_python::check_python_deps();
            if !missing.is_empty() {
                error!("Missing Python dependencies: {}", missing.join(", "));
                error!("Install with: pip install faster-whisper webrtcvad torch torchaudio pyaudio numpy");
                stop_flag.store(true, Ordering::SeqCst);
                return;
            }
            info!("Python dependencies OK");

            let rt = Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

        let mut py_engine = PythonSttEngine::new(config.clone(), transcription_tx.clone());
        let py_stop = stop_flag.clone();
        let py_recording = audio_recording_flag.clone();
        let py_shared_log = shared_log.clone();
        let py_command_tx = command_tx.clone();
        let py_web_event_tx = web_event_tx.clone();
        let py_dl = model_downloader.clone();

        std::thread::spawn(move || {
            let mut cmd_rx = py_command_tx.subscribe();
                let s = py_stop.clone();
                let r = py_recording.clone();
                let log = py_shared_log.clone();
                let wtx = py_web_event_tx.clone();

                rt.block_on(async move {
                    tokio::spawn(async move {
                        loop {
                            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                            while let Ok(cmd) = cmd_rx.try_recv() {
                                match cmd {
                                    AppCommand::Quit => {
                                        s.store(true, Ordering::SeqCst);
                                        return;
                                    }
                                    AppCommand::ToggleRecording => {
                                        let now = r.load(Ordering::SeqCst);
                                        r.store(!now, Ordering::SeqCst);
                                        info!("Recording: {}", !now);
                                    }
                                    AppCommand::Clear => {
                                        log.write_log("-- Clear --");
                                        let _ = wtx.send("CLEAR".to_string());
                                    }
                                    AppCommand::SetModel(m) => {
                                        info!("Model change requested: {} (restart required for Python backend)", m);
                                    }
                AppCommand::SetLanguage(l) => {
                    info!("Language change requested: {:?} (restart required for Python backend)", l);
                }
                AppCommand::DownloadModel(m) => {
                    info!("Model download requested: {}", m);
                    py_dl.download(&m);
                }
                _ => {}
            }
        }
        if s.load(Ordering::SeqCst) {
            return;
        }
    }
});

                    py_engine.run().await;
                });
            });
        }
        TranscriptionBackend::Stub => {
            let rt = Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

        let sf = stop_flag.clone();
        let arf = audio_recording_flag.clone();
        let sl = shared_log.clone();
        let wetx = web_event_tx.clone();
        let dl = model_downloader.clone();
        let stub_command_tx = command_tx.clone();

        std::thread::spawn(move || {
            let mut cmd_rx = stub_command_tx.subscribe();
                rt.block_on(async move {
                    info!("Running in stub mode - no real transcription");
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        if sf.load(Ordering::SeqCst) {
                            break;
                        }
                        while let Ok(cmd) = cmd_rx.try_recv() {
                            match cmd {
                                AppCommand::Quit => {
                                    sf.store(true, Ordering::SeqCst);
                                    return;
                                }
                                AppCommand::ToggleRecording => {
                                    let now = arf.load(Ordering::SeqCst);
                                    arf.store(!now, Ordering::SeqCst);
                                    info!("Recording: {}", !now);
                                }
                AppCommand::Clear => {
                    sl.write_log("-- Clear --");
                    let _ = wetx.send("CLEAR".to_string());
                }
                AppCommand::DownloadModel(m) => {
                    info!("Model download requested: {}", m);
                    dl.download(&m);
                }
                _ => {}
                            }
                        }
                    }
                });
            });
        }
    }
}

fn merge_audio(primary: &mut [f32], secondary: &[f32]) {
    let len = primary.len().min(secondary.len());
    for i in 0..len {
        primary[i] = (primary[i] + secondary[i]) * 0.5;
    }
}
