pub struct RingBuffer<T> {
    buf: Vec<T>,
    cap: usize,
    head: usize, // index of oldest element when full
}

impl<T> RingBuffer<T> {
    pub fn new(cap: usize) -> Self {
        RingBuffer { buf: Vec::new(), cap: cap.max(1), head: 0 }
    }

    pub fn push(&mut self, item: T) {
        if self.buf.len() < self.cap {
            self.buf.push(item);
        } else {
            self.buf[self.head] = item;
            self.head = (self.head + 1) % self.cap;
        }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &T> {
        let (a, b) = self.buf.split_at(self.head.min(self.buf.len()));
        b.iter().chain(a.iter())
    }
}

impl<T: Clone> RingBuffer<T> {
    pub fn to_vec(&self) -> Vec<T> {
        self.iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_within_capacity_preserves_order() {
        let mut r = RingBuffer::new(4);
        for v in [1, 2, 3] { r.push(v); }
        assert_eq!(r.len(), 3);
        assert_eq!(r.to_vec(), vec![1, 2, 3]);
    }

    #[test]
    fn push_over_capacity_overwrites_oldest() {
        let mut r = RingBuffer::new(3);
        for v in [1, 2, 3, 4, 5] { r.push(v); }
        assert_eq!(r.len(), 3);
        assert_eq!(r.to_vec(), vec![3, 4, 5]); // oldest -> newest, 1 and 2 evicted
    }

    #[test]
    fn zero_capacity_keeps_last_item() {
        let mut r = RingBuffer::new(0);
        r.push(7);
        r.push(8);
        assert_eq!(r.to_vec(), vec![8]);
    }
}
