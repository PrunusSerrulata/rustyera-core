use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Instant;

use serde::Serialize;

pub(super) struct CountingAllocator;

static ENABLED: AtomicBool = AtomicBool::new(false);
static ACTIVE: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static DEALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static DEALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static NET_BYTES: AtomicI64 = AtomicI64::new(0);
static PEAK_NET_BYTES: AtomicI64 = AtomicI64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MeasurementMode {
    Counting,
    Off,
}

#[derive(Clone, Copy, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AllocationStats {
    pub(super) allocations: u64,
    pub(super) deallocations: u64,
    pub(super) allocated_bytes: u64,
    pub(super) deallocated_bytes: u64,
    pub(super) net_bytes: i64,
    pub(super) peak_net_bytes: i64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Calibration {
    pub(super) samples: u32,
    pub(super) off_ns: u128,
    pub(super) counting_ns: u128,
    pub(super) overhead_percent: f64,
}

impl CountingAllocator {
    pub(super) const fn new() -> Self {
        Self
    }
}

struct MeasurementGuard;

impl MeasurementGuard {
    fn begin() -> Self {
        if ACTIVE
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            panic!("nested allocation measurement is unsupported");
        }
        reset();
        ENABLED.store(true, Ordering::SeqCst);
        Self
    }
}

impl Drop for MeasurementGuard {
    fn drop(&mut self) {
        ENABLED.store(false, Ordering::SeqCst);
        ACTIVE.store(false, Ordering::SeqCst);
    }
}

struct SuspensionGuard {
    restore: bool,
}

impl Drop for SuspensionGuard {
    fn drop(&mut self) {
        if self.restore {
            ENABLED.store(true, Ordering::SeqCst);
        }
    }
}

fn size_i64(size: usize) -> i64 {
    i64::try_from(size).unwrap_or(i64::MAX)
}

fn record_allocation(size: usize) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let bytes = u64::try_from(size).unwrap_or(u64::MAX);
    let signed = size_i64(size);
    ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    ALLOCATED_BYTES.fetch_add(bytes, Ordering::Relaxed);
    let net = NET_BYTES.fetch_add(signed, Ordering::Relaxed).saturating_add(signed);
    let mut peak = PEAK_NET_BYTES.load(Ordering::Relaxed);
    while net > peak {
        match PEAK_NET_BYTES.compare_exchange_weak(
            peak,
            net,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break,
            Err(actual) => peak = actual,
        }
    }
}

fn record_deallocation(size: usize) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
    DEALLOCATED_BYTES.fetch_add(u64::try_from(size).unwrap_or(u64::MAX), Ordering::Relaxed);
    NET_BYTES.fetch_sub(size_i64(size), Ordering::Relaxed);
}

// SAFETY: all memory operations delegate to `System` with unchanged pointer/layout ownership.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplied a valid allocation layout.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        record_deallocation(layout.size());
        // SAFETY: the caller supplied the matching allocation pointer and layout.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller supplied a valid allocation layout.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller supplied the existing allocation and requested size.
        let replacement = unsafe { System.realloc(pointer, layout, new_size) };
        if !replacement.is_null() {
            record_deallocation(layout.size());
            record_allocation(new_size);
        }
        replacement
    }
}

pub(super) fn measure<T>(
    mode: MeasurementMode,
    operation: impl FnOnce() -> T,
) -> (T, Option<AllocationStats>) {
    if mode == MeasurementMode::Off {
        return (operation(), None);
    }
    let guard = MeasurementGuard::begin();
    let result = operation();
    drop(guard);
    (result, Some(snapshot()))
}

pub(super) fn without_counting<T>(operation: impl FnOnce() -> T) -> T {
    let guard = SuspensionGuard {
        restore: ENABLED.swap(false, Ordering::SeqCst),
    };
    let result = operation();
    drop(guard);
    result
}

pub(super) fn calibrate(samples: u32) -> Calibration {
    fn workload(samples: u32, mode: MeasurementMode) -> u128 {
        let started = Instant::now();
        let _ = measure(mode, || {
            for index in 0..samples {
                let mut value = Vec::with_capacity(64);
                value.extend_from_slice(&index.to_le_bytes());
                black_box(value);
            }
        });
        started.elapsed().as_nanos()
    }
    let off_ns = workload(samples, MeasurementMode::Off);
    let counting_ns = workload(samples, MeasurementMode::Counting);
    let overhead_percent = if off_ns == 0 {
        0.0
    } else {
        (counting_ns as f64 - off_ns as f64) * 100.0 / off_ns as f64
    };
    Calibration {
        samples,
        off_ns,
        counting_ns,
        overhead_percent,
    }
}

fn reset() {
    for counter in [
        &ALLOCATIONS,
        &DEALLOCATIONS,
        &ALLOCATED_BYTES,
        &DEALLOCATED_BYTES,
    ] {
        counter.store(0, Ordering::Relaxed);
    }
    NET_BYTES.store(0, Ordering::Relaxed);
    PEAK_NET_BYTES.store(0, Ordering::Relaxed);
}

fn snapshot() -> AllocationStats {
    AllocationStats {
        allocations: ALLOCATIONS.load(Ordering::Relaxed),
        deallocations: DEALLOCATIONS.load(Ordering::Relaxed),
        allocated_bytes: ALLOCATED_BYTES.load(Ordering::Relaxed),
        deallocated_bytes: DEALLOCATED_BYTES.load(Ordering::Relaxed),
        net_bytes: NET_BYTES.load(Ordering::Relaxed),
        peak_net_bytes: PEAK_NET_BYTES.load(Ordering::Relaxed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_mode_reports_null_and_counting_reports_net_semantics() {
        let (_, off) = measure(MeasurementMode::Off, || Vec::<u8>::with_capacity(8));
        assert!(off.is_none());
        let (value, counting) = measure(MeasurementMode::Counting, || Vec::<u8>::with_capacity(8));
        let counting = counting.unwrap();
        assert!(counting.allocations >= 1);
        assert!(counting.net_bytes >= 8);
        assert!(counting.peak_net_bytes >= counting.net_bytes);
        drop(value);
    }

    #[test]
    fn guard_disables_counting_after_unwind() {
        let _ = std::panic::catch_unwind(|| {
            let _ = measure(MeasurementMode::Counting, || panic!("fixture"));
        });
        assert!(!ENABLED.load(Ordering::SeqCst));
    }

    #[test]
    fn nested_counting_window_is_rejected_without_disabling_the_outer_window() {
        let result = std::panic::catch_unwind(|| {
            let _ = measure(MeasurementMode::Counting, || {
                let _ = measure(MeasurementMode::Counting, Vec::<u8>::new);
            });
        });
        assert!(result.is_err());
        assert!(!ENABLED.load(Ordering::SeqCst));
        assert!(!ACTIVE.load(Ordering::SeqCst));
    }

    #[test]
    fn freeing_an_older_allocation_reports_signed_window_growth() {
        let value = Vec::<u8>::with_capacity(32);
        let (_, counting) = measure(MeasurementMode::Counting, || drop(value));
        let counting = counting.unwrap();
        assert!(counting.deallocations >= 1);
        assert!(counting.net_bytes <= -32);
        assert_eq!(counting.peak_net_bytes, 0);
    }
}
