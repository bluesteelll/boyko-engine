use crate::ecs::constants::{MAX_EVENT_CAPACITY, MAX_EVENT_THREADS};
use crate::ecs::error::{EcsError, EcsResult};

/// Configuration for a per-type event buffer.
///
/// Specifies how many writer lanes (one per worker thread) and how many events
/// each lane can hold before overflowing. All buffers for a given event type
/// are allocated once at `preregister_event` time; no allocation occurs during
/// steady-state `send` or `update_events`.
///
/// Use [`EventConfig::new`] for validated construction or the [`DEFAULT`]
/// constant for single-threaded scenarios.
///
/// [`DEFAULT`]: EventConfig::DEFAULT
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EventConfig {
    /// Number of worker lanes. One per concurrent sender thread.
    pub(crate) thread_count: u32,
    /// Maximum events per lane per frame before `EventBufferFull` is returned.
    pub(crate) capacity_per_lane: u32,
}

impl EventConfig {
    /// Default per-lane capacity (1 024 events per thread per frame).
    pub const DEFAULT_CAPACITY: u32 = 1024;

    /// Convenience constant for single-threaded use with the default capacity.
    pub const DEFAULT: EventConfig = EventConfig {
        thread_count: 1,
        capacity_per_lane: Self::DEFAULT_CAPACITY,
    };

    /// Validates and constructs an `EventConfig`.
    ///
    /// # Errors
    ///
    /// Returns `Err(InvalidEventConfig)` if:
    /// - `thread_count` is 0 or exceeds [`MAX_EVENT_THREADS`] (64).
    /// - `capacity_per_lane` is 0 or exceeds [`MAX_EVENT_CAPACITY`] (16 384).
    pub fn new(thread_count: u32, capacity_per_lane: u32) -> EcsResult<Self> {
        if thread_count == 0 || thread_count > MAX_EVENT_THREADS {
            return Err(EcsError::InvalidEventConfig {
                reason: "thread_count out of range (must be 1..=64)",
            });
        }
        if capacity_per_lane == 0 || capacity_per_lane > MAX_EVENT_CAPACITY {
            return Err(EcsError::InvalidEventConfig {
                reason: "capacity_per_lane out of range (must be 1..=16384)",
            });
        }
        Ok(EventConfig { thread_count, capacity_per_lane })
    }

    /// Constructs a config with the given `thread_count` and [`DEFAULT_CAPACITY`].
    ///
    /// Validates `thread_count` via [`EventConfig::new`], so a caller that
    /// previously validated `thread_count` (e.g. `EventDispatcher::new`) may
    /// safely `.expect("invariant: ...")` the result.
    ///
    /// # Errors
    ///
    /// Returns `Err(InvalidEventConfig)` if `thread_count` is out of range.
    ///
    /// [`DEFAULT_CAPACITY`]: EventConfig::DEFAULT_CAPACITY
    #[inline]
    pub fn default_for(thread_count: u32) -> EcsResult<Self> {
        Self::new(thread_count, Self::DEFAULT_CAPACITY)
    }

    /// Returns the number of worker lanes.
    #[inline]
    pub fn thread_count(&self) -> u32 {
        self.thread_count
    }

    /// Returns the per-lane event capacity.
    #[inline]
    pub fn capacity_per_lane(&self) -> u32 {
        self.capacity_per_lane
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_config_bounds() {
        // Valid range.
        assert!(EventConfig::new(1, 1).is_ok());
        assert!(EventConfig::new(64, 16384).is_ok());

        // Zero thread_count is invalid.
        assert!(EventConfig::new(0, 1024).is_err());
        // Exceeding MAX_EVENT_THREADS is invalid.
        assert!(EventConfig::new(65, 1024).is_err());

        // Zero capacity is invalid.
        assert!(EventConfig::new(1, 0).is_err());
        // Exceeding MAX_EVENT_CAPACITY is invalid.
        assert!(EventConfig::new(1, 16385).is_err());
    }

    #[test]
    fn default_for_validates_thread_count() {
        assert!(EventConfig::default_for(1).is_ok());
        assert!(EventConfig::default_for(64).is_ok());
        assert!(EventConfig::default_for(0).is_err());
        assert!(EventConfig::default_for(65).is_err());
    }

    #[test]
    fn default_const_is_valid() {
        // The DEFAULT constant must pass validation so .expect() is honest.
        let cfg = EventConfig::new(
            EventConfig::DEFAULT.thread_count,
            EventConfig::DEFAULT.capacity_per_lane,
        );
        assert!(cfg.is_ok());
    }
}
