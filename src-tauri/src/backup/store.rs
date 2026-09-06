// Three operations, and the reason there are only three.
//
// A journal segment is a whole blob written once under a name that never changes meaning, so
// nothing above this trait needs a rename, a delete, a directory, a lock, a range read or a
// conditional write. What is left is put, get and list, which is the intersection of Drive's REST
// API and S3, and small enough that the second implementation was an afternoon and the one in
// `tests.rs` is thirty lines. A backup whose store trait needs a transaction is a backup that
// cannot be pointed at somebody's own bucket.
//
// A name is a path with forward slashes: `<account-hash>/<device-id>/<first>-<last>.seg`. Drive
// has no paths, so it maps them onto folders; S3 has no folders, so it takes the name as the key.
// Neither is allowed to invent a layout of its own, because two implementations that disagree
// about where a segment lives are two backups that cannot be swapped.
//
// Async in the `Provider` trait's shape: `impl Future + Send` on a `Sync` trait, so there is no
// boxing and no `async_trait`.

use std::future::Future;

pub trait BackupStore: Sync {
    /// Whole, or not at all. Both implementations write a blob in one request, so a name either
    /// does not exist or names every byte of a segment; a pass cut off halfway leaves the segments
    /// it finished and nothing else. `pass` depends on that and `tests.rs` holds it to it.
    fn put(&self, name: &str, bytes: &[u8]) -> impl Future<Output = Result<(), String>> + Send;

    fn get(&self, name: &str) -> impl Future<Output = Result<Vec<u8>, String>> + Send;

    /// Every name under a prefix, in no particular order. The caller sorts what it needs sorted.
    fn list(&self, prefix: &str) -> impl Future<Output = Result<Vec<String>, String>> + Send;
}
