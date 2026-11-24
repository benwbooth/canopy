//! Bandwidth limits (Insync has none): one token bucket for uploads and one
//! for downloads, shared by every transfer.

use parking_lot::Mutex;
use std::io::Read;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct Bucket {
    /// Bytes per second, 0 = unlimited.
    rate: AtomicU64,
    state: Mutex<Option<(f64, Instant)>>,
}

impl Bucket {
    const fn new() -> Self {
        Bucket { rate: AtomicU64::new(0), state: parking_lot::const_mutex(None) }
    }

    pub fn set_kib(&self, kib: u64) {
        self.rate.store(kib * 1024, Ordering::Relaxed);
        *self.state.lock() = None;
    }

    /// Wait until `n` bytes may pass.
    pub fn take(&self, n: usize) {
        let rate = self.rate.load(Ordering::Relaxed) as f64;
        if rate <= 0.0 {
            return;
        }
        let wait = {
            let mut g = self.state.lock();
            let now = Instant::now();
            let s = g.get_or_insert((rate, now));
            let elapsed = now.duration_since(s.1).as_secs_f64().min(2.0);
            s.0 = (s.0 + elapsed * rate).min(rate); // at most a second's burst
            s.1 = now;
            s.0 -= n as f64;
            if s.0 < 0.0 {
                Duration::from_secs_f64(-s.0 / rate)
            } else {
                Duration::ZERO
            }
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
    }
}

pub static UP: Bucket = Bucket::new();
pub static DOWN: Bucket = Bucket::new();

/// A reader that counts bytes, reports progress and keeps to a bucket.
pub struct Metered<'a, R: Read> {
    pub inner: R,
    pub bucket: &'static Bucket,
    pub done: u64,
    pub progress: &'a (dyn Fn(u64) + Sync),
}

impl<R: Read> Read for Metered<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // Small reads keep the limit smooth.
        let cap = buf.len().min(64 * 1024);
        let n = self.inner.read(&mut buf[..cap])?;
        if n > 0 {
            self.bucket.take(n);
            self.done += n as u64;
            (self.progress)(self.done);
        }
        Ok(n)
    }
}
