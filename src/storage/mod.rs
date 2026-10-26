//! The SQLite database: the connection and its foundation repositories, the
//! catalogue repository, the user-data change journal, the natural-order
//! collation registered on the pool, and the copy taken before a migration.

pub mod catalog;
pub mod database;
pub mod migration_backup;
pub mod natural_order;
pub mod sync;
