//! Per-node dirty flags — which subsystems need to re-process the node.

bitflags::bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct DirtyFlags: u32 {
        const LAYOUT  = 1 << 0;
        const VISUAL  = 1 << 1;
        const SPATIAL = 1 << 2;
        const ALL     = Self::LAYOUT.bits() | Self::VISUAL.bits() | Self::SPATIAL.bits();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_ops() {
        let mut f = DirtyFlags::empty();
        assert!(!f.contains(DirtyFlags::LAYOUT));
        f.insert(DirtyFlags::LAYOUT | DirtyFlags::VISUAL);
        assert!(f.contains(DirtyFlags::LAYOUT));
        assert!(f.contains(DirtyFlags::VISUAL));
        assert!(!f.contains(DirtyFlags::SPATIAL));
        f.remove(DirtyFlags::LAYOUT);
        assert!(!f.contains(DirtyFlags::LAYOUT));
    }
}
