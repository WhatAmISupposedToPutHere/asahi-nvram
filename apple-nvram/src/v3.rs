use std::{borrow::Cow, collections::HashMap, fmt};
use super::{Result, Error};

// https://github.com/apple-oss-distributions/xnu/blob/main/iokit/Kernel/IONVRAMV3Handler.cpp#L630

const VARIABLE_STORE_SIGNATURE: &[u8; 4] = b"3VVN";
const VARIABLE_STORE_VERSION: u8 = 0x1;
const VARIABLE_DATA: u16 = 0x55AA;

const STORE_HEADER_SIZE: usize = 24;
const VAR_HEADER_SIZE: usize = 36;

const APPLE_SYSTEM_VARIABLE_GUID: &[u8; 16] = &[0x40, 0xA0, 0xDD, 0xD2, 0x77, 0xF8, 0x43, 0x92, 0xB4, 0xA3, 0x1E, 0x73, 0x04, 0x20, 0x65, 0x16];

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
        let active = if p1.generation() > p2.generation() { 0 } else { 1 };
        let partitions = [p1, p2];
        Ok(Nvram { partitions, active })
    }

    pub fn active_part_mut(&mut self) -> &mut Partition<'a> {
        &mut self.partitions[self.active]
    }
}

#[derive(Debug, Clone)]
pub struct Partition<'a> {
    pub header: StoreHeader<'a>,
    pub values: HashMap<&'a [u8], Variable<'a>>,
}

impl<'a> Partition<'a> {
    pub fn parse(nvr: &[u8]) -> Result<Partition<'_>> {
        let header = StoreHeader::parse(&nvr[..STORE_HEADER_SIZE])?;
        let mut offset = STORE_HEADER_SIZE;
        let mut values = HashMap::new();

        while offset + VAR_HEADER_SIZE < header.size() {
            let mut empty = true;
            for i in 0..VAR_HEADER_SIZE {
                if nvr[offset + i] != 0 && nvr[offset + i] != 0xFF {
                    empty = false;
                    break;
                }
            }
            if empty {
                break
            }

            if let Ok(v_header) = VarHeader::parse(&nvr[offset..]) {
                let k_begin = offset + VAR_HEADER_SIZE;
                let k_end = k_begin + v_header.name_size as usize;
                let key = &nvr[k_begin..k_end - 1];

                let v_begin = k_end;
                let v_end = k_end + v_header.data_size as usize;
                let value = &nvr[v_begin..v_end];
                let v = Variable {
                    header: v_header, key,
                    value: Cow::Borrowed(value),
                };

                offset += v.size();
                values.insert(key, v);
            } else {
                offset += VAR_HEADER_SIZE;
            }
        }

        Ok(Partition { header, values })
    }

    pub fn generation(&self) -> u32 {
        self.header.generation
    }
}

#[derive(Debug, Clone)]
pub struct StoreHeader<'a> {
    pub name: &'a [u8],
    pub size: u32,
    pub generation: u32,
    pub state: u8,
    pub flags: u8,
    pub version: u8,
    pub system_size: u32,
    pub common_size: u32,
}

impl<'a> StoreHeader<'a> {
    pub fn parse(nvr: &[u8]) -> Result<StoreHeader<'_>> {
        let name = &nvr[..4];
        let size = u32::from_le_bytes(nvr[4..8].try_into().unwrap());
        let generation = u32::from_le_bytes(nvr[8..12].try_into().unwrap());
        let state = nvr[12];
        let flags = nvr[13];
        let version = nvr[14];
        let system_size = u32::from_le_bytes(nvr[16..20].try_into().unwrap());
        let common_size = u32::from_le_bytes(nvr[20..24].try_into().unwrap());

        if name != VARIABLE_STORE_SIGNATURE {
            return Err(Error::ParseError);
        }
        if version != VARIABLE_STORE_VERSION {
            return Err(Error::ParseError);
        }

        Ok(StoreHeader {
            name, size,
            generation, state,
            flags, version,
            system_size, common_size,
        })
    }

    pub fn size(&self) -> usize {
        self.size as usize
    }
}

#[derive(Debug, Clone)]
pub struct Variable<'a> {
    pub header: VarHeader<'a>,
    pub key: &'a [u8],
    pub value: Cow<'a, [u8]>,
}

impl<'a> Variable<'a> {
    pub fn size(&self) -> usize {
        (self.header.name_size + self.header.data_size) as usize
    }

    pub fn typ(&self) -> VarType {
        if self.header.guid == APPLE_SYSTEM_VARIABLE_GUID {
            return VarType::System;
        }
        VarType::Common
    }
}

impl<'a> fmt::Display for Variable<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let key = String::from_utf8_lossy(self.key);
        let value = String::from_utf8_lossy(&self.value);
        write!(f, "{}:{}={}", self.typ(), key, value)
    }
}

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

#[derive(Debug, Clone)]
pub struct VarHeader<'a> {
    pub state: u8,
    pub attrs: u32,
    pub name_size: u32,
    pub data_size: u32,
    pub guid: &'a [u8],
}

impl<'a> VarHeader<'a> {
    pub fn parse(nvr: &[u8]) -> Result<VarHeader<'_>> {
        let start_id = u16::from_le_bytes(nvr[..2].try_into().unwrap());
        if start_id != VARIABLE_DATA {
            return Err(Error::ParseError);
        }
        let state = nvr[2];
        let attrs = u32::from_le_bytes(nvr[4..8].try_into().unwrap());
        let name_size = u32::from_le_bytes(nvr[8..12].try_into().unwrap());
        let data_size = u32::from_le_bytes(nvr[12..16].try_into().unwrap());
        let guid = &nvr[16..32];
        let _crc = u32::from_le_bytes(nvr[32..36].try_into().unwrap());

        if VAR_HEADER_SIZE + (name_size + data_size) as usize > nvr.len() {
            return Err(Error::ParseError);
        }

        Ok(VarHeader { state, attrs, name_size, data_size, guid })
    }
}
