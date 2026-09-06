// The state database: everything the user decided, keyed portably, journalled so it can roam.
//
// `schema` is frozen contract. Every write goes through `journal`, which appends an event and then
// applies it, so that replaying the journal from empty reproduces the tables exactly.

pub mod device;
pub mod journal;
pub mod merge;
pub mod read;
pub mod schema;
pub mod write;

#[cfg(test)]
mod tests;
