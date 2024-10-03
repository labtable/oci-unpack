mod parser;

use crate::digest::Digest;

/// Representation of a reference to an image in an OCI registry.
#[derive(Debug, PartialEq)]
pub struct Reference<'a> {
    pub registry: &'a str,
    pub repository: Repository<'a>,
    pub tag: &'a str,
    pub digest: Option<Digest>,
}

#[derive(Debug, PartialEq)]
pub enum Repository<'a> {
    Full(&'a str),
    Prefixed(&'a str, &'a str),
}

impl<'a> std::fmt::Display for Repository<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Repository::Full(full) => f.write_str(full),
            Repository::Prefixed(a, b) => write!(f, "{a}/{b}"),
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum ParseError {
    #[error("digest")]
    Digest(#[from] crate::digest::DigestParseError),
}

impl<'a> Reference<'a> {
    pub fn parse(reference: &'a str) -> Result<Self, ParseError> {
        parser::parse(reference)
    }
}
