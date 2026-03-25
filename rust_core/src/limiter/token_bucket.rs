use std::time::Instant;

/// A simple token bucket rate limiter.
/// Can be used to limit bandwidth per-process.
pub struct TokenBucket {
    capacity: f64,     // max tokens (bytes)
    tokens: f64,       // current available tokens
    rate: f64,         // tokens per second (bytes/sec)
    last_refill: Instant,
}

impl TokenBucket {
    /// Create a new token bucket.
    /// `rate_bps` is the fill rate in bytes per second.
    /// `capacity` is the burst size in bytes.
    pub fn new(rate_bps: f64, capacity: f64) -> Self {
        Self {
            capacity,
            tokens: capacity,
            rate: rate_bps,
            last_refill: Instant::now(),
        }
    }

    /// Try to consume `amount` tokens. Returns true if allowed.
    pub fn try_consume(&mut self, amount: f64) -> bool {
        self.refill();
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }

    /// Update the rate limit.
    pub fn set_rate(&mut self, rate_bps: f64) {
        self.rate = rate_bps;
    }

    /// Update the burst capacity.
    pub fn set_capacity(&mut self, capacity: f64) {
        self.capacity = capacity;
        if self.tokens > capacity {
            self.tokens = capacity;
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
        self.last_refill = now;
    }
}
