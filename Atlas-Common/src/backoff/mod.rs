//! Retry scheduling: how long to wait before attempting something again.
//!
//! Three layers, because the call sites need genuinely different things:
//!
//! 1. [`BackoffPolicy`] — the schedule itself. Pure: no clock, no I/O, no state.
//!    [`BackoffPolicy::base_delay_for_attempt`] is deterministic, so protocols that
//!    need every replica to compute the *same* delay (a view-change timeout, say) can
//!    use it directly.
//! 2. [`Backoff`] — a cursor over a policy. Tracks the attempt count and when the next
//!    attempt becomes due, and *never sleeps*, so a single-threaded state machine can
//!    keep servicing its message loop while a retry is pending. See
//!    [`Backoff::time_until_ready`], which is meant to be fed to a select timeout.
//! 3. [`Backoff::sleep`] and [`retry_with_backoff`] — blocking sugar for code that
//!    owns its thread and may park.
//!
//! # Jitter
//!
//! Jitter is **opt-in**. It matters whenever several nodes fail at the same instant —
//! at a cold start every peer is refused at once, and an unjittered schedule marches
//! them all in lockstep — but it is a correctness bug wherever replicas must agree on
//! the delay they computed. So the default is [`Jitter::None`] and
//! [`BackoffPolicy::base_delay_for_attempt`] ignores jitter entirely.

use std::fmt::Debug;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use tracing::warn;

use crate::prng::ThreadSafePrng;

/// How much randomness to mix into a computed delay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Jitter {
    /// Use the computed delay exactly. Deterministic: every node that runs the same
    /// policy at the same attempt count gets the same answer.
    #[default]
    None,
    /// Wait at least half the computed delay, then a random amount up to the other
    /// half. Keeps the schedule's shape while breaking lockstep between peers.
    Equal,
    /// Wait a random amount anywhere in `0..=delay`. Spreads the most, but a single
    /// retry can come back almost immediately.
    Full,
}

/// When to stop retrying.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Budget {
    /// Keep retrying forever.
    #[default]
    Unlimited,
    /// Give up after this many attempts.
    Attempts(u32),
    /// Give up once this much time has passed since the first attempt.
    ///
    /// Prefer this over [`Budget::Attempts`] when what you actually care about is how
    /// long you are willing to wait: an attempt count only pins that down if the
    /// schedule never changes.
    Elapsed(Duration),
}

/// A retry schedule. Pure — construct it once, `const` if you like, and share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackoffPolicy {
    initial: Duration,
    max: Duration,
    factor: u32,
    jitter: Jitter,
    budget: Budget,
}

impl BackoffPolicy {
    /// The same delay before every attempt.
    pub const fn fixed(delay: Duration) -> Self {
        Self {
            initial: delay,
            max: delay,
            factor: 1,
            jitter: Jitter::None,
            budget: Budget::Unlimited,
        }
    }

    /// Start at `initial` and double before each attempt, never exceeding `max`.
    ///
    /// `max` is expected to be at least `initial`; if it is not, every delay is `max`.
    pub const fn exponential(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            factor: 2,
            jitter: Jitter::None,
            budget: Budget::Unlimited,
        }
    }

    /// Grow by `factor` instead of doubling. A factor below 2 makes the schedule flat.
    pub const fn with_factor(mut self, factor: u32) -> Self {
        self.factor = factor;
        self
    }

    pub const fn with_jitter(mut self, jitter: Jitter) -> Self {
        self.jitter = jitter;
        self
    }

    pub const fn with_budget(mut self, budget: Budget) -> Self {
        self.budget = budget;
        self
    }

    pub const fn initial(&self) -> Duration {
        self.initial
    }

    pub const fn max(&self) -> Duration {
        self.max
    }

    pub const fn budget(&self) -> Budget {
        self.budget
    }

    /// The delay before the `attempt`-th retry (0-based), ignoring jitter.
    ///
    /// Deterministic, and the only variant safe to use where replicas must agree.
    pub fn base_delay_for_attempt(&self, attempt: u32) -> Duration {
        if self.factor <= 1 {
            return min_duration(self.initial, self.max);
        }

        // Walks up the schedule rather than computing a power, so an absurd `attempt`
        // cannot overflow -- growth stops the moment the cap is reached, which for any
        // sane policy is within a handful of steps.
        let mut delay = self.initial;

        for _ in 0..attempt {
            match delay.checked_mul(self.factor) {
                Some(next) if next < self.max => delay = next,
                _ => return self.max,
            }
        }

        min_duration(delay, self.max)
    }

    /// [`Self::base_delay_for_attempt`] with this policy's jitter applied.
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        apply_jitter(self.base_delay_for_attempt(attempt), self.jitter)
    }
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self::exponential(Duration::from_millis(50), Duration::from_secs(1))
    }
}

/// A cursor over a [`BackoffPolicy`]: how many attempts have been made, and when the
/// next one is due.
///
/// Nothing here blocks except [`Backoff::sleep`], so this is usable from a state
/// machine that must stay responsive to its message loop.
#[derive(Debug, Clone)]
pub struct Backoff {
    policy: BackoffPolicy,
    attempt: u32,
    started_at: Option<Instant>,
    ready_at: Option<Instant>,
}

impl Backoff {
    pub fn new(policy: BackoffPolicy) -> Self {
        Self {
            policy,
            attempt: 0,
            started_at: None,
            ready_at: None,
        }
    }

    pub fn policy(&self) -> &BackoffPolicy {
        &self.policy
    }

    /// Attempts recorded so far.
    pub fn attempts(&self) -> u32 {
        self.attempt
    }

    /// Whether the policy's [`Budget`] has been spent. An exhausted backoff never
    /// becomes ready again until it is [`reset`](Self::reset).
    pub fn is_exhausted(&self) -> bool {
        self.is_exhausted_at(Instant::now())
    }

    pub fn is_exhausted_at(&self, now: Instant) -> bool {
        match self.policy.budget {
            Budget::Unlimited => false,
            Budget::Attempts(limit) => self.attempt >= limit,
            Budget::Elapsed(limit) => self
                .started_at
                .is_some_and(|start| now.saturating_duration_since(start) >= limit),
        }
    }

    /// Record that an attempt just happened, and schedule the next one.
    pub fn record_attempt(&mut self) {
        self.record_attempt_at(Instant::now());
    }

    pub fn record_attempt_at(&mut self, now: Instant) {
        self.started_at.get_or_insert(now);

        let delay = self.policy.delay_for_attempt(self.attempt);

        self.attempt = self.attempt.saturating_add(1);
        self.ready_at = Some(now + delay);
    }

    /// Whether another attempt is due. False while waiting, and false once exhausted.
    pub fn is_ready(&self) -> bool {
        self.is_ready_at(Instant::now())
    }

    pub fn is_ready_at(&self, now: Instant) -> bool {
        if self.is_exhausted_at(now) {
            return false;
        }

        self.ready_at.is_none_or(|ready| now >= ready)
    }

    /// How long until the next attempt is due; [`Duration::ZERO`] if it is due now or
    /// nothing has been attempted yet.
    ///
    /// Reduce this with `min` across every pending backoff to get the timeout for a
    /// select loop, so the loop wakes exactly when there is work rather than on a
    /// fixed poll interval. Check [`Self::is_exhausted`] separately — an exhausted
    /// backoff also reports zero, and has nothing left to do.
    pub fn time_until_ready(&self) -> Duration {
        self.time_until_ready_at(Instant::now())
    }

    pub fn time_until_ready_at(&self, now: Instant) -> Duration {
        self.ready_at
            .map_or(Duration::ZERO, |ready| ready.saturating_duration_since(now))
    }

    /// Advance the schedule and hand back the delay that was just scheduled, or `None`
    /// if the budget is spent.
    pub fn next_delay(&mut self) -> Option<Duration> {
        let now = Instant::now();

        if self.is_exhausted_at(now) {
            return None;
        }

        self.record_attempt_at(now);

        Some(self.time_until_ready_at(now))
    }

    /// Forget every attempt, as after a success.
    pub fn reset(&mut self) {
        self.attempt = 0;
        self.started_at = None;
        self.ready_at = None;
    }

    /// Block for the next scheduled delay. Returns `false` if the budget is spent, in
    /// which case nothing was slept and the caller should give up.
    ///
    /// Only for code that owns its thread. A state machine sharing a thread with a
    /// message loop wants [`Self::time_until_ready`] instead.
    pub fn sleep(&mut self) -> bool {
        match self.next_delay() {
            Some(delay) => {
                if !delay.is_zero() {
                    std::thread::sleep(delay);
                }

                true
            }
            None => false,
        }
    }
}

/// Run `op` on the calling thread, sleeping between attempts, until it succeeds or the
/// policy's [`Budget`] runs out. The last error is returned.
///
/// The shape mirrors [`crate::circuit_breaker::CircuitBreaker::execute_in_circuit_breaker`],
/// which retries immediately; this one waits. Use it for a plain "keep trying this"
/// loop — where the retry body needs to branch or bail early, drive a [`Backoff`]
/// directly instead.
pub fn retry_with_backoff<F, T, E>(policy: BackoffPolicy, mut op: F) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
    E: Debug,
{
    let mut backoff = Backoff::new(policy);

    loop {
        match op() {
            Ok(value) => return Ok(value),
            Err(err) => {
                if backoff.is_exhausted() {
                    return Err(err);
                }

                warn!(
                    "Attempt {} failed, retrying after backoff. Error: {:?}",
                    backoff.attempts() + 1,
                    err
                );

                if !backoff.sleep() {
                    return Err(err);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

const fn min_duration(a: Duration, b: Duration) -> Duration {
    if a.as_nanos() <= b.as_nanos() { a } else { b }
}

/// Shared across every jittered backoff. `ThreadSafePrng` is thread-local internally,
/// so this costs no synchronisation, and keeps `Backoff` cheap to construct — seeding a
/// generator per peer would hit `OsRng` once per connection.
fn jitter_rng() -> &'static ThreadSafePrng {
    static RNG: OnceLock<ThreadSafePrng> = OnceLock::new();

    RNG.get_or_init(ThreadSafePrng::new)
}

fn apply_jitter(delay: Duration, jitter: Jitter) -> Duration {
    if matches!(jitter, Jitter::None) || delay.is_zero() {
        return delay;
    }

    let nanos = delay.as_nanos().min(u64::MAX as u128) as u64;

    let jittered = match jitter {
        Jitter::None => nanos,
        Jitter::Equal => {
            let half = nanos / 2;

            half + random_below(nanos - half + 1)
        }
        Jitter::Full => random_below(nanos + 1),
    };

    Duration::from_nanos(jittered)
}

/// Uniform-ish draw in `0..bound`. The modulo bias is irrelevant at these magnitudes —
/// this is spreading retries, not generating keys.
fn random_below(bound: u64) -> u64 {
    if bound <= 1 {
        return 0;
    }

    jitter_rng().next_state() % bound
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    #[test]
    fn exponential_doubles_then_caps() {
        let policy = BackoffPolicy::exponential(MS(25), MS(1000));

        assert_eq!(policy.base_delay_for_attempt(0), MS(25));
        assert_eq!(policy.base_delay_for_attempt(1), MS(50));
        assert_eq!(policy.base_delay_for_attempt(2), MS(100));
        assert_eq!(policy.base_delay_for_attempt(3), MS(200));
        assert_eq!(policy.base_delay_for_attempt(4), MS(400));
        assert_eq!(policy.base_delay_for_attempt(5), MS(800));
        assert_eq!(policy.base_delay_for_attempt(6), MS(1000));
    }

    #[test]
    fn absurd_attempt_counts_saturate_at_max() {
        let policy = BackoffPolicy::exponential(MS(25), MS(1000));

        assert_eq!(policy.base_delay_for_attempt(u32::MAX), MS(1000));
    }

    #[test]
    fn fixed_policy_never_grows() {
        let policy = BackoffPolicy::fixed(MS(500));

        assert_eq!(policy.base_delay_for_attempt(0), MS(500));
        assert_eq!(policy.base_delay_for_attempt(50), MS(500));
    }

    #[test]
    fn max_below_initial_clamps_to_max() {
        let policy = BackoffPolicy::exponential(MS(1000), MS(10));

        assert_eq!(policy.base_delay_for_attempt(0), MS(10));
        assert_eq!(policy.base_delay_for_attempt(3), MS(10));
    }

    #[test]
    fn base_delay_ignores_jitter() {
        let policy = BackoffPolicy::exponential(MS(100), MS(100)).with_jitter(Jitter::Full);

        for _ in 0..100 {
            assert_eq!(policy.base_delay_for_attempt(3), MS(100));
        }
    }

    #[test]
    fn equal_jitter_stays_in_the_upper_half() {
        let policy = BackoffPolicy::fixed(MS(100)).with_jitter(Jitter::Equal);

        for _ in 0..1000 {
            let delay = policy.delay_for_attempt(0);

            assert!(delay >= MS(50), "{delay:?} below half");
            assert!(delay <= MS(100), "{delay:?} above the base delay");
        }
    }

    #[test]
    fn full_jitter_stays_within_the_delay() {
        let policy = BackoffPolicy::fixed(MS(100)).with_jitter(Jitter::Full);

        for _ in 0..1000 {
            assert!(policy.delay_for_attempt(0) <= MS(100));
        }
    }

    #[test]
    fn jitter_actually_varies() {
        let policy = BackoffPolicy::fixed(MS(100)).with_jitter(Jitter::Full);

        let first = policy.delay_for_attempt(0);
        let varied = (0..1000).any(|_| policy.delay_for_attempt(0) != first);

        assert!(varied, "jittered delays never differed");
    }

    #[test]
    fn attempt_budget_exhausts() {
        let mut backoff =
            Backoff::new(BackoffPolicy::exponential(MS(1), MS(2)).with_budget(Budget::Attempts(3)));

        for _ in 0..3 {
            assert!(!backoff.is_exhausted());
            assert!(backoff.next_delay().is_some());
        }

        assert!(backoff.is_exhausted());
        assert!(backoff.next_delay().is_none());
    }

    #[test]
    fn elapsed_budget_exhausts_on_the_clock_not_the_count() {
        let policy = BackoffPolicy::fixed(MS(1)).with_budget(Budget::Elapsed(MS(100)));

        let mut backoff = Backoff::new(policy);
        let start = Instant::now();

        backoff.record_attempt_at(start);

        assert!(!backoff.is_exhausted_at(start + MS(99)));
        assert!(backoff.is_exhausted_at(start + MS(100)));
    }

    #[test]
    fn ready_only_after_the_scheduled_delay() {
        let mut backoff = Backoff::new(BackoffPolicy::fixed(MS(100)));
        let start = Instant::now();

        assert!(backoff.is_ready_at(start), "first attempt should be due");

        backoff.record_attempt_at(start);

        assert!(!backoff.is_ready_at(start + MS(99)));
        assert_eq!(backoff.time_until_ready_at(start + MS(99)), MS(1));

        assert!(backoff.is_ready_at(start + MS(100)));
        assert_eq!(backoff.time_until_ready_at(start + MS(100)), Duration::ZERO);
    }

    #[test]
    fn exhausted_backoff_is_never_ready() {
        let mut backoff =
            Backoff::new(BackoffPolicy::fixed(MS(1)).with_budget(Budget::Attempts(1)));

        let start = Instant::now();

        backoff.record_attempt_at(start);

        assert!(!backoff.is_ready_at(start + MS(1000)));
    }

    #[test]
    fn reset_restores_the_first_attempt() {
        let mut backoff = Backoff::new(
            BackoffPolicy::exponential(MS(10), MS(1000)).with_budget(Budget::Attempts(2)),
        );

        backoff.record_attempt();
        backoff.record_attempt();

        assert!(backoff.is_exhausted());

        backoff.reset();

        assert_eq!(backoff.attempts(), 0);
        assert!(!backoff.is_exhausted());
        assert!(backoff.is_ready());
        assert_eq!(backoff.time_until_ready(), Duration::ZERO);
    }

    #[test]
    fn next_delay_follows_the_schedule() {
        let mut backoff = Backoff::new(BackoffPolicy::exponential(MS(10), MS(40)));

        assert_eq!(backoff.next_delay(), Some(MS(10)));
        assert_eq!(backoff.next_delay(), Some(MS(20)));
        assert_eq!(backoff.next_delay(), Some(MS(40)));
        assert_eq!(backoff.next_delay(), Some(MS(40)));
    }

    #[test]
    fn retry_returns_the_first_success() {
        let mut calls = 0;

        let result: Result<u32, &str> = retry_with_backoff(
            BackoffPolicy::fixed(Duration::ZERO).with_budget(Budget::Attempts(10)),
            || {
                calls += 1;

                if calls < 3 { Err("not yet") } else { Ok(calls) }
            },
        );

        assert_eq!(result, Ok(3));
    }

    #[test]
    fn retry_gives_up_on_an_exhausted_budget() {
        let mut calls = 0;

        let result: Result<(), &str> = retry_with_backoff(
            BackoffPolicy::fixed(Duration::ZERO).with_budget(Budget::Attempts(3)),
            || {
                calls += 1;

                Err("always")
            },
        );

        assert_eq!(result, Err("always"));
        assert_eq!(calls, 4, "one call per attempt, plus the one that gives up");
    }
}
