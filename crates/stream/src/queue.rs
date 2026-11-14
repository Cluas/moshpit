//! A FIFO of bytes kept in fixed-size chunks.

use std::collections::VecDeque;

/// The size of a chunk.
pub(crate) const CHUNK: usize = 64 * 1024;

/// A FIFO of bytes in chunks of [`CHUNK`]: appending never moves what is
/// already queued (no 32 MB reallocation while a lock is held halfway
/// through a burst), and a drained queue keeps its last chunk, so steady
/// typing allocates nothing.
#[derive(Default)]
pub(crate) struct ByteQueue {
    chunks: VecDeque<Vec<u8>>,
    /// Read offset into the front chunk.
    head: usize,
    /// Bytes queued.
    len: usize,
    /// A drained chunk kept for the next one the queue needs.
    spare: Option<Vec<u8>>,
}

impl ByteQueue {
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Appends `data` at the back.
    pub(crate) fn push(&mut self, mut data: &[u8]) {
        self.len += data.len();
        if let Some(last) = self.chunks.back_mut() {
            let n = (last.capacity() - last.len()).min(data.len());
            last.extend_from_slice(&data[..n]);
            data = &data[n..];
        }
        while !data.is_empty() {
            let n = data.len().min(CHUNK);
            let mut chunk = self.spare.take().unwrap_or_default();
            chunk.reserve_exact(CHUNK);
            chunk.extend_from_slice(&data[..n]);
            self.chunks.push_back(chunk);
            data = &data[n..];
        }
    }

    /// Moves up to `buf.len()` bytes from the front into `buf`; returns how
    /// many.
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> usize {
        let mut n = 0;
        while n < buf.len() {
            let last = self.chunks.len() == 1;
            let Some(front) = self.chunks.front_mut() else {
                break;
            };
            let available = &front[self.head..];
            let k = available.len().min(buf.len() - n);
            buf[n..n + k].copy_from_slice(&available[..k]);
            n += k;
            self.head += k;
            if self.head < front.len() {
                continue;
            }
            self.head = 0;
            if last {
                // Keep the last chunk (and its room) for what comes next.
                front.clear();
                break;
            }
            if let Some(mut done) = self.chunks.pop_front() {
                done.clear();
                self.spare = Some(done);
            }
        }
        self.len -= n;
        n
    }

    /// Takes everything queued.
    #[cfg(all(test, not(moshpit_loom)))]
    pub(crate) fn take_all(&mut self) -> Vec<u8> {
        let mut bytes = vec![0; self.len];
        let n = self.read(&mut bytes);
        debug_assert_eq!(n, bytes.len());
        bytes
    }
}

#[cfg(all(test, not(moshpit_loom)))]
mod tests {
    use super::*;

    /// Bytes `0, 1, 2, …` (mod 251, a prime, so chunk boundaries never line
    /// up with the pattern).
    fn pattern(from: usize, len: usize) -> Vec<u8> {
        (from..from + len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn reads_what_was_pushed_in_order_across_chunks() {
        let mut queue = ByteQueue::default();
        let sizes = [1, 7, CHUNK - 8, 3 * CHUNK + 5, 0, 1, CHUNK, 2];
        let mut pushed = 0;
        for size in sizes {
            queue.push(&pattern(pushed, size));
            pushed += size;
        }
        assert_eq!(queue.len(), pushed);
        let mut out = Vec::new();
        let mut buf = vec![0; 1000];
        loop {
            let n = queue.read(&mut buf);
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        assert_eq!(out, pattern(0, pushed));
        assert!(queue.is_empty());
    }

    #[test]
    fn interleaved_pushes_and_reads_keep_order() {
        let mut queue = ByteQueue::default();
        let mut pushed = 0;
        let mut out = Vec::new();
        let mut buf = vec![0; 3 * CHUNK];
        for round in 0..200 {
            let size = (round * 7919) % (2 * CHUNK) + 1;
            queue.push(&pattern(pushed, size));
            pushed += size;
            let cap = (round * 104_729) % buf.len() + 1;
            let n = queue.read(&mut buf[..cap]);
            out.extend_from_slice(&buf[..n]);
        }
        out.extend(queue.take_all());
        assert_eq!(out, pattern(0, pushed));
    }

    #[test]
    fn a_drained_queue_keeps_one_chunk_and_a_spare() {
        let mut queue = ByteQueue::default();
        queue.push(&pattern(0, 3 * CHUNK));
        let all = queue.take_all();
        assert_eq!(all.len(), 3 * CHUNK);
        assert_eq!(queue.chunks.len(), 1);
        assert!(queue.spare.is_some());
        // Small pushes go into the kept chunk.
        queue.push(b"abc");
        assert_eq!(queue.chunks.len(), 1);
        assert_eq!(queue.take_all(), b"abc");
    }

    #[test]
    fn reading_into_an_empty_buffer_takes_nothing() {
        let mut queue = ByteQueue::default();
        queue.push(b"xyz");
        assert_eq!(queue.read(&mut []), 0);
        assert_eq!(queue.len(), 3);
    }
}
