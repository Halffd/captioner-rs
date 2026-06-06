use crossbeam_channel::Receiver;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tracing::{debug, error};

use rdev::{listen, Event, EventType, Key};
pub use captioner_core::HotkeyEvent;

pub struct HotkeyListener {
    stop_flag: Arc<AtomicBool>,
    hotkey_tx: Arc<crossbeam_channel::Sender<HotkeyEvent>>,
    hotkey_rx: Receiver<HotkeyEvent>,
}

impl HotkeyListener {
    pub fn new() -> Self {
        let (hotkey_tx, hotkey_rx) = crossbeam_channel::unbounded();

        Self {
            stop_flag: Arc::new(AtomicBool::new(false)),
            hotkey_tx: Arc::new(hotkey_tx),
            hotkey_rx,
        }
    }

    pub fn receiver(&self) -> Receiver<HotkeyEvent> {
        self.hotkey_rx.clone()
    }

    pub fn stop(&self) {
        self.stop_flag.store(true, Ordering::SeqCst);
    }

    pub fn run(&self) {
        let tx = self.hotkey_tx.clone();

    std::thread::spawn(move || {
        let mut modifiers = Modifiers::default();

            let callback = move |event: Event| {
                match event.event_type {
                    EventType::KeyPress(key) => {
                        match key {
                            Key::ControlLeft | Key::ControlRight => {
                                modifiers.ctrl = true;
                            }
                            Key::ShiftLeft | Key::ShiftRight => {
                                modifiers.shift = true;
                            }
                            Key::Alt => {
                                modifiers.alt = true;
                            }
                            Key::Return => {
                                if modifiers.ctrl {
                                    let _ = tx.send(HotkeyEvent::ToggleRecording);
                                    debug!("Hotkey: ToggleRecording (Ctrl+Return)");
                                }
                                modifiers.reset();
                            }
                            Key::Backspace => {
                                if modifiers.ctrl && modifiers.shift {
                                    let _ = tx.send(HotkeyEvent::ShowSettings);
                                    debug!("Hotkey: ShowSettings (Ctrl+Shift+Backspace)");
                                }
                                modifiers.reset();
                            }
                            Key::Equal => {
                                if modifiers.ctrl && modifiers.shift {
                                    let _ = tx.send(HotkeyEvent::IncreaseFontSize);
                                    debug!("Hotkey: IncreaseFontSize (Ctrl+Shift+=)");
                                }
                                modifiers.reset();
                            }
                            Key::Minus => {
                                if modifiers.ctrl && modifiers.shift {
                                    let _ = tx.send(HotkeyEvent::DecreaseFontSize);
                                    debug!("Hotkey: DecreaseFontSize (Ctrl+Shift+-)");
                                }
                                modifiers.reset();
                            }
                            Key::KeyX => {
                                if modifiers.ctrl && modifiers.alt {
                                    let _ = tx.send(HotkeyEvent::ClearText);
                                    debug!("Hotkey: ClearText (Ctrl+Alt+X)");
                                }
                                modifiers.reset();
                            }
                            Key::PageUp => {
                                if modifiers.ctrl {
                                    let _ = tx.send(HotkeyEvent::IncreaseTransparency);
                                    debug!("Hotkey: IncreaseTransparency (Ctrl+PageUp)");
                                }
                                modifiers.reset();
                            }
                            Key::PageDown => {
                                if modifiers.ctrl {
                                    let _ = tx.send(HotkeyEvent::DecreaseTransparency);
                                    debug!("Hotkey: DecreaseTransparency (Ctrl+PageDown)");
                                }
                                modifiers.reset();
                            }
                            Key::UpArrow => {
                                if modifiers.ctrl && modifiers.alt {
                                    let _ = tx.send(HotkeyEvent::IncreaseWindowHeight);
                                    debug!("Hotkey: IncreaseWindowHeight (Ctrl+Alt+Up)");
                                }
                                modifiers.reset();
                            }
                            Key::DownArrow => {
                                if modifiers.ctrl && modifiers.alt {
                                    let _ = tx.send(HotkeyEvent::DecreaseWindowHeight);
                                    debug!("Hotkey: DecreaseWindowHeight (Ctrl+Alt+Down)");
                                }
                                modifiers.reset();
                            }
                            Key::LeftArrow => {
                                if modifiers.ctrl && modifiers.alt {
                                    let _ = tx.send(HotkeyEvent::DecreaseWindowWidth);
                                    debug!("Hotkey: DecreaseWindowWidth (Ctrl+Alt+Left)");
                                }
                                modifiers.reset();
                            }
                            Key::RightArrow => {
                                if modifiers.ctrl && modifiers.alt {
                                    let _ = tx.send(HotkeyEvent::IncreaseWindowWidth);
                                    debug!("Hotkey: IncreaseWindowWidth (Ctrl+Alt+Right)");
                                }
                                modifiers.reset();
                            }
                            _ => {
                                modifiers.reset();
                            }
                        }
                    }
                    EventType::KeyRelease(key) => {
                        match key {
                            Key::ControlLeft | Key::ControlRight => {
                                modifiers.ctrl = false;
                            }
                            Key::ShiftLeft | Key::ShiftRight => {
                                modifiers.shift = false;
                            }
                            Key::Alt => {
                                modifiers.alt = false;
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            };

            if let Err(e) = listen(callback) {
                error!("Failed to start rdev listener: {:?}", e);
            }
        });
    }
}

impl Default for HotkeyListener {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default)]
struct Modifiers {
    ctrl: bool,
    shift: bool,
    alt: bool,
}

impl Modifiers {
    fn reset(&mut self) {
        self.ctrl = false;
        self.shift = false;
        self.alt = false;
    }
}