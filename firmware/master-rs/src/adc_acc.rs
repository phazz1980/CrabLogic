//! Pure logic of the continuous ADC reader: per-channel accumulation of DMA samples, the mean
//! since the last read, and the channel list for the driver pattern. No hardware here, so it is
//! tested on the desktop.

/// Channel slots (ADC1 has 8 channels on the ESP32, 5 on the C3).
pub const SLOTS: usize = 16;

#[derive(Debug, Clone, Copy)]
struct Slot {
    known: bool,
    sum: u64,
    count: u32,
    /// Mean of the last drain that brought samples; `None` before the first sample.
    last: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct AdcAccum {
    slots: [Slot; SLOTS],
}

impl Default for AdcAccum {
    fn default() -> Self {
        Self::new()
    }
}

impl AdcAccum {
    pub const fn new() -> Self {
        Self {
            slots: [Slot {
                known: false,
                sum: 0,
                count: 0,
                last: None,
            }; SLOTS],
        }
    }

    /// Adds `channel` to the pattern; true if it was not there yet (the driver needs a reconfigure).
    pub fn add_channel(&mut self, channel: u8) -> bool {
        match self.slots.get_mut(channel as usize) {
            Some(s) if !s.known => {
                s.known = true;
                true
            }
            _ => false,
        }
    }

    pub fn is_known(&self, channel: u8) -> bool {
        self.slots.get(channel as usize).is_some_and(|s| s.known)
    }

    /// Known channels in ascending order, for the driver pattern.
    pub fn channels(&self) -> impl Iterator<Item = u8> + '_ {
        self.slots
            .iter()
            .enumerate()
            .filter(|(_, s)| s.known)
            .map(|(i, _)| i as u8)
    }

    /// Adds one sample; samples of channels outside the pattern are ignored.
    pub fn push(&mut self, channel: u8, value: u16) {
        if let Some(s) = self.slots.get_mut(channel as usize) {
            if s.known {
                s.sum += u64::from(value);
                s.count = s.count.saturating_add(1);
            }
        }
    }

    /// True once the channel has produced at least one sample (pending or already taken).
    pub fn has_sample(&self, channel: u8) -> bool {
        self.slots
            .get(channel as usize)
            .is_some_and(|s| s.count > 0 || s.last.is_some())
    }

    /// Mean of the samples since the previous take (rounded); the last value when nothing new
    /// arrived; 0 before any sample.
    pub fn take(&mut self, channel: u8) -> i32 {
        let Some(s) = self.slots.get_mut(channel as usize) else {
            return 0;
        };
        if s.count > 0 {
            let mean = ((s.sum + u64::from(s.count) / 2) / u64::from(s.count)) as i32;
            s.sum = 0;
            s.count = 0;
            s.last = Some(mean);
        }
        s.last.unwrap_or(0)
    }

    /// Parses a DMA buffer of `width`-byte results (2 on the ESP32, 4 on the C3) into the
    /// accumulators. In both formats the low 16 bits are `data[11:0]`, `channel[15:12]`. A
    /// trailing partial result is ignored. Returns the number of results parsed.
    pub fn feed(&mut self, buf: &[u8], width: usize) -> usize {
        if width < 2 {
            return 0;
        }
        let mut n = 0;
        for r in buf.chunks_exact(width) {
            let word = u16::from_le_bytes([r[0], r[1]]);
            self.push((word >> 12) as u8, word & 0x0fff);
            n += 1;
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(channel: u8, value: u16, width: usize) -> Vec<u8> {
        let word = (u16::from(channel) << 12) | value;
        let mut v = word.to_le_bytes().to_vec();
        v.resize(width, 0);
        v
    }

    #[test]
    fn channels_ascend_and_dedupe() {
        let mut a = AdcAccum::new();
        assert!(a.add_channel(3));
        assert!(a.add_channel(0));
        assert!(!a.add_channel(3));
        assert!(!a.add_channel(200));
        assert_eq!(a.channels().collect::<Vec<_>>(), vec![0, 3]);
        assert!(a.is_known(0) && !a.is_known(1));
    }

    #[test]
    fn mean_then_last() {
        let mut a = AdcAccum::new();
        a.add_channel(2);
        assert!(!a.has_sample(2));
        assert_eq!(a.take(2), 0);
        a.push(2, 100);
        a.push(2, 101);
        assert!(a.has_sample(2));
        assert_eq!(a.take(2), 101); // 100.5 rounds up
        assert_eq!(a.take(2), 101); // nothing new: last value
        a.push(2, 7);
        assert_eq!(a.take(2), 7);
    }

    #[test]
    fn channels_do_not_mix() {
        let mut a = AdcAccum::new();
        a.add_channel(1);
        a.add_channel(4);
        a.push(1, 4000);
        a.push(4, 10);
        a.push(5, 999); // not in the pattern
        assert_eq!(a.take(4), 10);
        assert_eq!(a.take(1), 4000);
        assert_eq!(a.take(5), 0);
    }

    #[test]
    fn feed_both_widths() {
        for width in [2usize, 4] {
            let mut a = AdcAccum::new();
            a.add_channel(0);
            a.add_channel(6);
            let mut buf = Vec::new();
            buf.extend(result(0, 1000, width));
            buf.extend(result(6, 2000, width));
            buf.extend(result(0, 1002, width));
            buf.extend([0xff]); // partial tail
            assert_eq!(a.feed(&buf, width), 3);
            assert_eq!(a.take(0), 1001);
            assert_eq!(a.take(6), 2000);
        }
    }

    #[test]
    fn pending_counts_as_sample_and_big_sums_do_not_overflow() {
        let mut a = AdcAccum::new();
        a.add_channel(0);
        for _ in 0..2_000_000 {
            a.push(0, 4095);
        }
        assert_eq!(a.take(0), 4095);
    }
}
