//! Packed task priority ported from `util/tasks/task_priority.h`.

/// Represents task priority as four ordered 8-bit bands.
///
/// The packed `whole` value mirrors the C++ union layout on the supported
/// little-endian targets: band3 takes precedence over band2, then band1, then
/// band0, and regular integer ordering gives the task ordering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskPriority {
    pub whole: u32,
}

impl TaskPriority {
    pub const BAND_MAX: u8 = 255;

    pub const fn new(band0: u8, band1: u8, band2: u8, band3: u8) -> Self {
        Self {
            whole: band0 as u32
                | ((band1 as u32) << 8)
                | ((band2 as u32) << 16)
                | ((band3 as u32) << 24),
        }
    }

    pub const fn min() -> Self {
        Self { whole: 0 }
    }

    pub const fn max() -> Self {
        Self { whole: u32::MAX }
    }

    pub const fn band0(self) -> u8 {
        self.whole as u8
    }

    pub const fn band1(self) -> u8 {
        (self.whole >> 8) as u8
    }

    pub const fn band2(self) -> u8 {
        (self.whole >> 16) as u8
    }

    pub const fn band3(self) -> u8 {
        (self.whole >> 24) as u8
    }

    pub fn set_band0(&mut self, value: u8) {
        self.whole = (self.whole & !0x0000_00ff) | value as u32;
    }

    pub fn set_band1(&mut self, value: u8) {
        self.whole = (self.whole & !0x0000_ff00) | ((value as u32) << 8);
    }

    pub fn set_band2(&mut self, value: u8) {
        self.whole = (self.whole & !0x00ff_0000) | ((value as u32) << 16);
    }

    pub fn set_band3(&mut self, value: u8) {
        self.whole = (self.whole & !0xff00_0000) | ((value as u32) << 24);
    }
}

#[cfg(test)]
mod tests {
    use super::TaskPriority;

    #[test]
    fn default_is_min_priority() {
        assert_eq!(TaskPriority::default(), TaskPriority::min());
        assert_eq!(TaskPriority::min().whole, 0);
    }

    #[test]
    fn constructor_packs_bands_in_cpp_order() {
        let p = TaskPriority::new(1, 2, 3, 4);
        assert_eq!(p.whole, 0x0403_0201);
        assert_eq!(p.band0(), 1);
        assert_eq!(p.band1(), 2);
        assert_eq!(p.band2(), 3);
        assert_eq!(p.band3(), 4);
    }

    #[test]
    fn higher_bands_take_precedence_in_ordering() {
        assert!(TaskPriority::new(255, 0, 0, 0) < TaskPriority::new(0, 1, 0, 0));
        assert!(TaskPriority::new(255, 255, 0, 0) < TaskPriority::new(0, 0, 1, 0));
        assert!(TaskPriority::new(255, 255, 255, 0) < TaskPriority::new(0, 0, 0, 1));
    }

    #[test]
    fn setters_update_one_band_without_touching_others() {
        let mut p = TaskPriority::new(1, 2, 3, 4);
        p.set_band0(10);
        p.set_band1(20);
        p.set_band2(30);
        p.set_band3(40);
        assert_eq!(p, TaskPriority::new(10, 20, 30, 40));
    }

    #[test]
    fn max_priority_sets_all_bands() {
        assert_eq!(TaskPriority::max().whole, u32::MAX);
        assert_eq!(TaskPriority::max().band0(), TaskPriority::BAND_MAX);
        assert_eq!(TaskPriority::max().band1(), TaskPriority::BAND_MAX);
        assert_eq!(TaskPriority::max().band2(), TaskPriority::BAND_MAX);
        assert_eq!(TaskPriority::max().band3(), TaskPriority::BAND_MAX);
    }
}
