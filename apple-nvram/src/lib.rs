// SPDX-License-Identifier: MIT
use std::{
    borrow::Cow,
    fmt::{Debug, Display, Formatter},
    fs::File,
    os::unix::io::AsRawFd,
};

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
}

type Result<T> = std::result::Result<T, Error>;

#[repr(C)]
pub struct EraseInfoUser {
    start: u32,
    length: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct MtdInfoUser {
    ty: u8,
    flags: u32,
    size: u32,
    erasesize: u32,
    writesize: u32,
    oobsize: u32,
    padding: u64,
}

nix::ioctl_write_ptr!(mtd_mem_erase, b'M', 2, EraseInfoUser);
nix::ioctl_read!(mtd_mem_get_info, b'M', 1, MtdInfoUser);

pub fn erase_if_needed(file: &File, size: usize) {
    if unsafe { mtd_mem_get_info(file.as_raw_fd(), &mut MtdInfoUser::default()) }.is_err() {
        return;
    }
    let erase_info = EraseInfoUser {
        start: 0,
        length: size as u32,
    };
    unsafe {
        mtd_mem_erase(file.as_raw_fd(), &erase_info).unwrap();
    }
}

// #[derive(Debug)]
// pub enum Nvram<'a> {
//     V1V2(v1v2::Nvram<'a>),
//     V3(v3::Nvram<'a>),
// }

// impl<'a> Nvram<'a> {
//     pub fn parse(nvr: &'a [u8]) -> Result<Nvram<'a>> {
//         match (v3::Nvram::parse(nvr), v1v2::Nvram::parse(nvr)) {
//             (Ok(nvram_v3), Err(_)) => Ok(Nvram::V3(nvram_v3)),
//             (Err(_), Ok(nvram_v1v2)) => Ok(Nvram::V1V2(nvram_v1v2)),
//             _ => Err(Error::ParseError),
//         }
//     }

//     pub fn serialize(&self) -> Result<Vec<u8>> {
//         match self {
//             Nvram::V1V2(nvram) => nvram.serialize(),
//             Nvram::V3(nvram) => nvram.serialize(),
//         }
//     }

//     pub fn prepare_for_write(&mut self) {
//         match self {
//             Nvram::V1V2(nvram) => nvram.prepare_for_write(),
//             Nvram::V3(nvram) => nvram.prepare_for_write(),
//         }
//     }

//     pub fn active_part_mut(&mut self) -> PartitionMut<'a, '_> {
//         match self {
//             Nvram::V1V2(nvram) => PartitionMut::V1V2(nvram.active_part_mut()),
//             Nvram::V3(nvram) => PartitionMut::V3(nvram.active_part_mut()),
//         }
//     }

//     pub fn partitions(&self) -> impl Iterator<Item = Partition<'a, '_>> {
//         match self {
//             Nvram::V1V2(nvram) => Either::Left(nvram.partitions().map(|p| Partition::V1V2(p))),
//             Nvram::V3(nvram) => Either::Right(nvram.partitions().map(|p| Partition::V3(p))),
//         }
//     }
// }
// #[derive(Debug)]
// pub enum Partition<'a, 'b> {
//     V1V2(&'b v1v2::Partition<'a>),
//     V3(&'b v3::Partition<'a>),
// }

// impl<'a> Partition<'a, '_> {
//     pub fn variables(&self) -> impl Iterator<Item = Variable<'a, '_>> {
//         match self {
//             Partition::V1V2(p) => Either::Left(p.variables().map(|v| Variable::V1V2(v))),
//             Partition::V3(p) => Either::Right(p.variables().map(|v| Variable::V3(v))),
//         }
//     }
// }

// impl Display for Partition<'_, '_> {
//     fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
//         match *self {
//             Partition::V1V2(p) => Display::fmt(p, f),
//             Partition::V3(p) => Display::fmt(p, f),
//         }
//     }
// }

// #[derive(Debug)]
// pub enum PartitionMut<'a, 'b> {
//     V1V2(&'b mut v1v2::Partition<'a>),
//     V3(&'b mut v3::Partition<'a>),
// }

// impl<'a> PartitionMut<'a, '_> {
//     pub fn get_variable(&self, key: &'a [u8]) -> Option<Variable<'a, '_>> {
//         match self {
//             PartitionMut::V1V2(p) => p.get_variable(key).map(|v| Variable::V1V2(v)),
//             PartitionMut::V3(p) => p.get_variable(key).map(|v| Variable::V3(v)),
//         }
//     }

//     pub fn insert_variable(&mut self, key: &'a [u8], value: Cow<'a, [u8]>, typ: VarType) {
//         match self {
//             PartitionMut::V1V2(p) => p.insert_variable(key, value, typ),
//             PartitionMut::V3(p) => p.insert_variable(key, value, typ),
//         };
//     }

//     pub fn remove_variable(&mut self, key: &'a [u8], typ: VarType) {
//         match self {
//             PartitionMut::V1V2(p) => p.remove_variable(key, typ),
//             PartitionMut::V3(p) => p.remove_variable(key, typ),
//         };
//     }

//     pub fn variables(&self) -> impl Iterator<Item = Variable<'a, '_>> {
//         match self {
//             PartitionMut::V1V2(p) => Either::Left(p.variables().map(|v| Variable::V1V2(v))),
//             PartitionMut::V3(p) => Either::Right(p.variables().map(|v| Variable::V3(v))),
//         }
//     }
// }

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

// #[derive(Clone)]
// pub enum Variable<'a, 'b> {
//     V1V2(&'b v1v2::Variable<'a>),
//     V3(&'b v3::Variable<'a>),
// }

// impl<'a> Variable<'a, '_> {
//     pub fn value(&self) -> Cow<'a, [u8]> {
//         match *self {
//             Variable::V1V2(v) => v.value(),
//             Variable::V3(v) => v.value(),
//         }
//     }
// }

// impl Display for Variable<'_, '_> {
//     fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
//         match *self {
//             Variable::V1V2(v) => Display::fmt(v, f),
//             Variable::V3(v) => Display::fmt(v, f),
//         }
//     }
// }

pub fn nvram_parse<'a>(nvr: &'a [u8]) -> Result<Box<dyn Nvram<'a> + 'a>> {
    match (v3::Nvram::parse(nvr), v1v2::Nvram::parse(nvr)) {
        (Ok(nvram_v3), Err(_)) => Ok(Box::new(nvram_v3)),
        (Err(_), Ok(nvram_v1v2)) => Ok(Box::new(nvram_v1v2)),
        _ => Err(Error::ParseError),
    }
}

pub trait Nvram<'a> {
    fn prepare_for_write(&mut self);
    fn active_part_mut(&mut self) -> &mut dyn Partition<'a>;
    fn partitions(&self) -> Box<dyn Iterator<Item = &dyn Partition<'a>> + '_>;
    fn serialize(&self) -> Result<Vec<u8>>;
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
