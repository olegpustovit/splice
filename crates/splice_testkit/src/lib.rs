//! Fixtures, golden snapshots, seeded op generator, MCP transcript replay, allocation probes (mem_sys counting).
#![deny(missing_docs)]

use std::{fs, io, path::Path};

/// Allocation assertions for a thread-local counting allocator.
pub mod alloc {
    /// Measures only the current thread. Pass `mem_sys::local_alloc_count` with
    /// the `counting` feature enabled in the test crate.
    #[derive(Clone, Copy)]
    pub struct Meter {
        count: fn() -> u64,
        reset: fn(),
    }

    impl Meter {
        /// Uses the given thread-local counter and its reset function.
        pub fn new(count: fn() -> u64, reset: fn()) -> Self {
            Self { count, reset }
        }

        /// Runs the action once to warm it, then asserts the second run allocates nothing.
        pub fn warm(&self, action: &mut dyn FnMut()) {
            action();
            (self.reset)();
            action();
            let allocations = (self.count)();
            assert_eq!(allocations, 0, "allocations on warm invocation");
        }
    }
}

/// Small deterministic random stream for reproducible tests.
pub struct Xorshift(u64);

impl Xorshift {
    /// Seeds xorshift64. The all-zero state is replaced with a nonzero seed.
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    /// Advances the stream and returns the next value.
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Compares a golden byte for byte, or refreshes it when `SPLICE_BLESS=1`.
pub fn golden(path: &Path, actual: &[u8]) -> io::Result<()> {
    check_golden(
        path,
        actual,
        std::env::var("SPLICE_BLESS").as_deref() == Ok("1"),
    )
}

fn check_golden(path: &Path, actual: &[u8], bless: bool) -> io::Result<()> {
    if bless {
        return fs::write(path, actual);
    }
    let expected = fs::read(path)?;
    if expected == actual {
        return Ok(());
    }
    let offset = expected
        .iter()
        .zip(actual)
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(actual.len()));
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "{} differs at byte {offset} (expected {} bytes, got {})",
            path.display(),
            expected.len(),
            actual.len()
        ),
    ))
}

/// Entry point reserved for JSONL transcript verification.
pub mod transcript {
    use std::{io, path::Path};

    /// Reports that replay is unavailable until the MCP transport and fixtures land in T3.4.
    pub fn run(_path: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "MCP transcript replay is pending T3.4",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_detects_allocations() {
        let meter = alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
        meter.warm(&mut || std::hint::black_box(()));
        let result = std::panic::catch_unwind(|| {
            meter.warm(&mut || {
                let data = Box::new(std::hint::black_box(42));
                std::hint::black_box(data);
            });
        });
        assert!(result.is_err());
    }

    #[test]
    fn pending_transcript_uses_an_explicit_error() {
        let error = transcript::run(Path::new("sample.jsonl")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn xorshift_sequence_is_reproducible() {
        let mut rng = Xorshift::new(1);
        assert_eq!(rng.next_u64(), 1_082_269_761);
        assert_eq!(rng.next_u64(), 1_152_992_998_833_853_505);
        assert_ne!(Xorshift::new(0).next_u64(), 0);
    }

    #[test]
    fn golden_requires_exact_bytes_and_can_refresh() {
        let dir = std::env::temp_dir().join(format!("splice-golden-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("snapshot.json");
        assert_eq!(
            check_golden(&path, b"{}", false).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        check_golden(&path, b"{}", true).unwrap();
        check_golden(&path, b"{}", false).unwrap();
        assert_eq!(
            check_golden(&path, b"{}\n", false).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
