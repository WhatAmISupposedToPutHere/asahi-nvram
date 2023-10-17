// SPDX-License-Identifier: MIT
use std::{
    borrow::Cow,
    fmt::{Debug, Display, Formatter},
};

pub mod mtd;
pub use mtd::erase_if_needed; // TODO: remove

pub mod v1v2;
pub mod v3;

fn chrp_checksum_add(lhs: u8, rhs: u8) -> u8 {
    let (out, carry) = lhs.overflowing_add(rhs);
    if carry {
        out + 1
    } else {
        out
    }
}

fn slice_rstrip<'a, T: PartialEq<T>>(mut ts: &'a [T], t: &T) -> &'a [T] {
    while let Some(last) = ts.last() {
        if last == t {
            ts = ts.split_last().unwrap().1;
        } else {
            break;
        }
    }
    ts
}

fn slice_find<T: PartialEq<T>>(ts: &[T], t: &T) -> Option<usize> {
    let mut ret = None;
    for (i, v) in ts.iter().enumerate() {
        if v == t {
            ret = Some(i);
            break;
        }
    }
    ret
}

#[derive(Debug)]
pub enum Error {
    ParseError,
    SectionTooBig,
    ApplyError(std::io::Error),
}

type Result<T> = std::result::Result<T, Error>;

#[derive(Clone)]
pub enum VarType {
    Common,
    System,
}

impl Display for VarType {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            &VarType::Common => write!(f, "common"),
            &VarType::System => write!(f, "system"),
        }
    }
}

pub fn nvram_parse<'a>(nvr: &'a [u8]) -> Result<Box<dyn Nvram<'a> + 'a>> {
    match (v3::Nvram::parse(nvr), v1v2::Nvram::parse(nvr)) {
        (Ok(nvram_v3), Err(_)) => Ok(Box::new(nvram_v3)),
        (Err(_), Ok(nvram_v1v2)) => Ok(Box::new(nvram_v1v2)),
        _ => Err(Error::ParseError),
    }
}

pub trait NvramWriter {
    fn write_all(&mut self, offset: u32, buf: &[u8]) -> std::io::Result<()>;
}

pub trait Nvram<'a> {
    fn prepare_for_write(&mut self);
    fn active_part_mut(&mut self) -> &mut dyn Partition<'a>;
    fn partitions(&self) -> Box<dyn Iterator<Item = &dyn Partition<'a>> + '_>;
    fn serialize(&self) -> Result<Vec<u8>>;
    fn apply(&self, w: &mut dyn NvramWriter) -> Result<()> {
        let data = self.serialize()?;
        // TODO: only erase the bank that was actually modified
        // (do not overwrite the whole nvram)
        w.write_all(0, &data).map_err(|e| Error::ApplyError(e))?;
        Ok(())
    }
}

pub trait Partition<'a>: Display {
    fn variables(&self) -> Box<dyn Iterator<Item = &dyn Variable<'a>> + '_>;
    fn get_variable(&self, key: &'a [u8]) -> Option<&dyn Variable<'a>>;
    fn insert_variable(&mut self, key: &'a [u8], value: Cow<'a, [u8]>, typ: VarType);
    fn remove_variable(&mut self, key: &'a [u8], typ: VarType);
}

pub trait Variable<'a>: Display {
    fn value(&self) -> Cow<'a, [u8]>;
}
