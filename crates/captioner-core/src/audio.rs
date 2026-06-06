use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{error, info, warn};

use super::error::CaptionerError;

const TARGET_SAMPLE_RATE: u32 = 16000;

pub struct AudioCapture {
    device_name: String,
    stream: Option<cpal::Stream>,
    stop_flag: Arc<AtomicBool>,
    recording_enabled: Arc<AtomicBool>,
    buffer: Arc<parking_lot::Mutex<Vec<f32>>>,
}

pub struct OutputCapture {
    device_name: String,
    stream: Option<cpal::Stream>,
    buffer: Arc<parking_lot::Mutex<Vec<f32>>>,
}

pub struct DualCapture {
    pub input: AudioCapture,
    pub output: Option<OutputCapture>,
}

impl AudioCapture {
    pub fn new(device_spec: Option<&str>) -> Result<Self, CaptionerError> {
        let host = cpal::default_host();

        let device = if let Some(spec) = device_spec {
            if let Ok(idx) = spec.parse::<usize>() {
                host.input_devices()
                    .map_err(|e| CaptionerError::AudioDevice(e.to_string()))?
                    .nth(idx)
            } else {
                host.input_devices()
                    .map_err(|e| CaptionerError::AudioDevice(e.to_string()))?
                    .find(|d| d.name().map(|n| n.contains(spec)).unwrap_or(false))
            }
        } else {
            host.default_input_device()
        };

        let device = device.ok_or_else(|| {
            CaptionerError::AudioDevice("No input device available".to_string())
        })?;

        let device_name = device
            .name()
            .map_err(|e| CaptionerError::AudioDevice(e.to_string()))?;

        Ok(Self {
            device_name,
            stream: None,
            stop_flag: Arc::new(AtomicBool::new(false)),
            recording_enabled: Arc::new(AtomicBool::new(true)),
            buffer: Arc::new(parking_lot::Mutex::new(Vec::with_capacity(16000))),
        })
    }

    pub fn start(&mut self) -> Result<(), CaptionerError> {
        let host = cpal::default_host();
        let device = host
            .input_devices()
            .map_err(|e| CaptionerError::AudioDevice(e.to_string()))?
            .find(|d| d.name().map(|n| n == self.device_name).unwrap_or(false))
            .ok_or_else(|| {
                CaptionerError::AudioDevice(format!("Device '{}' not found", self.device_name))
            })?;

        self.start_on_device(&device)
    }

    fn start_on_device(&mut self, device: &cpal::Device) -> Result<(), CaptionerError> {
        let config = device
            .default_input_config()
            .map_err(|e| CaptionerError::AudioCapture(e.to_string()))?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();
        info!(
            "Audio device: {}, sample_rate: {}, channels: {}",
            self.device_name, sample_rate, channels
        );

        let buffer = self.buffer.clone();
        let recording = self.recording_enabled.clone();
        let resample_needed = sample_rate != TARGET_SAMPLE_RATE;

        let err_fn = |err: cpal::StreamError| {
            error!("Audio stream error: {}", err);
        };

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    if !recording.load(Ordering::SeqCst) {
                        return;
                    }
                    let mono = mono_f32(data, channels);
                    let resampled = if resample_needed {
                        linear_resample(&mono, sample_rate, TARGET_SAMPLE_RATE)
                    } else {
                        mono
                    };
                    buffer.lock().extend_from_slice(&resampled);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    if !recording.load(Ordering::SeqCst) {
                        return;
                    }
                    let mono = mono_i16_to_f32(data, channels);
                    let resampled = if resample_needed {
                        linear_resample(&mono, sample_rate, TARGET_SAMPLE_RATE)
                    } else {
                        mono
                    };
                    buffer.lock().extend_from_slice(&resampled);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config.into(),
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    if !recording.load(Ordering::SeqCst) {
                        return;
                    }
                    let mono = mono_u16_to_f32(data, channels);
                    let resampled = if resample_needed {
                        linear_resample(&mono, sample_rate, TARGET_SAMPLE_RATE)
                    } else {
                        mono
                    };
                    buffer.lock().extend_from_slice(&resampled);
                },
                err_fn,
                None,
            ),
            format => {
                return Err(CaptionerError::AudioCapture(format!(
                    "Unsupported sample format: {:?}",
                    format
                )));
            }
        }
        .map_err(|e| CaptionerError::AudioCapture(e.to_string()))?;

        stream.play().map_err(|e| CaptionerError::AudioCapture(e.to_string()))?;
        self.stream = Some(stream);

        info!(
            "Audio capture started: {} ({}Hz → {}Hz, {}ch → mono)",
            self.device_name, sample_rate, TARGET_SAMPLE_RATE, channels
        );

        Ok(())
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        self.stream.take();
    }

    pub fn set_recording(&self, enabled: bool) {
        self.recording_enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn recording_flag(&self) -> Arc<AtomicBool> {
        self.recording_enabled.clone()
    }

    pub fn toggle_recording(&self) -> bool {
        let current = self.recording_enabled.load(Ordering::SeqCst);
        self.recording_enabled.store(!current, Ordering::SeqCst);
        !current
    }

    pub fn is_recording(&self) -> bool {
        self.recording_enabled.load(Ordering::SeqCst)
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn pull_samples(&self, max_samples: usize) -> Vec<f32> {
        let mut buf = self.buffer.lock();
        let drain_count = buf.len().min(max_samples);
        buf.drain(0..drain_count).collect()
    }

    pub fn has_samples(&self) -> bool {
        !self.buffer.lock().is_empty()
    }

    pub fn clear_buffer(&self) {
        self.buffer.lock().clear();
    }

    pub fn sample_rate(&self) -> u32 {
        TARGET_SAMPLE_RATE
    }

    pub fn buffer_ptr(&self) -> Arc<parking_lot::Mutex<Vec<f32>>> {
        self.buffer.clone()
    }
}

impl OutputCapture {
    pub fn new(device_spec: &str) -> Result<Self, CaptionerError> {
        let host = cpal::default_host();

        let device = if let Ok(idx) = device_spec.parse::<usize>() {
            host.output_devices()
                .map_err(|e| CaptionerError::AudioDevice(e.to_string()))?
                .nth(idx)
        } else {
            host.output_devices()
                .map_err(|e| CaptionerError::AudioDevice(e.to_string()))?
                .find(|d| d.name().map(|n| n.contains(device_spec)).unwrap_or(false))
        };

        let device = device.ok_or_else(|| {
            CaptionerError::AudioDevice(format!("Output device '{}' not found", device_spec))
        })?;

        let device_name = device
            .name()
            .map_err(|e| CaptionerError::AudioDevice(e.to_string()))?;

        Ok(Self {
            device_name,
            stream: None,
            buffer: Arc::new(parking_lot::Mutex::new(Vec::with_capacity(16000))),
        })
    }

    pub fn start(&mut self) -> Result<(), CaptionerError> {
        let host = cpal::default_host();
        let device = host
            .output_devices()
            .map_err(|e| CaptionerError::AudioDevice(e.to_string()))?
            .find(|d| d.name().map(|n| n == self.device_name).unwrap_or(false))
            .ok_or_else(|| {
                CaptionerError::AudioDevice(format!("Output device '{}' not found", self.device_name))
            })?;

        let config = device
            .default_output_config()
            .map_err(|e| CaptionerError::AudioCapture(e.to_string()))?;

        let sample_rate = config.sample_rate().0;
        let channels = config.channels();

        let buffer = self.buffer.clone();
        let resample_needed = sample_rate != TARGET_SAMPLE_RATE;

        let err_fn = |err: cpal::StreamError| {
            error!("Output capture stream error: {}", err);
        };

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let mono = mono_f32(data, channels);
                    let resampled = if resample_needed {
                        linear_resample(&mono, sample_rate, TARGET_SAMPLE_RATE)
                    } else {
                        mono
                    };
                    buffer.lock().extend_from_slice(&resampled);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                &config.into(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mono = mono_i16_to_f32(data, channels);
                    let resampled = if resample_needed {
                        linear_resample(&mono, sample_rate, TARGET_SAMPLE_RATE)
                    } else {
                        mono
                    };
                    buffer.lock().extend_from_slice(&resampled);
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_input_stream(
                &config.into(),
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let mono = mono_u16_to_f32(data, channels);
                    let resampled = if resample_needed {
                        linear_resample(&mono, sample_rate, TARGET_SAMPLE_RATE)
                    } else {
                        mono
                    };
                    buffer.lock().extend_from_slice(&resampled);
                },
                err_fn,
                None,
            ),
            format => {
                return Err(CaptionerError::AudioCapture(format!(
                    "Unsupported sample format: {:?}",
                    format
                )));
            }
        };

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                warn!("Cannot open output device '{}' as input (loopback not supported): {}", self.device_name, e);
                warn!("Monitor capture requires PulseAudio loopback or ALSA snd-aloop module");
                return Err(CaptionerError::AudioCapture(format!(
                    "Cannot capture from output device '{}': {}. Install PulseAudio loopback or ALSA snd-aloop.",
                    self.device_name, e
                )));
            }
        };

        stream.play().map_err(|e| CaptionerError::AudioCapture(e.to_string()))?;
        self.stream = Some(stream);

        info!(
            "Output capture started: {} ({}Hz → {}Hz, {}ch → mono)",
            self.device_name, sample_rate, TARGET_SAMPLE_RATE, channels
        );

        Ok(())
    }

    pub fn stop(&mut self) {
        self.stream.take();
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn has_samples(&self) -> bool {
        !self.buffer.lock().is_empty()
    }

    pub fn pull_samples(&self, max_samples: usize) -> Vec<f32> {
        let mut buf = self.buffer.lock();
        let drain_count = buf.len().min(max_samples);
        buf.drain(0..drain_count).collect()
    }

    pub fn buffer_ptr(&self) -> Arc<parking_lot::Mutex<Vec<f32>>> {
        self.buffer.clone()
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Drop for OutputCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

fn mono_f32(data: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return data.to_vec();
    }
    data.chunks(channels as usize)
        .map(|c| c.iter().sum::<f32>() / channels as f32)
        .collect()
}

fn mono_i16_to_f32(data: &[i16], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return data.iter().map(|&s| s as f32 / 32768.0).collect();
    }
    data.chunks(channels as usize)
        .map(|c| {
            c.iter().map(|&s| s as f32 / 32768.0).sum::<f32>() / channels as f32
        })
        .collect()
}

fn mono_u16_to_f32(data: &[u16], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return data.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).collect();
    }
    data.chunks(channels as usize)
        .map(|c| {
            c.iter().map(|&s| (s as f32 - 32768.0) / 32768.0).sum::<f32>()
                / channels as f32
        })
        .collect()
}

fn linear_resample(audio: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || audio.len() < 2 {
        return audio.to_vec();
    }

    let ratio = to_rate as f64 / from_rate as f64;
    let output_len = ((audio.len() - 1) as f64 * ratio).ceil() as usize;
    let mut output = Vec::with_capacity(output_len);

    for i in 0..output_len {
        let src_pos = i as f64 / ratio;
        let src_idx = src_pos as usize;
        let frac = src_pos - src_idx as f64;

        if src_idx + 1 < audio.len() {
            let s0 = audio[src_idx] as f64;
            let s1 = audio[src_idx + 1] as f64;
            output.push((s0 + (s1 - s0) * frac) as f32);
        } else {
            output.push(audio[audio.len() - 1]);
        }
    }

    output
}

pub fn print_audio_devices() {
    let host = cpal::default_host();
    println!("\n=== Available Audio Input Devices ===");
    if let Ok(devices) = host.input_devices() {
        for (i, device) in devices.enumerate() {
            if let Ok(name) = device.name() {
                let channels = device.default_input_config().map(|c| c.channels()).unwrap_or(0);
                let rate = device.default_input_config().map(|c| c.sample_rate().0).unwrap_or(0);
                println!(" [{}] {} - {} ch, {} Hz", i, name, channels, rate);
            }
        }
    }
    println!("\n=== Available Audio Output Devices (for monitoring) ===");
    if let Ok(devices) = host.output_devices() {
        for (i, device) in devices.enumerate() {
            if let Ok(name) = device.name() {
                let channels = device.default_output_config().map(|c| c.channels()).unwrap_or(0);
                let rate = device.default_output_config().map(|c| c.sample_rate().0).unwrap_or(0);
                println!(" [{}] {} - {} ch, {} Hz", i, name, channels, rate);
            }
        }
    }
    println!("\n=== Hints ===");
    println!(" -i, --input <name or index>   Select input device (microphone)");
    println!(" -o, --output <name or index>  Select output device for monitor capture (requires loopback)");
}
