//! Lightweight luma-only content observations, not semantic scene recognition.
const COLS: usize = 32;
const ROWS: usize = 18;
const N: usize = COLS * ROWS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContentClass { Unknown, Changing, StableDetail, StableSimple }

pub struct ContentAnalyzer {
    previous: [u8; N],
    geometry: Option<(usize,usize)>,
    stable_frames: u8,
    class: ContentClass,
}

impl Default for ContentAnalyzer {
    fn default() -> Self {
        Self { previous: [0;N], geometry: None, stable_frames: 0, class: ContentClass::Unknown }
    }
}

impl ContentAnalyzer {
    pub fn class(&self) -> ContentClass { self.class }

    /// Sample cell centers and local one-pixel gradients in the existing Y plane.
    /// This estimates texture, not text or face identity. No per-frame allocation.
    pub fn observe(&mut self, y: &[u8], width: usize, height: usize) -> ContentClass {
        if width < 2 || height < 2 || width.checked_mul(height).is_none_or(|n| n > y.len()) {
            *self = Self::default();
            return self.class;
        }
        let same = self.geometry == Some((width,height));
        let mut delta = 0u32;
        let mut edges = 0u32;
        for row in 0..ROWS {
            let py = ((2*row+1)*height/(2*ROWS)).min(height-2);
            for col in 0..COLS {
                let px = ((2*col+1)*width/(2*COLS)).min(width-2);
                let i = py*width+px;
                let k = row*COLS+col;
                let value = y[i];
                delta += u32::from(value.abs_diff(self.previous[k]));
                self.previous[k] = value;
                if value.abs_diff(y[i+1]) >= 24 || value.abs_diff(y[i+width]) >= 24 { edges += 1; }
            }
        }
        self.geometry = Some((width,height));
        if !same || delta > N as u32 * 3 {
            self.stable_frames = 0;
            self.class = ContentClass::Changing;
        } else {
            self.stable_frames = self.stable_frames.saturating_add(1);
            if self.stable_frames >= 15 {
                self.class = if edges >= N as u32 / 20 { ContentClass::StableDetail }
                    else { ContentClass::StableSimple };
            }
        }
        self.class
    }
}

/// Only reduce the rate of sustained simple content, never exceed the network
/// controller's budget. Detail and motion retain the full available budget.
pub fn content_rate(network_kbps: u32, class: ContentClass) -> u32 {
    if class == ContentClass::StableSimple {
        (network_kbps.saturating_mul(3)/4).max(500).min(network_kbps)
    } else { network_kbps }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_simple_requires_sustained_observations_and_change_is_immediate() {
        let mut a = ContentAnalyzer::default();
        let y = vec![90;64*36];
        assert_eq!(a.observe(&y,64,36),ContentClass::Changing);
        for _ in 0..14 { assert_eq!(a.observe(&y,64,36),ContentClass::Changing); }
        assert_eq!(a.observe(&y,64,36),ContentClass::StableSimple);
        assert_eq!(a.observe(&vec![180;64*36],64,36),ContentClass::Changing);
        assert_eq!(a.observe(&[],64,36),ContentClass::Unknown);
    }
    #[test]
    fn stable_edges_keep_network_budget() {
        let mut a = ContentAnalyzer::default();
        let y: Vec<u8> = (0..64*36).map(|i| if i%2==0 {16} else {235}).collect();
        for _ in 0..16 { a.observe(&y,64,36); }
        assert_eq!(a.class(),ContentClass::StableDetail);
        assert_eq!(content_rate(6000,a.class()),6000);
        assert_eq!(content_rate(6000,ContentClass::StableSimple),4500);
        assert_eq!(content_rate(500,ContentClass::StableSimple),500);
    }
}
