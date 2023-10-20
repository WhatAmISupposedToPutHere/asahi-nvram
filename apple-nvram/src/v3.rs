use std::{
    borrow::Cow,
    fmt::{Display, Formatter},
    ops::ControlFlow,
};

use crate::{Error, VarType};

// https://github.com/apple-oss-distributions/xnu/blob/main/iokit/Kernel/IONVRAMV3Handler.cpp#L630

const VARIABLE_STORE_SIGNATURE: &[u8; 4] = b"3VVN";
const VARIABLE_STORE_VERSION: u8 = 0x1;
const VARIABLE_DATA: u16 = 0x55AA;

const PARTITION_SIZE: usize = 0x10000;
const STORE_HEADER_SIZE: usize = 24;
const VAR_HEADER_SIZE: usize = 36;
const VAR_ADDED: u8 = 0x7F;
const VAR_IN_DELETED_TRANSITION: u8 = 0xFE;
const VAR_DELETED: u8 = 0xFD;

const APPLE_COMMON_VARIABLE_GUID: &[u8; 16] = &[
    0x7C, 0x43, 0x61, 0x10, 0xAB, 0x2A, 0x4B, 0xBB, 0xA8, 0x80, 0xFE, 0x41, 0x99, 0x5C, 0x9F, 0x82,
];
const APPLE_SYSTEM_VARIABLE_GUID: &[u8; 16] = &[
    0x40, 0xA0, 0xDD, 0xD2, 0x77, 0xF8, 0x43, 0x92, 0xB4, 0xA3, 0x1E, 0x73, 0x04, 0x20, 0x65, 0x16,
];

#[derive(Debug, Default)]
enum Slot<T> {
    Valid(T),
    Invalid,
    #[default]
    Empty,
}

impl<T> Slot<T> {
    pub const fn as_ref(&self) -> Slot<&T> {
        match self {
            Slot::Valid(v) => Slot::Valid(v),
            Slot::Invalid => Slot::Invalid,
            Slot::Empty => Slot::Empty,
        }
    }

    pub fn as_mut(&mut self) -> Slot<&mut T> {
        match self {
            Slot::Valid(v) => Slot::Valid(v),
            Slot::Invalid => Slot::Invalid,
            Slot::Empty => Slot::Empty,
        }
    }

    pub fn unwrap(self) -> T {
        match self {
            Slot::Valid(v) => v,
            Slot::Invalid => panic!("called `Slot::unwrap()` on an `Invalid` value"),
            Slot::Empty => panic!("called `Slot::unwrap()` on an `Empty` value"),
        }
    }

    pub fn empty(&self) -> bool {
        match self {
            Slot::Empty => true,
            _ => false,
        }
    }
}

#[derive(Debug)]
pub struct Nvram<'a> {
    partitions: [Slot<Partition<'a>>; 16],
    partition_count: usize,
    active: usize,
}

impl<'a> Nvram<'a> {
    pub fn parse(nvr: &'a [u8]) -> crate::Result<Nvram<'_>> {
        let partition_count = nvr.len() / PARTITION_SIZE;
        let mut partitions: [Slot<Partition<'a>>; 16] = Default::default();
        let mut active = 0;
        let mut max_gen = 0;
        let mut valid_partitions = 0;

        for i in 0..partition_count {
            let offset = i * PARTITION_SIZE;
            if offset >= nvr.len() {
                break;
            }
            match Partition::parse(&nvr[offset..offset + PARTITION_SIZE]) {
                Ok(p) => {
                    let p_gen = p.generation();
                    if p_gen > max_gen {
                        active = i;
                        max_gen = p_gen;
                    }
                    partitions[i] = Slot::Valid(p);
                    valid_partitions += 1;
                }
                Err(V3Error::Empty) => {
                    partitions[i] = Slot::Empty;
                }
                Err(_) => {
                    partitions[i] = Slot::Invalid;
                }
            }
        }

        if valid_partitions == 0 {
            return Err(Error::ParseError);
        }

        Ok(Nvram {
            partitions,
            partition_count,
            active,
        })
    }

    fn partitions(&self) -> impl Iterator<Item = &Partition<'a>> {
        self.partitions
            .iter()
            .take(self.partition_count)
            .filter_map(|x| match x {
                Slot::Valid(p) => Some(p),
                Slot::Invalid => None,
                Slot::Empty => None,
            })
    }

    fn active_part(&self) -> &Partition<'a> {
        self.partitions[self.active].as_ref().unwrap()
    }
}

impl<'a> crate::Nvram<'a> for Nvram<'a> {
    fn serialize(&self) -> crate::Result<Vec<u8>> {
        let mut v = Vec::with_capacity(self.partition_count * PARTITION_SIZE);
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

    fn apply(&mut self, w: &mut dyn crate::NvramWriter) -> crate::Result<()> {
        let offset;
        // check total size before serializing
        // if it's too big, copy added variables to the next bank
        if self.active_part().total_size() <= PARTITION_SIZE {
            offset = (self.active * PARTITION_SIZE) as u32;
        } else {
            let new_active = (self.active + 1) % self.partition_count;
            offset = (new_active * PARTITION_SIZE) as u32;
            if !self.partitions[new_active].empty() {
                w.erase_if_needed(offset, PARTITION_SIZE);
            }
            // must only clone 0x7F variables to the next partition
            self.partitions[new_active] = Slot::Valid(
                self.partitions[self.active]
                    .as_ref()
                    .unwrap()
                    .clone_active(),
            );
            self.active = new_active;
            // we could still have too many active variables
            if self.active_part().total_size() > PARTITION_SIZE {
                return Err(Error::SectionTooBig);
            }
        }

        let mut data = Vec::with_capacity(PARTITION_SIZE);
        self.active_part().serialize(&mut data);
        w.write_all(offset, &data)
            .map_err(|e| Error::ApplyError(e))?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct Partition<'a> {
    pub header: StoreHeader<'a>,
    pub values: Vec<(&'a [u8], Variable<'a>)>,
}

enum V3Error {
    ParseError,
    Empty,
}

type Result<T> = std::result::Result<T, V3Error>;

impl<'a> Partition<'a> {
    fn parse(nvr: &'a [u8]) -> Result<Partition<'a>> {
        if let Ok(header) = StoreHeader::parse(&nvr[..STORE_HEADER_SIZE]) {
            let mut offset = STORE_HEADER_SIZE;
            let mut values = Vec::new();

            while offset + VAR_HEADER_SIZE < header.size() {
                let mut empty = true;
                for i in 0..VAR_HEADER_SIZE {
                    if nvr[offset + i] != 0 && nvr[offset + i] != 0xFF {
                        empty = false;
                        break;
                    }
                }
                if empty {
                    break;
                }

                let v_header = VarHeader::parse(&nvr[offset..])?;

                let k_begin = offset + VAR_HEADER_SIZE;
                let k_end = k_begin + v_header.name_size as usize;
                let key = &nvr[k_begin..k_end - 1];

                let v_begin = k_end;
                let v_end = v_begin + v_header.data_size as usize;
                let value = &nvr[v_begin..v_end];

                let crc = crc32fast::hash(value);
                if crc != v_header.crc {
                    return Err(V3Error::ParseError);
                }
                let v = Variable {
                    header: v_header,
                    key,
                    value: Cow::Borrowed(value),
                };

                offset += v.size();
                values.push((key, v));
            }

            Ok(Partition { header, values })
        } else {
            match nvr.iter().copied().try_for_each(|v| match v {
                0xFF => ControlFlow::Continue(()),
                _ => ControlFlow::Break(()),
            }) {
                ControlFlow::Continue(_) => Err(V3Error::Empty),
                ControlFlow::Break(_) => Err(V3Error::ParseError),
            }
        }
    }

    fn generation(&self) -> u32 {
        self.header.generation
    }

    fn entries(&mut self, key: &'a [u8], typ: VarType) -> impl Iterator<Item = &mut Variable<'a>> {
        self.values.iter_mut().filter_map(move |e| {
            if e.0 == key && e.1.typ() == typ && e.1.header.state == VAR_ADDED {
                Some(&mut e.1)
            } else {
                None
            }
        })
    }

    fn total_size(&self) -> usize {
        STORE_HEADER_SIZE + self.values.iter().fold(0, |acc, v| acc + v.1.size())
    }

    fn serialize(&self, v: &mut Vec<u8>) {
        let start_size = v.len();
        self.header.serialize(v);
        for var in self.variables() {
            var.serialize(v);
        }
        let my_size = v.len() - start_size;
        debug_assert!(v.len() == self.total_size());

        // padding
        for _ in 0..(self.header.size() - my_size) {
            v.push(0xFF);
        }
    }

    fn variables(&self) -> impl Iterator<Item = &Variable<'a>> {
        self.values.iter().map(|e| &e.1)
    }

    fn clone_active(&self) -> Partition<'a> {
        let mut header = self.header.clone();
        header.generation += 1;
        Partition {
            header,
            values: self
                .values
                .iter()
                .filter_map(|v| {
                    if v.1.header.state == VAR_ADDED {
                        Some(v.clone())
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }
}

impl<'a> crate::Partition<'a> for Partition<'a> {
    fn get_variable(&self, key: &[u8], typ: VarType) -> Option<&dyn crate::Variable<'a>> {
        self.values.iter().find_map(|e| {
            if e.0 == key && e.1.typ() == typ && e.1.header.state == VAR_ADDED {
                Some(&e.1 as &dyn crate::Variable<'a>)
            } else {
                None
            }
        })
    }

    fn insert_variable(&mut self, key: &'a [u8], value: Cow<'a, [u8]>, typ: VarType) {
        // invalidate any previous variable instances
        for var in self.entries(key, typ) {
            var.header.state = var.header.state & VAR_DELETED & VAR_IN_DELETED_TRANSITION;
        }

        let guid = match typ {
            VarType::Common => APPLE_COMMON_VARIABLE_GUID,
            VarType::System => APPLE_SYSTEM_VARIABLE_GUID,
        };
        let var = Variable {
            header: VarHeader {
                state: VAR_ADDED,
                attrs: 0,
                name_size: (key.len() + 1) as u32,
                data_size: value.len() as u32,
                guid,
                crc: crc32fast::hash(&value),
            },
            key,
            value,
        };
        self.values.push((key, var));
    }

    fn remove_variable(&mut self, key: &'a [u8], typ: VarType) {
        // invalidate all previous variable instances
        for var in self.entries(key, typ) {
            var.header.state = var.header.state & VAR_DELETED & VAR_IN_DELETED_TRANSITION;
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
    fn parse(nvr: &[u8]) -> Result<StoreHeader<'_>> {
        let name = &nvr[..4];
        let size = u32::from_le_bytes(nvr[4..8].try_into().unwrap());
        let generation = u32::from_le_bytes(nvr[8..12].try_into().unwrap());
        let state = nvr[12];
        let flags = nvr[13];
        let version = nvr[14];
        let system_size = u32::from_le_bytes(nvr[16..20].try_into().unwrap());
        let common_size = u32::from_le_bytes(nvr[20..24].try_into().unwrap());

        if name != VARIABLE_STORE_SIGNATURE {
            return Err(V3Error::ParseError);
        }
        if version != VARIABLE_STORE_VERSION {
            return Err(V3Error::ParseError);
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

    fn size(&self) -> usize {
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
    fn size(&self) -> usize {
        VAR_HEADER_SIZE + (self.header.name_size + self.header.data_size) as usize
    }

    fn typ(&self) -> VarType {
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
    fn parse(nvr: &[u8]) -> Result<VarHeader<'_>> {
        let start_id = u16::from_le_bytes(nvr[..2].try_into().unwrap());
        if start_id != VARIABLE_DATA {
            return Err(V3Error::ParseError);
        }
        let state = nvr[2];
        let attrs = u32::from_le_bytes(nvr[4..8].try_into().unwrap());
        let name_size = u32::from_le_bytes(nvr[8..12].try_into().unwrap());
        let data_size = u32::from_le_bytes(nvr[12..16].try_into().unwrap());
        let guid = &nvr[16..32];
        let crc = u32::from_le_bytes(nvr[32..36].try_into().unwrap());

        if VAR_HEADER_SIZE + (name_size + data_size) as usize > nvr.len() {
            return Err(V3Error::ParseError);
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

    fn serialize(&self, v: &mut Vec<u8>) {
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
