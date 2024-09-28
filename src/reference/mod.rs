mod parser;

use std::fmt::Display;

/// Representation of a reference to an image in an OCI registry.
#[derive(Debug, PartialEq)]
pub(crate) struct Reference<'a> {
    pub registry: &'a str,
    pub repository: Repository<'a>,
    pub tag: &'a str,
    pub digest: Option<Digest<'a>>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Digest<'a> {
    SHA256(&'a str),
    SHA512(&'a str),
}

#[derive(Debug, PartialEq)]
pub(crate) enum Repository<'a> {
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

#[derive(Debug)]
pub(crate) struct ParseError<'a> {
    pub reference: &'a str,
    pub message: &'static str,
}

impl<'a> ParseError<'a> {
    fn new(reference: &'a str, message: &'static str) -> Self {
        ParseError { reference, message }
    }
}

impl<'a> Display for ParseError<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to parse {:?}: {}.", self.reference, self.message)
    }
}

impl<'a> Reference<'a> {
    pub fn parse(reference: &'a str) -> Result<Self, ParseError<'a>> {
        parser::parse(reference)
    }
}
