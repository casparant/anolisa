//! Ledger database schema migration.
//!
//! The schema is compiled into the binary at build time so the store
//! carries no runtime dependency on external SQL files.

use rusqlite::Connection;

/// Applies the v1 Ledger schema to `conn`.
///
/// All statements use `IF NOT EXISTS` so re-running the migration on an
/// already-initialized database is a no-op.
pub(crate) fn run(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(include_str!("migration/schema.sql"))
}
