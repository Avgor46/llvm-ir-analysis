//! A custom error.
use std::fmt;
use std::result;

#[derive(Debug)]
pub enum Error {
    CallGraph(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Error::CallGraph(ref err) => write!(f, "{err}"),
        }
    }
}

pub type Result<T> = result::Result<T, Error>;
