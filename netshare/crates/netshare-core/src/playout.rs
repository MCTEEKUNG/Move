/// Simple adaptive playout (jitter) buffer for audio frames.
///
/// Strategy:
///  - Hold at least `target_frames` decoded PCM frames before outputting any.
///  - If the queue grows beyond `max_frames`, drop the oldest frame to catch up.
///  - Return `None` (→ caller outputs silence) when below the target threshold.
use std::collections::VecDeque;

pub struct PlayoutBuffer {
    frames: VecDeque<Vec<f32>>,
    target_frames: usize, // minimum buffering depth before playback starts
    max_frames: usize,    // drop oldest when we exceed this
}

impl PlayoutBuffer {
    /// `target_frames` — how many frames to pre-buffer before starting output.
    /// At 48kHz / 480 samples per frame, each frame = 10 ms.
    /// target_frames = 4 → 40 ms pre-buffer (enough to absorb typical LAN jitter).
    pub fn new(target_frames: usize) -> Self {
        Self {
            frames: VecDeque::new(),
            target_frames,
            max_frames: target_frames * 4,
        }
    }

    /// Push a newly decoded PCM frame (interleaved f32 samples).
    pub fn push(&mut self, frame: Vec<f32>) {
        if self.frames.len() >= self.max_frames {
            self.frames.pop_front(); // drop oldest to catch up
        }
        self.frames.push_back(frame);
    }

    /// Pop the next frame for playback.
    /// Returns `None` (output silence) if we haven't buffered enough yet.
    pub fn pop(&mut self) -> Option<Vec<f32>> {
        if self.frames.len() >= self.target_frames {
            self.frames.pop_front()
        } else {
            None
        }
    }

    pub fn buffered_frames(&self) -> usize {
        self.frames.len()
    }

    pub fn reset(&mut self) {
        self.frames.clear();
    }
}
