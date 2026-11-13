//! What can go wrong, kept apart by what the caller does about it.
use std::{fmt, io};

#[derive(Debug)]
pub enum Error {
    /// A file on this device.
    Io(io::Error),
    /// The storage refused a conditional write: the object changed since it was read.
    Refused,
    /// This game no longer holds the world: another host took it. Nothing more may be
    /// published from here.
    Fenced,
    /// The storage couldn't be reached or answered with an error; trying again may work.
    Store(String),
    /// Bytes that aren't what their checksum or file list says.
    Corrupt(String),
    /// A record that can't be read: a newer format, or not one of ours.
    Format(String),
    /// The player called it off.
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "{e}"),
            Self::Refused => f.write_str("the storage refused the write: the record changed first"),
            Self::Fenced => f.write_str("another player hosts this world now"),
            Self::Store(m) => write!(f, "the world's storage: {m}"),
            Self::Corrupt(m) => write!(f, "damaged data: {m}"),
            Self::Format(m) => write!(f, "unreadable record: {m}"),
            Self::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Format(e.to_string())
    }
}
