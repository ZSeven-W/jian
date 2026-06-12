//! Theme tokens for reusable components.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Density {
    #[default]
    Desktop,
    Touch,
}

/// Placeholder token set; Phase 2.2 fills this with shadcn colors.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Tokens {
    pub density: Density,
}
