//! Shared worlds: one save a group of friends owns together.
//!
//! The world always runs from its host's own storage, as single player does. A storage bucket the
//! group owns (S3's API: Cloudflare R2) holds a copy of the save and one small record, the
//! [`lease`], that says who hosts it now, who is playing and which copy is the newest. Whoever
//! opens the world while nobody hosts it takes the lease and hosts; when they leave, the lease
//! and the newest copy pass to a player who stays.
//!
//! What keeps a save whole:
//! - **One host.** Every change to the lease is a conditional write, which the storage accepts
//!   from one writer only ([`lease`]).
//! - **A copy counts once the lease names it.** Its files are written once and never changed
//!   ([`manifest`], [`sync`]), so a host that lost the lease can't put its copy in front of the
//!   group's.
//! - **Nothing is replaced before it is checked.** A file is rebuilt beside the one it replaces,
//!   its SHA-256 compared with the copy's file list, and only then renamed into place, behind a
//!   journal a restart finishes ([`apply`]). What it replaced is kept.
//!
//! No game code here: the game calls this crate, on this port as part of the engine and on the
//! official desktop game as a native mod.
#![forbid(unsafe_code)]

pub mod apply;
pub mod bore;
pub mod codec;
pub mod error;
pub mod invite;
pub mod lease;
pub mod manifest;
pub mod retention;
pub mod s3;
pub mod scan;
pub mod store;
pub mod sync;

pub use error::{Error, Result};
