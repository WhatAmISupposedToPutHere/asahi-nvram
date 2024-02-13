// SPDX-License-Identifier: MIT
#![allow(dead_code)]
use apple_nvram::{mtd::MtdWriter, nvram_parse, VarType};
use gpt::{disk::LogicalBlockSize, GptConfig};
use std::{
    borrow::Cow,
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    ops::Deref,
};
use uuid::Uuid;

struct NxSuperblock([u8; NxSuperblock::SIZE]);

impl NxSuperblock {
    const SIZE: usize = 1408;
    const MAGIC: u32 = 1112758350; //'BSXN'
    const MAX_FILE_SYSTEMS: usize = 100;
    fn get_buf(&mut self) -> &mut [u8] {
        &mut self.0
    }
    fn new() -> Self {
        NxSuperblock([0; NxSuperblock::SIZE])
    }
    fn magic(&self) -> u32 {
        u32::from_le_bytes(self.0[32..32 + 4].try_into().unwrap())
    }
    fn block_size(&self) -> u32 {
        u32::from_le_bytes(self.0[36..36 + 4].try_into().unwrap())
    }
    fn xid(&self) -> u64 {
        u64::from_le_bytes(self.0[16..16 + 8].try_into().unwrap())
    }
    fn omap_oid(&self) -> u64 {
        u64::from_le_bytes(self.0[160..160 + 8].try_into().unwrap())
    }
    fn xp_desc_blocks(&self) -> u32 {
        u32::from_le_bytes(self.0[104..104 + 4].try_into().unwrap())
    }
    fn xp_desc_base(&self) -> u64 {
        u64::from_le_bytes(self.0[112..112 + 8].try_into().unwrap())
    }
    fn fs_oid(&self, i: usize) -> u64 {
        let at = 184 + 8 * i;
        u64::from_le_bytes(self.0[at..at + 8].try_into().unwrap())
    }
}

struct OmapPhys<'a>(&'a [u8]);
impl OmapPhys<'_> {
    const SIZE: usize = 88;
    fn tree_oid(&self) -> u64 {
        u64::from_le_bytes(self.0[48..48 + 8].try_into().unwrap())
    }
}

struct NLoc<'a>(&'a [u8]);

impl NLoc<'_> {
    fn off(&self) -> u16 {
        u16::from_le_bytes(self.0[0..2].try_into().unwrap())
    }
    fn len(&self) -> u16 {
        u16::from_le_bytes(self.0[2..2 + 2].try_into().unwrap())
    }
}

struct KVOff<'a>(&'a [u8]);
impl KVOff<'_> {
    const SIZE: usize = 4;
    fn k(&self) -> u16 {
        u16::from_le_bytes(self.0[0..2].try_into().unwrap())
    }
    fn v(&self) -> u16 {
        u16::from_le_bytes(self.0[2..2 + 2].try_into().unwrap())
    }
}

struct OmapKey<'a>(&'a [u8]);
impl OmapKey<'_> {
    fn oid(&self) -> u64 {
        u64::from_le_bytes(self.0[0..8].try_into().unwrap())
    }
    fn xid(&self) -> u64 {
        u64::from_le_bytes(self.0[8..8 + 8].try_into().unwrap())
    }
}

struct OmapVal<'a>(&'a [u8]);
impl OmapVal<'_> {
    fn flags(&self) -> u32 {
        u32::from_le_bytes(self.0[0..4].try_into().unwrap())
    }
    fn size(&self) -> u32 {
        u32::from_le_bytes(self.0[4..4 + 4].try_into().unwrap())
    }
    fn paddr(&self) -> u64 {
        u64::from_le_bytes(self.0[8..8 + 8].try_into().unwrap())
    }
}

struct BTreeInfo;
impl BTreeInfo {
    const SIZE: usize = 40;
}

struct BTreeNodePhys<'a>(&'a [u8]);
impl BTreeNodePhys<'_> {
    const FIXED_KV_SIZE: u16 = 0x4;
    const ROOT: u16 = 0x1;
    const SIZE: usize = 56;
    fn flags(&self) -> u16 {
        u16::from_le_bytes(self.0[32..32 + 2].try_into().unwrap())
    }
    fn level(&self) -> u16 {
        u16::from_le_bytes(self.0[34..34 + 2].try_into().unwrap())
    }
    fn table_space(&self) -> NLoc<'_> {
        NLoc(&self.0[40..])
    }
    fn nkeys(&self) -> u32 {
        u32::from_le_bytes(self.0[36..36 + 4].try_into().unwrap())
    }
}

struct ApfsSuperblock<'a>(&'a [u8]);
impl ApfsSuperblock<'_> {
    fn volname(&self) -> &[u8] {
        &self.0[704..704 + 128]
    }
    fn vol_uuid(&self) -> Uuid {
        Uuid::from_slice(&self.0[240..240 + 16]).unwrap()
    }
    fn volume_group_id(&self) -> Uuid {
        Uuid::from_slice(&self.0[1008..1008 + 16]).unwrap()
    }
}

fn pread<T: Read + Seek>(file: &mut T, pos: u64, target: &mut [u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(pos))?;
    file.read_exact(target)
}

// should probably fix xids here
fn lookup(_disk: &mut File, cur_node: &BTreeNodePhys, key: u64) -> Option<u64> {
    if cur_node.level() != 0 {
        unimplemented!();
    }
    if cur_node.flags() & BTreeNodePhys::FIXED_KV_SIZE != 0 {
        let toc_off = cur_node.table_space().off() as usize + BTreeNodePhys::SIZE;
        let key_start = toc_off + cur_node.table_space().len() as usize;
        let val_end = cur_node.0.len()
            - if cur_node.flags() & BTreeNodePhys::ROOT == 0 {
                0
            } else {
                BTreeInfo::SIZE
            };
        for i in 0..cur_node.nkeys() as usize {
            let entry = KVOff(&cur_node.0[(toc_off + i * KVOff::SIZE)..]);
            let key_off = entry.k() as usize + key_start;
            let map_key = OmapKey(&cur_node.0[key_off..]);
            if map_key.oid() == key {
                let val_off = val_end - entry.v() as usize;
                let val = OmapVal(&cur_node.0[val_off..]);
                return Some(val.paddr());
            }
        }
        None
    } else {
        unimplemented!();
    }
}

fn trim_zeroes(s: &[u8]) -> &[u8] {
    for i in 0..s.len() {
        if s[i] == 0 {
            return &s[..i];
        }
    }
    s
}

fn scan_volume(disk: &mut File) -> io::Result<HashMap<Uuid, Vec<String>>> {
    let mut sb = NxSuperblock::new();
    disk.read_exact(sb.get_buf())?;
    if sb.magic() != NxSuperblock::MAGIC {
        return Ok(HashMap::new());
    }
    let block_size = sb.block_size() as u64;
    for i in 0..sb.xp_desc_blocks() {
        let mut sbc = NxSuperblock::new();
        pread(disk, (sb.xp_desc_base() + i as u64) * block_size, sbc.get_buf())?;
        if sbc.magic() == NxSuperblock::MAGIC {
            if sbc.xid() > sb.xid() {
                sb = sbc;
            }
        }
    }
    let mut omap_bytes = vec![0; OmapPhys::SIZE];
    pread(disk, sb.omap_oid() * block_size, &mut omap_bytes)?;
    let omap = OmapPhys(&omap_bytes);
    let mut node_bytes = vec![0; sb.block_size() as usize];
    pread(disk, omap.tree_oid() * block_size, &mut node_bytes)?;
    let node = BTreeNodePhys(&node_bytes);
    let mut vgs_found = HashMap::<Uuid, Vec<String>>::new();
    for i in 0..NxSuperblock::MAX_FILE_SYSTEMS {
        let fs_id = sb.fs_oid(i);
        if fs_id == 0 {
            break;
        }
        let vsb = lookup(disk, &node, fs_id);
        let mut asb_bytes = vec![0; sb.block_size() as usize];
        if vsb.is_none() {
            continue;
        }
        pread(disk, vsb.unwrap() * sb.block_size() as u64, &mut asb_bytes)?;
        let asb = ApfsSuperblock(&asb_bytes);
        if asb.volume_group_id().is_nil() {
            continue;
        }
        if let Ok(name) = std::str::from_utf8(trim_zeroes(asb.volname())) {
            vgs_found
                .entry(asb.volume_group_id())
                .or_default()
                .push(name.to_owned());
        }
    }
    Ok(vgs_found)
}

#[derive(Debug)]
pub struct BootCandidate {
    pub part_uuid: Uuid,
    pub vg_uuid: Uuid,
    pub vol_names: Vec<String>,
}

fn swap_uuid(u: &Uuid) -> Uuid {
    let (a, b, c, d) = u.as_fields();
    Uuid::from_fields(a.swap_bytes(), b.swap_bytes(), c.swap_bytes(), d)
}

#[derive(Debug)]
pub enum Error {
    Parse,
    SectionTooBig,
    ApplyError(std::io::Error),
    OutOfRange,
    Ambiguous,
    NvramReadError(std::io::Error),
    DiskReadError(std::io::Error),
}

impl From<apple_nvram::Error> for Error {
    fn from(e: apple_nvram::Error) -> Self {
        match e {
            apple_nvram::Error::ParseError => Error::Parse,
            apple_nvram::Error::SectionTooBig => Error::SectionTooBig,
            apple_nvram::Error::ApplyError(e) => Error::ApplyError(e),
        }
    }
}

type Result<T> = std::result::Result<T, Error>;

pub fn get_boot_candidates() -> Result<Vec<BootCandidate>> {
    let disk = GptConfig::new()
        .writable(false)
        .logical_block_size(LogicalBlockSize::Lb4096)
        .open("/dev/nvme0n1")
        .map_err(Error::DiskReadError)?;
    let mut cands = Vec::new();
    for (i, v) in disk.partitions() {
        if v.part_type_guid.guid != "7C3457EF-0000-11AA-AA11-00306543ECAC" {
            continue;
        }
        let mut part = File::open(format!("/dev/nvme0n1p{i}")).map_err(Error::DiskReadError)?;
        for (vg_uuid, vol_names) in scan_volume(&mut part).unwrap_or_default() {
            cands.push(BootCandidate {
                vg_uuid,
                vol_names,
                part_uuid: swap_uuid(&v.part_guid),
            });
        }
    }

    return Ok(cands);
}

pub fn get_boot_volume(next: bool) -> Result<BootCandidate> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/mtd0")
        .map_err(Error::NvramReadError)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).map_err(Error::NvramReadError)?;
    let mut nv = nvram_parse(&data)?;

    let active = nv.active_part_mut();
    let v;
    if next {
        v = active
            .get_variable(b"alt-boot-volume", VarType::System)
            .or(active.get_variable(b"boot-volume", VarType::System))
            .ok_or(Error::Parse);
    } else {
        v = active
            .get_variable(b"boot-volume", VarType::System)
            .ok_or(Error::Parse);
    }
    let data = String::from_utf8(v?.value().deref().to_vec()).unwrap();
    let [_, part_uuid, part_vg_uuid]: [&str; 3] =
        data.split(":").collect::<Vec<&str>>().try_into().unwrap();

    Ok(BootCandidate {
        vol_names: Vec::new(),
        part_uuid: Uuid::parse_str(part_uuid).unwrap(),
        vg_uuid: Uuid::parse_str(part_vg_uuid).unwrap(),
    })
}

pub fn set_boot_volume(cand: &BootCandidate, next: bool) -> Result<()> {
    let mut nvram_key: &[u8] = b"boot-volume".as_ref();
    if next {
        nvram_key = b"alt-boot-volume".as_ref();
    }

    let boot_str = format!(
        "EF57347C-0000-AA11-AA11-00306543ECAC:{}:{}",
        cand.part_uuid
            .hyphenated()
            .encode_upper(&mut Uuid::encode_buffer()),
        cand.vg_uuid
            .hyphenated()
            .encode_upper(&mut Uuid::encode_buffer())
    );
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/mtd0").map_err(Error::ApplyError)?;
    let mut data = Vec::new();
    file.read_to_end(&mut data).map_err(Error::ApplyError)?;
    let mut nv = nvram_parse(&data)?;
    nv.prepare_for_write();
    nv.active_part_mut().insert_variable(
        nvram_key,
        Cow::Owned(boot_str.into_bytes()),
        VarType::System,
    );
    nv.apply(&mut MtdWriter::new(file))?;
    Ok(())
}
