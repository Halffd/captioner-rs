use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tracing::{error, info, warn};
use futures_util::StreamExt;

use super::config::get_data_dir;

const HF_BASE: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DownloadStatus {
    Idle,
    Downloading {
        model: String,
        downloaded: u64,
        total: u64,
    },
    Completed {
        model: String,
        path: PathBuf,
    },
    Failed {
        model: String,
        error: String,
    },
}

pub struct ModelDownloader {
    cancel: Arc<AtomicBool>,
    progress_downloaded: Arc<AtomicU64>,
    progress_total: Arc<AtomicU64>,
    status: Arc<parking_lot::Mutex<DownloadStatus>>,
}

impl Default for ModelDownloader {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelDownloader {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            progress_downloaded: Arc::new(AtomicU64::new(0)),
            progress_total: Arc::new(AtomicU64::new(0)),
            status: Arc::new(parking_lot::Mutex::new(DownloadStatus::Idle)),
        }
    }

    pub fn status(&self) -> DownloadStatus {
        self.status.lock().clone()
    }

    pub fn progress(&self) -> (u64, u64) {
        (
            self.progress_downloaded.load(Ordering::Relaxed),
            self.progress_total.load(Ordering::Relaxed),
        )
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn is_downloading(&self) -> bool {
        matches!(*self.status.lock(), DownloadStatus::Downloading { .. })
    }

    pub fn download(&self, model_name: &str) {
        if self.is_downloading() {
            warn!("Download already in progress");
            return;
        }

        let model = model_name.to_string();
        let cancel = self.cancel.clone();
        let progress_dl = self.progress_downloaded.clone();
        let progress_tot = self.progress_total.clone();
        let status = self.status.clone();

        cancel.store(false, Ordering::SeqCst);
        progress_dl.store(0, Ordering::Relaxed);
        progress_tot.store(0, Ordering::Relaxed);

        let model_clone = model.clone();
        let status_clone = status.clone();
        *status.lock() = DownloadStatus::Downloading {
            model: model.clone(),
            downloaded: 0,
            total: 0,
        };

        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();

            let result = rt.block_on(download_model_async(
                &model,
                cancel,
                progress_dl,
                progress_tot,
            ));

            match result {
                Ok(path) => {
                    info!("Model '{}' downloaded to {}", model_clone, path.display());
                    *status_clone.lock() = DownloadStatus::Completed {
                        model: model_clone,
                        path,
                    };
                }
                Err(e) => {
                    error!("Failed to download model '{}': {}", model_clone, e);
                    *status_clone.lock() = DownloadStatus::Failed {
                        model: model_clone,
                        error: e.to_string(),
                    };
                }
            }
        });
    }
}

fn model_url(model_name: &str) -> String {
    let filename = format!("ggml-{}.bin", model_name);
    format!("{}/{}", HF_BASE, filename)
}

fn model_dest_path(model_name: &str) -> PathBuf {
    let models_dir = get_data_dir().join("models");
    let safe_name = model_name.replace(['/', '\\'], "-");
    models_dir.join(format!("ggml-{}.bin", safe_name))
}

pub fn model_file_size(model_name: &str) -> Option<u64> {
    let path = model_dest_path(model_name);
    path.exists().then(|| {
        std::fs::metadata(&path).map(|m| m.len()).ok()
    }).flatten()
}

pub fn is_model_downloaded(model_name: &str) -> bool {
    let path = model_dest_path(model_name);
    path.exists()
}

pub fn downloadable_models() -> &'static [(&'static str, &'static str)] {
    &[
        ("tiny",       "75 MB"),
        ("tiny.en",    "75 MB"),
        ("base",       "142 MB"),
        ("base.en",    "142 MB"),
        ("small",      "466 MB"),
        ("small.en",   "466 MB"),
        ("medium",     "1.5 GB"),
        ("medium.en",  "1.5 GB"),
        ("large-v2",   "3.0 GB"),
        ("large-v3",   "3.0 GB"),
    ]
}

async fn download_model_async(
    model_name: &str,
    cancel: Arc<AtomicBool>,
    progress_dl: Arc<AtomicU64>,
    progress_tot: Arc<AtomicU64>,
) -> anyhow::Result<PathBuf> {
    let url = model_url(model_name);
    let dest = model_dest_path(model_name);

    if dest.exists() {
        info!("Model '{}' already exists at {}", model_name, dest.display());
        return Ok(dest);
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    info!("Downloading model '{}' from {}", model_name, url);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;

    let response = client.get(&url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("HTTP {} for {}", response.status(), url);
    }

    let total_size = response.content_length().unwrap_or(0);
    progress_tot.store(total_size, Ordering::Relaxed);

    let tmp_path = dest.with_extension("bin.tmp");
    let mut file = std::fs::File::create(&tmp_path)?;
    let mut downloaded: u64 = 0;
    let mut stream = response.bytes_stream();

    use std::io::Write;

    while let Some(chunk_result) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            drop(file);
            let _ = std::fs::remove_file(&tmp_path);
            anyhow::bail!("Download cancelled");
        }

        let chunk = chunk_result?;
        file.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        progress_dl.store(downloaded, Ordering::Relaxed);
    }

    file.flush()?;
    drop(file);

    std::fs::rename(&tmp_path, &dest)?;

    Ok(dest)
}

pub fn print_downloadable_models() {
    let downloaded = super::config::scan_available_models();
    println!("\n=== Whisper Models ===\n");
    println!(" {:<20} {:<12} Status", "Model", "Size");
    println!("  {}", "-".repeat(50));

    for &(name, size) in downloadable_models() {
        let status = if downloaded.iter().any(|d| d == name) {
            "downloaded"
        } else {
            "not downloaded"
        };
        println!("  {:<20} {:<12} {}", name, size, status);
    }

    println!("\n  Download with: captioner --download-model <name>");
    println!("  Models stored in: {}/models/\n", get_data_dir().display());
}

pub fn list_models_json() -> Vec<ModelInfo> {
    let downloaded = super::config::scan_available_models();
    downloadable_models()
        .iter()
        .map(|&(name, size)| ModelInfo {
            name: name.to_string(),
            estimated_size: size.to_string(),
            downloaded: downloaded.iter().any(|d| d == name),
        })
        .collect()
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ModelInfo {
    pub name: String,
    pub estimated_size: String,
    pub downloaded: bool,
}
