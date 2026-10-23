//! Fresh ids from the clock and a random source.

use uuid::Uuid;

use crate::app::ports::IdSource;

/// Gives UUID v7 ids: ordered by the time they are made, with random bits after.
#[derive(Debug, Default, Clone, Copy)]
pub struct UuidV7;

impl IdSource for UuidV7 {
    fn next_uuid(&mut self) -> Uuid {
        Uuid::now_v7()
    }
}
