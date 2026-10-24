/// Whether a load fits under a limit.
pub fn fits(used: u32, limit: u32) -> bool {
    used <= limit
}

/// What is left of a limit once `used` is taken.
pub fn spare(limit: u32, used: u32) -> u32 {
    limit - used
}
