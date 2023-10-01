// SPDX-License-Identifier: MIT
use std::{
    borrow::Cow,
    collections::HashMap,
    fmt::{Debug, Formatter, self},
    fs::File,
    os::unix::io::AsRawFd,
};

pub mod v1v2;
pub mod v3;

pub struct UnescapeVal<I> {
    inner: I,
    esc_out: u8,
    remaining: u8,
}

impl<I> UnescapeVal<I>
where
    I: Iterator<Item = u8>,
{
    pub fn new(inner: I) -> Self {
        Self {
            inner,
            esc_out: 0,
            remaining: 0,
        }
    }
}

impl<I> Iterator for UnescapeVal<I>
where
    I: Iterator<Item = u8>,
{
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        if self.remaining != 0 {
            self.remaining -= 1;
            return Some(self.esc_out);
        }
        if let Some(n) = self.inner.next() {
            if n != 0xFF {
                return Some(n);
            }
            let count = self.inner.next()?;
            self.esc_out = if count & 0x80 == 0 { 0 } else { 0xFF };
            self.remaining = (count & 0x7F) - 1;
            Some(self.esc_out)
        } else {
            None
        }
    }
}

#[derive(Clone)]
pub struct CHRPHeader<'a> {
    pub name: &'a [u8],
    pub size: u16,
    pub signature: u8,
}

impl Debug for CHRPHeader<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CHRPHeader")
            .field("name", &String::from_utf8_lossy(self.name).into_owned())
            .field("size", &self.size)
            .field("signature", &self.signature)
            .finish()
    }
}

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

impl CHRPHeader<'_> {
    pub fn parse(nvr: &[u8]) -> Result<CHRPHeader<'_>> {
        let signature = nvr[0];
        let cksum = nvr[1];
        let size = u16::from_le_bytes(nvr[2..4].try_into().unwrap());
        let name = slice_rstrip(&nvr[4..16], &0);
        let cand = CHRPHeader {
            name,
            size,
            signature,
        };
        if cand.checksum() != cksum {
            return Err(Error::ParseError);
        }
        Ok(cand)
    }
    fn checksum(&self) -> u8 {
        let mut cksum = 0;
        for &u in self.name {
            cksum = chrp_checksum_add(cksum, u);
        }
        cksum = chrp_checksum_add(cksum, self.signature);
        cksum = chrp_checksum_add(cksum, (self.size & 0xFF) as u8);
        chrp_checksum_add(cksum, (self.size >> 8) as u8)
    }

    pub fn serialize(&self, v: &mut Vec<u8>) {
        v.push(self.signature);
        v.push(self.checksum());
        v.extend_from_slice(&self.size.to_le_bytes());
        v.extend_from_slice(self.name);
        for _ in 0..(12 - self.name.len()) {
            v.push(0);
        }
    }
}

#[derive(Clone)]
pub struct Variable<'a> {
    pub key: &'a [u8],
    pub value: Cow<'a, [u8]>,
    pub typ: VarType,
}

impl Variable<'_> {
    pub fn new<'a>(key: &'a [u8], value: &'a [u8], typ: VarType) -> Variable<'a> {
        Variable {
            key,
            value: Cow::Borrowed(value),
            typ,
        }
    }
}

impl<'a> fmt::Display for Variable<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let key = String::from_utf8_lossy(self.key);
        let mut value = String::new();
        for c in UnescapeVal::new(self.value.iter().copied()) {
            if (c as char).is_ascii() && !(c as char).is_ascii_control() {
                value.push(c as char);
            } else {
                value.push_str(&format!("%{c:02x}"));
            }
        }

        let value: String = value.chars().take(128).collect();
        write!(f, "{}:{}={}", self.typ, key, value)
    }
}

#[derive(Clone)]
pub struct Section<'a> {
    pub header: CHRPHeader<'a>,
    pub values: HashMap<&'a [u8], Variable<'a>>,
}

impl Section<'_> {
    pub fn parse(mut nvr: &[u8]) -> Result<Section<'_>> {
        let header = CHRPHeader::parse(&nvr[..16])?;
        nvr = &nvr[16..];
        let mut values = HashMap::new();
        loop {
            let zero = slice_find(nvr, &0);
            if zero.is_none() {
                break;
            }
            let zero = zero.unwrap();
            let cand = &nvr[..zero];
            let eq = slice_find(cand, &b'=');
            if eq.is_none() {
                break;
            }
            let eq = eq.unwrap();
            let key = &cand[..eq];
            let typ = if header.name == b"common" {
                VarType::Common
            } else {
                VarType::System
            };
            values.insert(key, Variable::new(key, &cand[(eq + 1)..], typ));
            nvr = &nvr[(zero + 1)..]
        }
        Ok(Section { header, values })
    }
    fn size_bytes(&self) -> usize {
        self.header.size as usize * 16
    }
    pub fn serialize(&self, v: &mut Vec<u8>) -> Result<()> {
        let start_size = v.len();
        self.header.serialize(v);
        for val in self.values.values() {
            v.extend_from_slice(val.key);
            v.push(b'=');
            v.extend_from_slice(&val.value);
            v.push(0);
        }
        let my_size = v.len() - start_size;
        if my_size > self.size_bytes() {
            return Err(Error::SectionTooBig);
        }
        for _ in 0..(self.size_bytes() - my_size) {
            v.push(0);
        }
        Ok(())
    }
}

struct SectionDebug<'a, 'b>(&'a Section<'b>);
impl Debug for SectionDebug<'_, '_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut m = f.debug_map();
        for v in self.0.values.values() {
            m.entry(
                &String::from_utf8_lossy(v.key).into_owned(),
                &String::from_utf8_lossy(
                    &UnescapeVal::new(v.value.iter().copied()).collect::<Vec<_>>(),
                )
                .into_owned(),
            );
        }
        m.finish()
    }
}

impl Debug for Section<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Section")
            .field("header", &self.header)
            .field("values", &SectionDebug(self))
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct Partition<'a> {
    pub header: CHRPHeader<'a>,
    pub generation: u32,
    pub common: Section<'a>,
    pub system: Section<'a>,
}

impl Partition<'_> {
    pub fn parse(nvr: &[u8]) -> Result<Partition<'_>> {
        let header = CHRPHeader::parse(&nvr[..16])?;
        if header.name != b"nvram" {
            return Err(Error::ParseError);
        }
        let adler = u32::from_le_bytes(nvr[16..20].try_into().unwrap());
        let generation = u32::from_le_bytes(nvr[20..24].try_into().unwrap());
        let sec1 = Section::parse(&nvr[32..])?;
        let sec2 = Section::parse(&nvr[(32 + sec1.size_bytes())..])?;
        let calc_adler =
            adler32::adler32(&nvr[20..(32 + sec1.size_bytes() + sec2.size_bytes())]).unwrap();
        if adler != calc_adler {
            return Err(Error::ParseError);
        }
        let mut com = None;
        let mut sys = None;
        if sec1.header.name == b"common" {
            com = Some(sec1);
        } else if sec1.header.name == b"system" {
            sys = Some(sec1);
        }
        if sec2.header.name == b"common" {
            com = Some(sec2);
        } else if sec2.header.name == b"system" {
            sys = Some(sec2);
        }
        if com.is_none() || sys.is_none() {
            return Err(Error::ParseError);
        }
        Ok(Partition {
            header,
            generation,
            common: com.unwrap(),
            system: sys.unwrap(),
        })
    }
    fn size_bytes(&self) -> usize {
        32 + self.common.size_bytes() + self.system.size_bytes()
    }
    pub fn serialize(&self, v: &mut Vec<u8>) -> Result<()> {
        self.header.serialize(v);
        v.extend_from_slice(&[0; 4]);
        let adler_start = v.len();
        v.extend_from_slice(&self.generation.to_le_bytes());
        v.extend_from_slice(&[0; 8]);
        self.common.serialize(v)?;
        self.system.serialize(v)?;
        let adler_end = v.len();
        let adler = adler32::adler32(&v[adler_start..adler_end]).unwrap();
        v[(adler_start - 4)..adler_start].copy_from_slice(&adler.to_le_bytes());
        Ok(())
    }
}

#[derive(Debug)]
pub struct Nvram<'a> {
    pub partitions: [Partition<'a>; 2],
    pub active: usize,
}

impl<'a> Nvram<'a> {
    pub fn parse(nvr: &[u8]) -> Result<Nvram<'_>> {
        let p1;
        let p2;
        match (Partition::parse(nvr), Partition::parse(&nvr[0x10000..])) {
            (Err(err), Err(_)) => return Err(err),
            (Ok(p1r), Err(_)) => {
                p1 = p1r;
                p2 = p1.clone();
            }
            (Err(_), Ok(p2r)) => {
                p2 = p2r;
                p1 = p2.clone();
            }
            (Ok(p1r), Ok(p2r)) => {
                p1 = p1r;
                p2 = p2r;
            }
        }
        let active = if p1.generation > p2.generation { 0 } else { 1 };
        let partitions = [p1, p2];
        Ok(Nvram { partitions, active })
    }
    pub fn serialize(&self) -> Result<Vec<u8>> {
        let mut v = Vec::with_capacity(self.partitions[0].size_bytes() * 2);
        self.partitions[0].serialize(&mut v)?;
        self.partitions[1].serialize(&mut v)?;
        Ok(v)
    }
    pub fn prepare_for_write(&mut self) {
        let inactive = 1 - self.active;
        self.partitions[inactive] = self.partitions[self.active].clone();
        self.partitions[inactive].generation += 1;
        self.active = inactive;
    }
    pub fn active_part_mut(&mut self) -> &mut Partition<'a> {
        &mut self.partitions[self.active]
    }
}

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

#[derive(Clone)]
pub enum VarType {
    Common, System
}

impl fmt::Display for VarType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            &VarType::Common => write!(f, "common"),
            &VarType::System => write!(f, "system"),
        }
    }
}
