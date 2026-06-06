use chrono::{NaiveDate, DateTime, Local};
use parking_lot::Mutex;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Log {
    file: Option<File>,
    test_file: Option<File>,
    path: PathBuf,
    log_dir: String,
    filename: String,
    args_str: String,
    current_date: Option<NaiveDate>,
}

impl Log {
    pub fn new(
        path: &Path,
        log_dir: &str,
        lang: Option<&str>,
        model_name: &str,
        test_name: Option<&str>,
        args: &LogArgs,
    ) -> Self {
        let lang_part = match lang {
            Some(l) => format!("-{}-", l),
            None => "-".to_string(),
        };
        let filename = format!("speech{}{}", lang_part, model_name);

        let args_str = args.to_string();

        let mut log = Self {
            file: None,
            test_file: None,
            path: path.to_path_buf(),
            log_dir: log_dir.to_string(),
            filename,
            args_str,
            current_date: None,
        };

        log.create_log_file(test_name);
        log
    }

    fn get_log_dir_path(&self, now: &DateTime<Local>) -> PathBuf {
        self.path
            .join(&self.log_dir)
            .join(now.format("%Y").to_string())
            .join(now.format("%m").to_string())
            .join(now.format("%d").to_string())
    }

    fn create_log_dir(&self) -> Option<PathBuf> {
        let log_dir_path = self.path.join(&self.log_dir);
        match fs::create_dir_all(&log_dir_path) {
            Ok(_) => Some(log_dir_path),
            Err(e) => {
                eprintln!("Could not create log directory: {}", e);
                None
            }
        }
    }

    fn create_log_file(&mut self, test_name: Option<&str>) {
        let now = Local::now();
        let today = now.date_naive();
        let formatted_date = now.format("%d-%m-%Y").to_string();
        let formatted_time = now.format("%H:%M:%S").to_string();
        let weekday = now.format("%A").to_string();

        let log_dir = self.get_log_dir_path(&now);
        if let Err(e) = fs::create_dir_all(&log_dir) {
            eprintln!("Could not create nested log directory: {}", e);
            if let Some(fallback) = self.create_log_dir() {
                if let Some(ref mut f) = self.file {
                    let _ = f.flush();
                }
                self.file = None;
                let time_part = now.format("%d-%H-%M-%S");
                let file_path = fallback.join(format!("{}_{}.log", time_part, self.filename));
                if let Ok(f) = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&file_path)
                {
                    self.file = Some(f);
                }
            }
            return;
        }

        let time_part = now.format("%d-%H-%M-%S");
        let file_path = log_dir.join(format!("{}_{}.log", time_part, self.filename));

        if let Some(ref mut f) = self.file {
            let _ = f.flush();
        }
        self.file = None;
        self.test_file = None;

        if let Ok(metadata) = file_path.metadata() {
            if metadata.len() > 0 {
                if let Ok(f) = OpenOptions::new().append(true).open(&file_path) {
                    self.file = Some(f);
                    if let Some(ref mut file) = self.file {
                        let _ = file.write_all(format!("\n{}\n", formatted_time).as_bytes());
                    }
                }
            } else {
                self.open_new_log_file(&file_path, &weekday, &formatted_date, &formatted_time);
            }
        } else {
            self.open_new_log_file(&file_path, &weekday, &formatted_date, &formatted_time);
        }

        if let Some(name) = test_name {
            let test_path = log_dir.join(format!("{}_{}.txt", time_part, name));
            if let Ok(f) = OpenOptions::new()
                .create(true)
                .truncate(true)
                .open(&test_path)
            {
                self.test_file = Some(f);
                if let Some(ref mut file) = self.test_file {
                    let _ = file.write_all(
                        format!("{} {}\n{}\n", formatted_date, formatted_time, self.args_str).as_bytes(),
                    );
                }
            }
        }

        self.current_date = Some(today);
    }

    fn open_new_log_file(
        &mut self,
        file_path: &Path,
        weekday: &str,
        formatted_date: &str,
        formatted_time: &str,
    ) {
        match OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)
        {
            Ok(f) => {
                self.file = Some(f);
                if let Some(ref mut file) = self.file {
                    let _ = file.write_all(
                        format!("{} {} {}\n", weekday, formatted_date, formatted_time).as_bytes(),
                    );
                    let _ = file.write_all(format!("Args: {}\n", self.args_str).as_bytes());
                }
            }
            Err(e) => {
                eprintln!("Could not create log file: {}", e);
            }
        }
    }

    pub fn write_log(&mut self, message: &str) {
        self.write_log_to(message, LogTarget::Main);
    }

    pub fn write_log_to(&mut self, message: &str, target: LogTarget) {
        let now = Local::now();
        let today = now.date_naive();

        if today != self.current_date.unwrap_or(today) {
            self.close();
            self.create_log_file(None);
        }

        let file: &mut Option<File> = match target {
            LogTarget::Main => &mut self.file,
            LogTarget::Test => &mut self.test_file,
        };

        if file.is_none() {
            self.create_log_file(None);
        }

        let file: &mut Option<File> = match target {
            LogTarget::Main => &mut self.file,
            LogTarget::Test => &mut self.test_file,
        };

        if let Some(ref mut f) = file {
            let current_time = now.format("%H:%M:%S").to_string();
            if let Err(e) = f.write_all(format!("{} {}\n", current_time, message).as_bytes()) {
                eprintln!("Error writing to log file: {}", e);
                self.create_log_file(None);
            } else {
                let _ = f.flush();
            }
        }
    }

    pub fn write_log_test(&mut self, message: &str) {
        self.write_log_to(message, LogTarget::Test);
    }

    pub fn close(&mut self) {
        if let Some(ref mut f) = self.file {
            let _ = f.flush();
        }
        self.file = None;

        if let Some(ref mut f) = self.test_file {
            let _ = f.flush();
        }
        self.test_file = None;
    }
}

impl Drop for Log {
    fn drop(&mut self) {
        self.close();
    }
}

pub enum LogTarget {
    Main,
    Test,
}

pub struct LogArgs {
    pub model_name: String,
    pub realtime_model: Option<String>,
    pub lang: Option<String>,
    pub gui: bool,
    pub web: bool,
    pub web_port: u16,
    pub hotkey: bool,
    pub debug_mode: bool,
    pub test_mode: bool,
    pub realtime: bool,
    pub path: String,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub transcription_backend: String,
}

impl fmt::Display for LogArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{{'model_name': '{}', 'realtime_model': {}, 'lang': {}, 'gui': {}, 'web': {}, 'web_port': {}, 'hotkey': {}, 'debug_mode': {}, 'test_mode': {}, 'realtime': {}, 'path': '{}', 'input_device': {}, 'output_device': {}, 'transcription_backend': '{}'}}",
            self.model_name,
            self.realtime_model.as_ref().map(|v| format!("'{}'", v)).unwrap_or("None".to_string()),
            self.lang.as_ref().map(|v| format!("'{}'", v)).unwrap_or("None".to_string()),
            self.gui,
            self.web,
            self.web_port,
            self.hotkey,
            self.debug_mode,
            self.test_mode,
            self.realtime,
            self.path,
            self.input_device.as_ref().map(|v| format!("'{}'", v)).unwrap_or("None".to_string()),
            self.output_device.as_ref().map(|v| format!("'{}'", v)).unwrap_or("None".to_string()),
            self.transcription_backend,
        )
    }
}

impl LogArgs {
    pub fn from_config(config: &captioner_core::Config) -> Self {
        Self {
            model_name: config.model_name.clone(),
            realtime_model: config.realtime_model.clone(),
            lang: config.language.clone(),
            gui: config.gui,
            web: config.web,
            web_port: config.web_port,
            hotkey: config.hotkey,
            debug_mode: config.debug_mode,
            test_mode: config.test_mode,
            realtime: config.realtime,
            path: config.path.display().to_string(),
            input_device: config.input_device.clone(),
            output_device: config.output_device.clone(),
            transcription_backend: config.transcription_backend.to_string(),
        }
    }
}

pub struct SharedLog {
    inner: Mutex<Log>,
}

impl SharedLog {
    pub fn new(
        path: &Path,
        log_dir: &str,
        lang: Option<&str>,
        model_name: &str,
        test_name: Option<&str>,
        args: &LogArgs,
    ) -> Self {
        Self {
            inner: Mutex::new(Log::new(path, log_dir, lang, model_name, test_name, args)),
        }
    }

    pub fn write_log(&self, message: &str) {
        self.inner.lock().write_log(message);
    }

    pub fn write_log_test(&self, message: &str) {
        self.inner.lock().write_log_test(message);
    }

    pub fn close(&self) {
        self.inner.lock().close();
    }
}
