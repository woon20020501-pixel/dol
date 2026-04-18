//! Bounded-channel backpressure for multi-symbol tick fan-out.
//!
//! When many symbols produce ticks faster than the decision engine consumes
//! them, we need a principled overflow policy. An unbounded channel OOMs;
//! naive blocking head-of-line-blocks behind the slowest consumer. This
//! module exposes a [`BoundedTickChannel`] whose overflow behaviour is
//! explicit and operator-chosen.
//!
//! # Policies
//!
//! - **`DropOldest`** — on full, discard the oldest queued item. Correct
//!   for real-time decision loops where stale ticks are useless.
//! - **`DropNewest`** — on full, discard the incoming item.
//! - **`Block`** — async-wait for a slot.
//!
//! # Implementation
//!
//! We use `Arc<Mutex<VecDeque<T>>>` + `tokio::sync::Notify` so `DropOldest`
//! can evict synchronously without racing the consumer. This is heavier
//! than `mpsc` but semantically exact for the drop-oldest case (which
//! `tokio::sync::mpsc` alone cannot implement).
//!
//! # References
//! - Kahn (1974) "The semantics of a simple language for parallel
//!   programming", IFIP Congress — bounded FIFO channels.
//! - Lamport (1977) "Concurrent reading and writing", CACM 20(11):806.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::{Mutex, Notify};

/// How to handle a send when the channel buffer is full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackpressurePolicy {
    /// Drop the oldest queued item to make room for the incoming one.
    DropOldest,
    /// Drop the incoming item; queue stays unchanged.
    DropNewest,
    /// Async-wait for a free slot.
    Block,
}

#[derive(Debug)]
struct Inner<T> {
    buf: Mutex<VecDeque<T>>,
    capacity: usize,
    policy: BackpressurePolicy,
    /// Notified by the producer on every enqueue (so recv() can wake).
    /// Also notified by the consumer on every dequeue (so Block senders
    /// can wake).
    notify: Notify,
    dropped_oldest: AtomicU64,
    dropped_newest: AtomicU64,
    blocked_sends: AtomicU64,
    closed: std::sync::atomic::AtomicBool,
}

#[derive(Debug)]
pub struct BoundedTickChannel<T> {
    inner: Arc<Inner<T>>,
}

#[derive(Debug)]
pub struct BoundedTickRx<T> {
    inner: Arc<Inner<T>>,
}

impl<T> Clone for BoundedTickChannel<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T: Send + 'static> BoundedTickChannel<T> {
    /// Construct a new channel. Returns (sender, receiver). The sender is
    /// cloneable; the receiver is not (single-consumer by convention).
    pub fn new(capacity: usize, policy: BackpressurePolicy) -> (Self, BoundedTickRx<T>) {
        assert!(capacity > 0, "capacity must be > 0");
        let inner = Arc::new(Inner {
            buf: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
            policy,
            notify: Notify::new(),
            dropped_oldest: AtomicU64::new(0),
            dropped_newest: AtomicU64::new(0),
            blocked_sends: AtomicU64::new(0),
            closed: std::sync::atomic::AtomicBool::new(false),
        });
        (
            Self {
                inner: Arc::clone(&inner),
            },
            BoundedTickRx { inner },
        )
    }

    /// Send an item respecting the configured policy.
    /// Returns `Ok(true)` if enqueued, `Ok(false)` if dropped per policy,
    /// `Err(ChannelClosed)` if the receiver was dropped.
    pub async fn send(&self, item: T) -> Result<bool, ChannelClosed> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(ChannelClosed);
        }
        match self.inner.policy {
            BackpressurePolicy::DropNewest => {
                let mut buf = self.inner.buf.lock().await;
                if buf.len() >= self.inner.capacity {
                    drop(buf);
                    self.inner.dropped_newest.fetch_add(1, Ordering::Relaxed);
                    Ok(false)
                } else {
                    buf.push_back(item);
                    drop(buf);
                    self.inner.notify.notify_one();
                    Ok(true)
                }
            }
            BackpressurePolicy::DropOldest => {
                let mut buf = self.inner.buf.lock().await;
                if buf.len() >= self.inner.capacity {
                    // Evict head; push tail.
                    buf.pop_front();
                    self.inner.dropped_oldest.fetch_add(1, Ordering::Relaxed);
                }
                buf.push_back(item);
                drop(buf);
                self.inner.notify.notify_one();
                Ok(true)
            }
            BackpressurePolicy::Block => {
                self.inner.blocked_sends.fetch_add(1, Ordering::Relaxed);
                loop {
                    {
                        let mut buf = self.inner.buf.lock().await;
                        if buf.len() < self.inner.capacity {
                            buf.push_back(item);
                            drop(buf);
                            self.inner.notify.notify_one();
                            return Ok(true);
                        }
                    }
                    // Full — await a dequeue notification.
                    self.inner.notify.notified().await;
                    if self.inner.closed.load(Ordering::Acquire) {
                        return Err(ChannelClosed);
                    }
                }
            }
        }
    }

    pub fn policy(&self) -> BackpressurePolicy {
        self.inner.policy
    }
    pub fn dropped_oldest(&self) -> u64 {
        self.inner.dropped_oldest.load(Ordering::Relaxed)
    }
    pub fn dropped_newest(&self) -> u64 {
        self.inner.dropped_newest.load(Ordering::Relaxed)
    }
    pub fn blocked_sends(&self) -> u64 {
        self.inner.blocked_sends.load(Ordering::Relaxed)
    }
}

impl<T: Send + 'static> BoundedTickRx<T> {
    /// Async recv; waits when empty. Returns None when the channel is
    /// closed (last sender dropped).
    pub async fn recv(&mut self) -> Option<T> {
        loop {
            {
                let mut buf = self.inner.buf.lock().await;
                if let Some(item) = buf.pop_front() {
                    drop(buf);
                    self.inner.notify.notify_one();
                    return Some(item);
                }
            }
            if self.inner.closed.load(Ordering::Acquire) {
                // Drain any remaining items before closing.
                let mut buf = self.inner.buf.lock().await;
                return buf.pop_front();
            }
            self.inner.notify.notified().await;
        }
    }

    /// Non-blocking recv. Returns Err(Empty) when the queue is empty.
    pub async fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let mut buf = self.inner.buf.lock().await;
        match buf.pop_front() {
            Some(v) => {
                drop(buf);
                self.inner.notify.notify_one();
                Ok(v)
            }
            None => Err(TryRecvError::Empty),
        }
    }
}

impl<T> Drop for BoundedTickRx<T> {
    fn drop(&mut self) {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelClosed;

impl std::fmt::Display for ChannelClosed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("channel closed: receiver dropped")
    }
}

impl std::error::Error for ChannelClosed {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TryRecvError {
    Empty,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn drop_newest_drops_incoming_when_full() {
        let (tx, mut rx) = BoundedTickChannel::<i32>::new(2, BackpressurePolicy::DropNewest);
        assert!(tx.send(1).await.unwrap());
        assert!(tx.send(2).await.unwrap());
        assert!(!tx.send(3).await.unwrap(), "3 must be dropped");
        assert_eq!(tx.dropped_newest(), 1);
        assert_eq!(rx.recv().await, Some(1));
        assert_eq!(rx.recv().await, Some(2));
    }

    #[tokio::test]
    async fn drop_oldest_evicts_head_and_enqueues_new() {
        let (tx, mut rx) = BoundedTickChannel::<i32>::new(2, BackpressurePolicy::DropOldest);
        assert!(tx.send(1).await.unwrap());
        assert!(tx.send(2).await.unwrap());
        assert!(tx.send(3).await.unwrap(), "3 must enqueue (evicting 1)");
        assert_eq!(tx.dropped_oldest(), 1);
        // FIFO after eviction: [2, 3]
        assert_eq!(rx.recv().await, Some(2));
        assert_eq!(rx.recv().await, Some(3));
    }

    #[tokio::test]
    async fn drop_oldest_accumulates_counter() {
        let (tx, mut rx) = BoundedTickChannel::<i32>::new(1, BackpressurePolicy::DropOldest);
        for i in 0..5 {
            assert!(tx.send(i).await.unwrap());
        }
        assert_eq!(tx.dropped_oldest(), 4);
        assert_eq!(rx.recv().await, Some(4));
    }

    #[tokio::test]
    async fn block_policy_waits_until_slot_free() {
        let (tx, mut rx) = BoundedTickChannel::<i32>::new(2, BackpressurePolicy::Block);
        assert!(tx.send(1).await.unwrap());
        assert!(tx.send(2).await.unwrap());
        // Spawn consumer that drains one item after 40ms.
        let tx2 = tx.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = rx.recv().await;
            // Keep rx alive through end of test.
            (rx, tx2)
        });
        let t0 = std::time::Instant::now();
        assert!(tx.send(3).await.unwrap());
        assert!(
            t0.elapsed() >= Duration::from_millis(30),
            "Block should have waited ~40ms, got {:?}",
            t0.elapsed()
        );
        assert_eq!(tx.blocked_sends(), 3);
        let _ = handle.await;
    }

    #[tokio::test]
    async fn receiver_drop_closes_channel() {
        let (tx, rx) = BoundedTickChannel::<i32>::new(2, BackpressurePolicy::DropNewest);
        drop(rx);
        match tx.send(1).await {
            Err(ChannelClosed) => {}
            other => panic!("expected ChannelClosed, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn counters_independent_per_policy() {
        let (tx_dn, _rx) = BoundedTickChannel::<i32>::new(1, BackpressurePolicy::DropNewest);
        let _ = tx_dn.send(1).await;
        let _ = tx_dn.send(2).await;
        assert_eq!(tx_dn.dropped_newest(), 1);
        assert_eq!(tx_dn.dropped_oldest(), 0);

        let (tx_do, _rx) = BoundedTickChannel::<i32>::new(1, BackpressurePolicy::DropOldest);
        let _ = tx_do.send(1).await;
        let _ = tx_do.send(2).await;
        assert_eq!(tx_do.dropped_oldest(), 1);
        assert_eq!(tx_do.dropped_newest(), 0);
    }
}
