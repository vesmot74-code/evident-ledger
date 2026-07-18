//! In-memory fixed-window rate limiters for public verification endpoints (Stage 6.5).
//!
//! Rate limiting is IP-based and per-instance (in-memory).
//! It is a mitigating control against casual abuse and scraping,
//! not a hard guarantee against distributed or botnet-based probing.

use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const VERIFY_MAX_REQUESTS: u32 = 100;
pub const CERTIFICATE_MAX_REQUESTS: u32 = 20;
pub const REGISTER_MAX_REQUESTS: u32 = 10;
pub const DEFAULT_WINDOW_SECS: u64 = 60;
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitConfig {
    pub max_requests: u32,
    pub window_secs: u64,
    pub max_entries: usize,
}

impl RateLimitConfig {
    pub fn verify() -> Self {
        Self {
            max_requests: VERIFY_MAX_REQUESTS,
            window_secs: DEFAULT_WINDOW_SECS,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    pub fn certificate() -> Self {
        Self {
            max_requests: CERTIFICATE_MAX_REQUESTS,
            window_secs: DEFAULT_WINDOW_SECS,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    pub fn register() -> Self {
        Self {
            max_requests: REGISTER_MAX_REQUESTS,
            window_secs: DEFAULT_WINDOW_SECS,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    pub fn login() -> Self {
        Self {
            max_requests: 10,
            window_secs: DEFAULT_WINDOW_SECS,
            max_entries: DEFAULT_MAX_ENTRIES,
        }
    }

    pub fn window(&self) -> Duration {
        Duration::from_secs(self.window_secs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitDecision {
    pub allowed: bool,
    pub retry_after_secs: u64,
    pub remaining: u32,
    pub reset_unix: u64,
}

#[derive(Debug, Clone)]
struct WindowEntry {
    window_start: Instant,
    count: u32,
}

#[derive(Debug)]
pub struct FixedWindowLimiter {
    config: RateLimitConfig,
    entries: Mutex<HashMap<[u8; 32], WindowEntry>>,
}

impl FixedWindowLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        Self {
            config,
            entries: Mutex::new(HashMap::new()),
        }
    }

    pub fn config(&self) -> RateLimitConfig {
        self.config
    }

    pub fn reset(&self) {
        self.entries
            .lock()
            .expect("rate limit mutex poisoned")
            .clear();
    }

    pub fn check(&self, client_key: [u8; 32], now: Instant) -> RateLimitDecision {
        check_at(self, client_key, now)
    }
}

/// Public rate-limit key: `sha256(client_ip)` with optional user-agent extension.
pub fn rate_limit_client_key(ip: IpAddr, user_agent: Option<&str>) -> [u8; 32] {
    rate_limit_scoped_client_key(ip, user_agent, None)
}

/// Scoped rate-limit key, e.g. `register:<ip>` for registration abuse protection.
pub fn rate_limit_scoped_client_key(
    ip: IpAddr,
    user_agent: Option<&str>,
    scope: Option<&str>,
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    if let Some(scope) = scope {
        hasher.update(scope.as_bytes());
        hasher.update(b":");
    }
    hasher.update(ip.to_string().as_bytes());
    if let Some(ua) = user_agent {
        hasher.update(ua.as_bytes());
    }
    hasher.finalize().into()
}

fn check_at(
    limiter: &FixedWindowLimiter,
    client_key: [u8; 32],
    now: Instant,
) -> RateLimitDecision {
    let window = limiter.config.window();
    let mut entries = limiter.entries.lock().expect("rate limit mutex poisoned");
    evict_expired(&mut entries, window, now);
    enforce_capacity(&mut entries, limiter.config.max_entries);

    let entry = entries.entry(client_key).or_insert(WindowEntry {
        window_start: now,
        count: 0,
    });

    if now.duration_since(entry.window_start) >= window {
        entry.window_start = now;
        entry.count = 0;
    }

    let elapsed = now.duration_since(entry.window_start);
    let retry_after_secs = window
        .checked_sub(elapsed)
        .map(|d| d.as_secs().max(1))
        .unwrap_or(limiter.config.window_secs);
    let reset_unix = unix_after(entry.window_start, window);

    if entry.count >= limiter.config.max_requests {
        return RateLimitDecision {
            allowed: false,
            retry_after_secs,
            remaining: 0,
            reset_unix,
        };
    }

    entry.count += 1;
    RateLimitDecision {
        allowed: true,
        retry_after_secs,
        remaining: limiter.config.max_requests.saturating_sub(entry.count),
        reset_unix,
    }
}

fn evict_expired(entries: &mut HashMap<[u8; 32], WindowEntry>, window: Duration, now: Instant) {
    entries.retain(|_, entry| now.duration_since(entry.window_start) < window);
}

fn enforce_capacity(entries: &mut HashMap<[u8; 32], WindowEntry>, max_entries: usize) {
    if entries.len() <= max_entries {
        return;
    }
    let mut keys: Vec<[u8; 32]> = entries.keys().copied().collect();
    keys.sort_by_key(|key| entries[key].window_start);
    let overflow = entries.len().saturating_sub(max_entries);
    for key in keys.into_iter().take(overflow) {
        entries.remove(&key);
    }
}

fn unix_after(start: Instant, window: Duration) -> u64 {
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let elapsed = start.elapsed();
    now_unix + window.saturating_sub(elapsed).as_secs()
}

#[derive(Debug, Clone)]
pub struct LoginRateLimitState {
    pub login: std::sync::Arc<FixedWindowLimiter>,
    pub trust_proxy_headers: bool,
}

impl LoginRateLimitState {
    pub fn from_config(trust_proxy_headers: bool) -> Self {
        Self {
            login: std::sync::Arc::new(FixedWindowLimiter::new(RateLimitConfig::login())),
            trust_proxy_headers,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PublicRateLimitState {
    pub verify: std::sync::Arc<FixedWindowLimiter>,
    pub certificate: std::sync::Arc<FixedWindowLimiter>,
    pub register: std::sync::Arc<FixedWindowLimiter>,
    pub trust_proxy_headers: bool,
    pub include_user_agent_in_key: bool,
}

impl PublicRateLimitState {
    pub fn from_config(trust_proxy_headers: bool) -> Self {
        Self {
            verify: std::sync::Arc::new(FixedWindowLimiter::new(RateLimitConfig::verify())),
            certificate: std::sync::Arc::new(FixedWindowLimiter::new(RateLimitConfig::certificate())),
            register: std::sync::Arc::new(FixedWindowLimiter::new(RateLimitConfig::register())),
            trust_proxy_headers,
            include_user_agent_in_key: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    fn test_limiter(max: u32, window_secs: u64) -> FixedWindowLimiter {
        FixedWindowLimiter::new(RateLimitConfig {
            max_requests: max,
            window_secs,
            max_entries: 100,
        })
    }

    #[test]
    fn fixed_window_blocks_after_limit() {
        let limiter = test_limiter(3, 60);
        let key = rate_limit_client_key(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), None);
        let start = Instant::now();
        assert!(check_at(&limiter, key, start).allowed);
        assert!(check_at(&limiter, key, start).allowed);
        assert!(check_at(&limiter, key, start).allowed);
        assert!(!check_at(&limiter, key, start).allowed);
    }

    #[test]
    fn fixed_window_resets_after_window() {
        let limiter = test_limiter(2, 60);
        let key = rate_limit_client_key(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), None);
        let start = Instant::now();
        assert!(check_at(&limiter, key, start).allowed);
        assert!(check_at(&limiter, key, start).allowed);
        assert!(!check_at(&limiter, key, start + Duration::from_secs(1)).allowed);
        assert!(check_at(&limiter, key, start + Duration::from_secs(61)).allowed);
    }

    #[test]
    fn different_ips_are_isolated() {
        let limiter = test_limiter(1, 60);
        let key_a = rate_limit_client_key(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 3)), None);
        let key_b = rate_limit_client_key(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 4)), None);
        let now = Instant::now();
        assert!(check_at(&limiter, key_a, now).allowed);
        assert!(!check_at(&limiter, key_a, now).allowed);
        assert!(check_at(&limiter, key_b, now).allowed);
    }

    #[test]
    fn hash_change_does_not_change_client_key() {
        let ip = IpAddr::V4(Ipv4Addr::new(203, 0, 113, 9));
        let key = rate_limit_client_key(ip, None);
        let key_again = rate_limit_client_key(ip, None);
        assert_eq!(key, key_again);
    }

    #[test]
    fn expired_entries_are_evicted() {
        let limiter = test_limiter(1, 1);
        let key = rate_limit_client_key(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)), None);
        let start = Instant::now();
        assert!(check_at(&limiter, key, start).allowed);
        assert!(!check_at(&limiter, key, start).allowed);
        assert!(check_at(&limiter, key, start + Duration::from_secs(2)).allowed);
        assert_eq!(limiter.entries.lock().unwrap().len(), 1);
    }
}
