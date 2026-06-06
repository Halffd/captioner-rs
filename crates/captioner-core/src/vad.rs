use webrtc_vad::{Vad, VadMode, SampleRate};

const FRAME_SAMPLES_10MS: usize = 160;

pub struct VoiceActivityDetector {
    vad: Vad,
    frame_size: usize,
}

impl VoiceActivityDetector {
    pub fn new(sensitivity: u32) -> Self {
        let mode = match sensitivity {
            0 => VadMode::VeryAggressive,
            1 => VadMode::Aggressive,
            2 => VadMode::LowBitrate,
            _ => VadMode::Quality,
        };

        let vad = Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, mode);

        Self {
            vad,
            frame_size: FRAME_SAMPLES_10MS,
        }
    }

    pub fn is_speech(&mut self, audio_chunk: &[f32]) -> bool {
        if audio_chunk.len() < self.frame_size {
            return false;
        }

        let mut any_speech = false;
        for frame in audio_chunk.chunks(self.frame_size) {
            if frame.len() < self.frame_size {
                break;
            }

            let i16_frame: Vec<i16> = frame
                .iter()
                .map(|&s| (s * 32768.0).clamp(-32768.0, 32767.0) as i16)
                .collect();

            match self.vad.is_voice_segment(&i16_frame) {
                Ok(true) => {
                    any_speech = true;
                    break;
                }
                Ok(false) => {}
                Err(_) => {}
            }
        }

        any_speech
    }

    pub fn is_speech_all_frames(&mut self, audio_chunk: &[f32]) -> bool {
        if audio_chunk.len() < self.frame_size {
            return false;
        }

        let mut all_speech = true;
        for frame in audio_chunk.chunks(self.frame_size) {
            if frame.len() < self.frame_size {
                break;
            }

            let i16_frame: Vec<i16> = frame
                .iter()
                .map(|&s| (s * 32768.0).clamp(-32768.0, 32767.0) as i16)
                .collect();

            match self.vad.is_voice_segment(&i16_frame) {
                Ok(true) => {}
                Ok(false) => {
                    all_speech = false;
                    break;
                }
                Err(_) => {
                    all_speech = false;
                    break;
                }
            }
        }

        all_speech
    }

    pub fn set_sensitivity(&mut self, sensitivity: u32) {
        let mode = match sensitivity {
            0 => VadMode::VeryAggressive,
            1 => VadMode::Aggressive,
            2 => VadMode::LowBitrate,
            _ => VadMode::Quality,
        };
        self.vad.set_mode(mode);
    }

    pub fn frame_size(&self) -> usize {
        self.frame_size
    }

    pub fn reset(&mut self) {
        self.vad.reset();
    }
}

impl Clone for VoiceActivityDetector {
    fn clone(&self) -> Self {
        Self::new(0)
    }
}
