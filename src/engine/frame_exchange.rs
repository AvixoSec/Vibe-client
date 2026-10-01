//! Single-slot immutable snapshots. Readers never wait for the producer and
//! never copy frame contents; old snapshots remain valid while being drawn.
use std::sync::{Arc, Mutex, TryLockError};

pub struct LatestFrame<T> {
    latest: Mutex<Option<Arc<T>>>,
}

impl<T> LatestFrame<T> {
    pub const fn new() -> Self {
        Self {
            latest: Mutex::new(None),
        }
    }

    pub fn publish(&self, value: T) {
        // Allocation and disposal of the previous frame happen outside the lock.
        let next = Arc::new(value);
        let old = {
            let mut slot = self.latest.lock().unwrap_or_else(|e| e.into_inner());
            slot.replace(next)
        };
        drop(old);
    }

    pub fn try_snapshot(&self) -> Option<Arc<T>> {
        match self.latest.try_lock() {
            Ok(slot) => slot.as_ref().map(Arc::clone),
            Err(TryLockError::WouldBlock) => None,
            // The slot contains only whole immutable frames, including after
            // poisoning. No partially updated commands/string pair is exposed.
            Err(TryLockError::Poisoned(e)) => e.into_inner().as_ref().map(Arc::clone),
        }
    }
}

impl<T> Default for LatestFrame<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_until_first_publish() {
        assert!(LatestFrame::<Vec<u8>>::new().try_snapshot().is_none());
    }

    #[test]
    fn readers_share_storage_without_deep_copy() {
        let exchange = LatestFrame::new();
        exchange.publish(vec![1, 2, 3]);
        let first = exchange.try_snapshot().unwrap();
        let second = exchange.try_snapshot().unwrap();
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn previous_snapshot_survives_replacement() {
        let exchange = LatestFrame::new();
        exchange.publish((vec![0], vec!["first".to_string()]));
        let first = exchange.try_snapshot().unwrap();
        exchange.publish((vec![0], vec!["second".to_string()]));
        assert_eq!(first.1[0], "first");
        assert_eq!(exchange.try_snapshot().unwrap().1[0], "second");
    }

    #[test]
    fn busy_producer_does_not_block_reader() {
        let exchange = LatestFrame::new();
        exchange.publish(1);
        let _held = exchange.latest.lock().unwrap();
        assert!(exchange.try_snapshot().is_none());
    }

    #[test]
    fn whole_frames_remain_usable_after_poisoning() {
        let exchange = Arc::new(LatestFrame::new());
        exchange.publish(1);
        let other = Arc::clone(&exchange);
        let _ = std::thread::spawn(move || {
            let _held = other.latest.lock().unwrap();
            panic!("simulate producer panic");
        })
        .join();
        assert_eq!(*exchange.try_snapshot().unwrap(), 1);
        exchange.publish(2);
        assert_eq!(*exchange.try_snapshot().unwrap(), 2);
    }

    #[test]
    fn concurrent_publication_keeps_command_string_pairs_consistent() {
        let exchange = Arc::new(LatestFrame::new());
        let producer = Arc::clone(&exchange);
        let worker = std::thread::spawn(move || {
            for i in 0..1000 {
                producer.publish((i, i.to_string()));
            }
        });
        for _ in 0..1000 {
            if let Some(frame) = exchange.try_snapshot() {
                assert_eq!(frame.0.to_string(), frame.1);
            }
        }
        worker.join().unwrap();
        assert_eq!(exchange.try_snapshot().unwrap().0, 999);
    }
}
