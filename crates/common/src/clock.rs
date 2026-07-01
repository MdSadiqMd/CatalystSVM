//! Clock abstraction for deterministic simulation and wall-clock modes

/// Clock trait for time operations — enables deterministic replay
pub trait Clock: Send + Sync {
    /// Current timestamp in milliseconds
    fn now_ms(&self) -> u64;

    /// Advance the clock (only meaningful for VirtualClock)
    fn advance(&self, ms: u64);

    /// Sleep/wait for a duration (modeled for virtual, real for system)
    fn sleep_ms(&self, ms: u64);
}

/// Virtual clock for deterministic simulations — time only advances explicitly
#[derive(Debug)]
pub struct VirtualClock {
    current_ms: std::sync::atomic::AtomicU64,
}

impl VirtualClock {
    pub fn new(start_ms: u64) -> Self {
        Self {
            current_ms: std::sync::atomic::AtomicU64::new(start_ms),
        }
    }

    pub fn set(&self, ms: u64) {
        self.current_ms
            .store(ms, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new(0)
    }
}

impl Clock for VirtualClock {
    fn now_ms(&self) -> u64 {
        self.current_ms.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn advance(&self, ms: u64) {
        self.current_ms
            .fetch_add(ms, std::sync::atomic::Ordering::SeqCst);
    }

    fn sleep_ms(&self, ms: u64) {
        self.advance(ms);
    }
}

/// System clock using real wall-clock time
#[derive(Debug, Default)]
pub struct SystemClock;

impl SystemClock {
    pub fn new() -> Self {
        Self
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }

    fn advance(&self, _ms: u64) {
        // No-op for system clock — time advances naturally
    }

    fn sleep_ms(&self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_virtual_clock_advance() {
        let clock = VirtualClock::new(1000);
        assert_eq!(clock.now_ms(), 1000);
        clock.advance(500);
        assert_eq!(clock.now_ms(), 1500);
    }

    #[test]
    fn test_virtual_clock_sleep() {
        let clock = VirtualClock::new(0);
        clock.sleep_ms(100);
        assert_eq!(clock.now_ms(), 100);
    }

    #[test]
    fn test_system_clock_advances() {
        let clock = SystemClock::new();
        let t1 = clock.now_ms();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let t2 = clock.now_ms();
        assert!(t2 >= t1);
    }
}
