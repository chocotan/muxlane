//! 环形回放缓冲：镜像订阅的首帧数据（最近 N 字节的历史输出）
use bytes::{Bytes, BytesMut};

pub struct ReplayBuffer {
    buf: BytesMut,
    capacity: usize,
}

impl ReplayBuffer {
    pub fn new(capacity: usize) -> Self {
        ReplayBuffer {
            buf: BytesMut::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, data: &[u8]) {
        if data.len() >= self.capacity {
            // 单次超容量：只留尾部
            self.buf.clear();
            self.buf
                .extend_from_slice(&data[data.len() - self.capacity..]);
            return;
        }
        // 先写，再裁剪到容量（split_to 丢弃最旧）
        self.buf.extend_from_slice(data);
        let total = self.buf.len();
        if total > self.capacity {
            let _ = self.buf.split_to(total - self.capacity);
        }
    }

    /// 快照：当前缓冲的只读视图
    pub fn snapshot(&self) -> Bytes {
        self.buf.clone().freeze()
    }

    /// 尾部快照：只拷贝最后 n 字节（屏幕采样等只需尾部的热路径，避免全量拷贝）
    pub fn tail(&self, n: usize) -> Bytes {
        let start = self.buf.len().saturating_sub(n);
        Bytes::copy_from_slice(&self.buf[start..])
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_tail() {
        let mut rb = ReplayBuffer::new(10);
        rb.push(b"0123456789");
        rb.push(b"abc");
        let snap = rb.snapshot();
        // 13 字节裁到 10：3456789abc
        assert_eq!(&snap[..], b"3456789abc");
    }

    #[test]
    fn single_over_capacity_keeps_tail() {
        let mut rb = ReplayBuffer::new(4);
        rb.push(b"123456");
        assert_eq!(&rb.snapshot()[..], b"3456");
    }

    #[test]
    fn tail_copies_only_suffix() {
        let mut rb = ReplayBuffer::new(16);
        rb.push(b"0123456789abcdef");
        assert_eq!(&rb.tail(4)[..], b"cdef");
        assert_eq!(&rb.tail(64)[..], b"0123456789abcdef");
        assert!(rb.tail(0).is_empty());
    }
}
