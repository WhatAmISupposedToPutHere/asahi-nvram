use std::{
    borrow::Cow,
    fmt::{Display, Formatter},
};

use crate::{Error, Result, VarType};

// https://github.com/apple-oss-distributions/xnu/blob/main/iokit/Kernel/IONVRAMV3Handler.cpp#L630

const VARIABLE_STORE_SIGNATURE: &[u8; 4] = b"3VVN";
const VARIABLE_STORE_VERSION: u8 = 0x1;
const VARIABLE_DATA: u16 = 0x55AA;

const STORE_HEADER_SIZE: usize = 24;
const VAR_HEADER_SIZE: usize = 36;
const VAR_ADDED: u8 = 0x7F;

const APPLE_COMMON_VARIABLE_GUID: &[u8; 16] = &[
    0x7C, 0x43, 0x61, 0x10, 0xAB, 0x2A, 0x4B, 0xBB, 0xA8, 0x80, 0xFE, 0x41, 0x99, 0x5C, 0x9F, 0x82,
];
const APPLE_SYSTEM_VARIABLE_GUID: &[u8; 16] = &[
    0x40, 0xA0, 0xDD, 0xD2, 0x77, 0xF8, 0x43, 0x92, 0xB4, 0xA3, 0x1E, 0x73, 0x04, 0x20, 0x65, 0x16,
];

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
            match Partition::parse(&nvr[offset..offset + 0x10000]) {
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

    fn partitions(&self) -> impl Iterator<Item = &Partition<'a>> {
        self.partitions.iter().filter_map(|x| x.as_ref())
    }

    pub fn active_part(&self) -> &Partition<'a> {
        self.partitions[self.active].as_ref().unwrap()
    }
}

impl<'a> crate::Nvram<'a> for Nvram<'a> {
    fn serialize(&self) -> Result<Vec<u8>> {
        let mut v = Vec::with_capacity(16 * 0x10000);
        for p in self.partitions() {
            p.serialize(&mut v);
        }
        Ok(v)
    }

    fn prepare_for_write(&mut self) {
        // nop
    }

    fn partitions(&self) -> Box<dyn Iterator<Item = &dyn crate::Partition<'a>> + '_> {
        Box::new(self.partitions().map(|p| p as &dyn crate::Partition<'a>))
    }

    fn active_part_mut(&mut self) -> &mut dyn crate::Partition<'a> {
        self.partitions[self.active].as_mut().unwrap()
    }

    fn apply(&self, w: &mut dyn crate::NvramWriter) -> Result<()> {
        let mut data = Vec::with_capacity(0x10000);
        self.active_part().serialize(&mut data);

        w.write_all(self.active as u32 * 0x10000, &data)
            .map_err(|e| Error::ApplyError(e))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Partition<'a> {
    pub header: StoreHeader<'a>,
    pub values: Vec<(&'a [u8], Variable<'a>)>,
    pub raw_data: &'a [u8],
}

impl<'a> Partition<'a> {
    pub fn parse(nvr: &[u8]) -> Result<Partition<'_>> {
        let header = StoreHeader::parse(&nvr[..STORE_HEADER_SIZE])?;
        let mut offset = STORE_HEADER_SIZE;
        let mut values = Vec::new();

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
                        return Err(Error::ParseError);
                    }
                    let v = Variable {
                        header: v_header,
                        key,
                        value: Cow::Borrowed(value),
                    };

                    offset += v.size();
                    // println!("DEBUG 0x{:04x} {}", offset, &v);
                    // if v.header.state == VAR_ADDED {
                    values.push((key, v));
                    // }
                }
                _ => {
                    offset += VAR_HEADER_SIZE;
                }
            }
        }

        Ok(Partition {
            header,
            values,
            raw_data: nvr,
        })
    }

    fn generation(&self) -> u32 {
        self.header.generation
    }

    fn entry_or_default(&mut self, key: &'a [u8]) -> &mut Variable<'a> {
        let idx = self
            .values
            .iter_mut()
            .position(|e| e.0 == key && e.1.header.state == VAR_ADDED);

        match idx {
            Some(idx) => self.values.get_mut(idx).map(|e| &mut e.1).unwrap(),
            None => {
                self.values.push((key, Variable::default()));
                self.values.last_mut().map(|e| &mut e.1).unwrap()
            }
        }
    }

    fn serialize(&self, v: &mut Vec<u8>) {
        let start_size = v.len();
        self.header.serialize(v);
        for var in self.variables() {
            var.serialize(v);
        }
        let my_size = v.len() - start_size;

        // padding
        for _ in 0..(self.header.size() - my_size) {
            v.push(0xFF);
        }
    }

    fn variables(&self) -> impl Iterator<Item = &Variable<'a>> {
        self.values.iter().map(|e| &e.1)
    }
}

impl<'a> crate::Partition<'a> for Partition<'a> {
    fn get_variable(&self, key: &[u8]) -> Option<&dyn crate::Variable<'a>> {
        self.values.iter().find_map(|e| {
            if e.0 == key && e.1.header.state == VAR_ADDED {
                Some(&e.1 as &dyn crate::Variable<'a>)
            } else {
                None
            }
        })
    }

    fn insert_variable(&mut self, key: &'a [u8], value: Cow<'a, [u8]>, _typ: VarType) {
        let var = self.entry_or_default(key);

        var.header.state = VAR_ADDED;
        var.header.name_size = (key.len() + 1) as u32;
        var.header.data_size = value.len() as u32;
        var.header.guid = match _typ {
            VarType::Common => APPLE_COMMON_VARIABLE_GUID,
            VarType::System => APPLE_SYSTEM_VARIABLE_GUID,
        };
        var.header.crc = crc32fast::hash(&value);

        var.key = key;
        var.value = value;
    }

    fn remove_variable(&mut self, key: &'a [u8], _typ: VarType) {
        let idx = self
            .values
            .iter()
            .position(|e| e.0 == key && e.1.header.state == VAR_ADDED);
        if let Some(idx) = idx {
            self.values.remove(idx);
        }
    }

    fn variables(&self) -> Box<dyn Iterator<Item = &dyn crate::Variable<'a>> + '_> {
        Box::new(self.variables().map(|e| e as &dyn crate::Variable<'a>))
    }
}

impl Display for Partition<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "size: {}, generation: {}, state: 0x{:02x}, flags: 0x{:02x}, count: {}",
            self.header.size,
            self.generation(),
            self.header.state,
            self.header.flags,
            self.values.len()
        )
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
            name,
            size,
            generation,
            state,
            flags,
            version,
            system_size,
            common_size,
        })
    }

    fn serialize(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(VARIABLE_STORE_SIGNATURE);
        v.extend_from_slice(&self.size.to_le_bytes());
        v.extend_from_slice(&self.generation.to_le_bytes());
        v.push(self.state);
        v.push(self.flags);
        v.push(self.version);
        v.push(0); // reserved
        v.extend_from_slice(&self.system_size.to_le_bytes());
        v.extend_from_slice(&self.common_size.to_le_bytes());
    }

    pub fn size(&self) -> usize {
        self.size as usize
    }
}

#[derive(Debug, Default, Clone)]
pub struct Variable<'a> {
    pub header: VarHeader<'a>,
    pub key: &'a [u8],
    pub value: Cow<'a, [u8]>,
}

impl<'a> Variable<'a> {
    pub fn size(&self) -> usize {
        VAR_HEADER_SIZE + (self.header.name_size + self.header.data_size) as usize
    }

    pub fn typ(&self) -> VarType {
        if self.header.guid == APPLE_SYSTEM_VARIABLE_GUID {
            return VarType::System;
        }
        VarType::Common
    }

    fn serialize(&self, v: &mut Vec<u8>) {
        self.header.serialize(v);
        v.extend_from_slice(self.key);
        v.push(0);
        v.extend_from_slice(&self.value);
    }
}

impl<'a> crate::Variable<'a> for Variable<'a> {
    fn value(&self) -> Cow<'a, [u8]> {
        self.value.clone()
    }
}

impl Display for Variable<'_> {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
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
        write!(
            f,
            "(s:0x{:02x}) {}:{}={}",
            self.header.state,
            self.typ(),
            key,
            value
        )
    }
}

#[derive(Debug, Default, Clone)]
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

        Ok(VarHeader {
            state,
            attrs,
            name_size,
            data_size,
            guid,
            crc,
        })
    }

    pub fn serialize(&self, v: &mut Vec<u8>) {
        v.extend_from_slice(&VARIABLE_DATA.to_le_bytes());
        v.push(self.state);
        v.push(0); // reserved
        v.extend_from_slice(&self.attrs.to_le_bytes());
        v.extend_from_slice(&self.name_size.to_le_bytes());
        v.extend_from_slice(&self.data_size.to_le_bytes());
        v.extend_from_slice(self.guid);
        v.extend_from_slice(&self.crc.to_le_bytes());
    }
}
