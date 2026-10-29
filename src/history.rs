//! Fixed-capacity sample history backing every sparkline.

use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct History {
    buf: VecDeque<f64>,
    cap: usize,
}

impl History {
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap),
            cap: cap.max(1),
        }
    }

    pub fn push(&mut self, value: f64) {
        if self.buf.len() == self.cap {
            self.buf.pop_front();
        }
        self.buf
            .push_back(if value.is_finite() { value } else { 0.0 });
    }

    #[cfg(test)]
    pub fn latest(&self) -> f64 {
        self.buf.back().copied().unwrap_or(0.0)
    }

    /// Largest value among the most recent `n` samples.
    pub fn peak(&self, n: usize) -> f64 {
        self.buf.iter().rev().take(n).copied().fold(0.0, f64::max)
    }

    /// The most recent `width` samples, right-aligned: missing history is
    /// padded with `None` on the left so the newest sample always sits at the
    /// right edge of the graph.
    pub fn window(&self, width: usize) -> Vec<Option<f64>> {
        let have = self.buf.len().min(width);
        let mut out = vec![None; width - have];
        out.extend(
            self.buf
                .iter()
                .skip(self.buf.len() - have)
                .map(|v| Some(*v)),
        );
        out
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.buf.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_oldest_when_full() {
        let mut h = History::new(3);
        for v in [1.0, 2.0, 3.0, 4.0] {
            h.push(v);
        }
        assert_eq!(h.len(), 3);
        assert_eq!(h.window(3), vec![Some(2.0), Some(3.0), Some(4.0)]);
        assert_eq!(h.latest(), 4.0);
    }

    #[test]
    fn window_right_aligns_short_history() {
        let mut h = History::new(10);
        h.push(5.0);
        h.push(7.0);
        assert_eq!(h.window(4), vec![None, None, Some(5.0), Some(7.0)]);
        assert_eq!(h.window(1), vec![Some(7.0)]);
    }

    #[test]
    fn peak_only_looks_at_recent_samples() {
        let mut h = History::new(10);
        for v in [9.0, 1.0, 2.0, 3.0] {
            h.push(v);
        }
        assert_eq!(h.peak(3), 3.0);
        assert_eq!(h.peak(10), 9.0);
    }

    #[test]
    fn non_finite_values_become_zero() {
        let mut h = History::new(2);
        h.push(f64::NAN);
        assert_eq!(h.latest(), 0.0);
    }
}
