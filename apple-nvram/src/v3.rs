use std::{borrow::Cow, fmt};
use indexmap::IndexMap;

use super::{VarType, Result, Error};

// https://github.com/apple-oss-distributions/xnu/blob/main/iokit/Kernel/IONVRAMV3Handler.cpp#L630

const VARIABLE_STORE_SIGNATURE: &[u8; 4] = b"3VVN";
const VARIABLE_STORE_VERSION: u8 = 0x1;
const VARIABLE_DATA: u16 = 0x55AA;

const STORE_HEADER_SIZE: usize = 24;
const VAR_HEADER_SIZE: usize = 36;
const VAR_ADDED: u8 = 0x7F;

const APPLE_SYSTEM_VARIABLE_GUID: &[u8; 16] = &[0x40, 0xA0, 0xDD, 0xD2, 0x77, 0xF8, 0x43, 0x92, 0xB4, 0xA3, 0x1E, 0x73, 0x04, 0x20, 0x65, 0x16];

#[derive(Debug)]
pub struct Nvram<'a> {
    pub partitions: [Option<Partition<'a>>; 16],
    pub active: usize,
}

impl<'a> Nvram<'a> {
    pub fn parse(nvr: &'a [u8]) -> Result<Nvram<'_>> {
        let mut partitions: [Option<Partition<'a>>; 16] = Default::default();
        let mut active = 0;
        let mut max_gen = 0;
        let mut valid_partitions = 0;

        for i in 0..16 {
            let offset = i * 0x10000;
            if offset >= nvr.len() {
                break;
            }
            match Partition::parse(&nvr[offset..]) {
                Ok(p) => {
                    let p_gen = p.generation();
                    if p_gen > max_gen {
                        active = i;
                        max_gen = p_gen;
                    }
                    partitions[i] = Some(p);
                    valid_partitions += 1;
                }
                Err(_e) => {}
            }
        }

        if valid_partitions == 0 {
            return Err(Error::ParseError);
        }
        Ok(Nvram { partitions, active })
    }

    pub fn partitions(&self) -> impl Iterator<Item=&Partition<'_>> {
        self.partitions.iter().filter_map(|x| x.as_ref())
    }

    pub fn active_part(&self) -> &Partition<'a> {
        self.partitions[self.active].as_ref().unwrap()
    }
    pub fn active_part_mut(&mut self) -> &mut Partition<'a> {
        self.partitions[self.active].as_mut().unwrap()
    }
}

#[derive(Debug, Clone)]
pub struct Partition<'a> {
    pub header: StoreHeader<'a>,
    pub values: IndexMap<&'a [u8], Variable<'a>>,
}

impl<'a> Partition<'a> {
    pub fn parse(nvr: &[u8]) -> Result<Partition<'_>> {
        let header = StoreHeader::parse(&nvr[..STORE_HEADER_SIZE])?;
        let mut offset = STORE_HEADER_SIZE;
        let mut values = IndexMap::new();

        while offset + VAR_HEADER_SIZE < header.size() {
            // let mut empty = true;
            // for i in 0..VAR_HEADER_SIZE {
            //     if nvr[offset + i] != 0 && nvr[offset + i] != 0xFF {
            //         empty = false;
            //         break;
            //     }
            // }
            // if empty {
            //     println!("DEBUG: stopped at offset 0x{:04x}", offset);
            //     break
            // }

            match VarHeader::parse(&nvr[offset..]) {
                Ok(v_header) => {
                    let k_begin = offset + VAR_HEADER_SIZE;
                    let k_end = k_begin + v_header.name_size as usize;
                    let key = &nvr[k_begin..k_end - 1];

                    let v_begin = k_end;
                    let v_end = v_begin + v_header.data_size as usize;
                    let value = &nvr[v_begin..v_end];

                    let crc = crc32fast::hash(value);
                    if crc != v_header.crc {
                        return Err(Error::ParseError)
                    }
                    let v = Variable {
                        header: v_header, key,
                        value: Cow::Borrowed(value),
                    };

                    offset += v.size();
                    // println!("DEBUG 0x{:04x} {}", offset, &v);
                    if v.header.state == VAR_ADDED {
                        values.insert(key, v);
                    }
                }
                _ => {
                    offset += VAR_HEADER_SIZE;
                }
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
        let mut value = String::new();
        for c in self.value.iter().copied() {
            if (c as char).is_ascii() && !(c as char).is_ascii_control() {
                value.push(c as char);
            } else {
                value.push_str(&format!("%{c:02x}"));
            }
        }

        let value: String = value.chars().take(128).collect();
        write!(f, "{}:{}={} (state:0x{:02x})",
            self.typ(), key, value, self.header.state)
    }
}

#[derive(Debug, Clone)]
pub struct VarHeader<'a> {
    pub state: u8,
    pub attrs: u32,
    pub name_size: u32,
    pub data_size: u32,
    pub guid: &'a [u8],
    pub crc: u32,
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
        let crc = u32::from_le_bytes(nvr[32..36].try_into().unwrap());

        if VAR_HEADER_SIZE + (name_size + data_size) as usize > nvr.len() {
            return Err(Error::ParseError);
        }

        Ok(VarHeader { state, attrs, name_size, data_size, guid, crc })
    }
}
