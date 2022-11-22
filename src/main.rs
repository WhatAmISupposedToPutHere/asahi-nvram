use std::{
    env,
    fmt::{Debug, Formatter},
    collections::HashMap,
    fs::OpenOptions,
    io::{Read, Write, Seek},
    borrow::Cow
};

struct UnescapeIter<I> {
    inner: I,
    esc_out: u8,
    remaining: u8
}

impl<I> UnescapeIter<I> where I: Iterator<Item=u8> {
    fn new(inner: I) -> Self {
        Self {
            inner,
            esc_out: 0,
            remaining: 0
        }
    }
}


impl<I> Iterator for UnescapeIter<I> where I: Iterator<Item=u8> {
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
            let count = self.inner.next();
            if count.is_none() {
                return None;
            }
            let count = count.unwrap();
            self.esc_out = if count & 0x80 == 0 { 0 } else { 0xFF };
            self.remaining = (count & 0x7F) - 1;
            Some(self.esc_out)
        } else {
            None
        }
    }
}

#[derive(Clone)]
struct CHRPHeader<'a> {
    name: &'a [u8],
    size: u16,
    signature: u8
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
enum Error {
    ParseError,
    SectionTooBig,
    MissingPartitionName,
    MissingValue,
    VariableNotFound,
    UnknownPartition,
    InvalidHex
}

type Result<T> = std::result::Result<T, Error>;

impl CHRPHeader<'_> {
    fn parse<'a>(nvr: &'a [u8]) -> Result<CHRPHeader<'a>> {
        let signature = nvr[0];
        let cksum = nvr[1];
        let size = u16::from_le_bytes(nvr[2..4].try_into().unwrap());
        let name = slice_rstrip(&nvr[4..16], &0);
        let cand = CHRPHeader {name, size, signature};
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

    fn serialize(&self, v: &mut Vec<u8>) {
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
struct Variable<'a> {
    key: &'a [u8],
    value: Cow<'a, [u8]>
}

impl Variable<'_> {
    fn new<'a>(key: &'a [u8], value: &'a [u8]) -> Variable<'a> {
        Variable {
            key,
            value: Cow::Borrowed(value)
        }
    }
}

#[derive(Clone)]
struct Section<'a> {
    header: CHRPHeader<'a>,
    values: HashMap<&'a [u8], Variable<'a>>
}

impl Section<'_> {
    fn parse<'a>(mut nvr: &'a [u8]) -> Result<Section<'a>> {
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
            values.insert(key, Variable::new(key, &cand[(eq + 1)..]));
            nvr = &nvr[(zero + 1)..]
        }
        Ok(Section {
            header, values
        })
    }
    fn size_bytes(&self) -> usize {
        return (self.header.size * 16) as usize;
    }
    fn serialize(&self, v: &mut Vec<u8>) -> Result<()> {
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
            m.entry(&String::from_utf8_lossy(v.key).into_owned(), &String::from_utf8_lossy(&UnescapeIter::new(v.value.iter().map(|x| *x)).collect::<Vec<_>>()).into_owned());
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
struct Partition<'a> {
    header: CHRPHeader<'a>,
    generation: u32,
    common: Section<'a>,
    system: Section<'a>
}

impl Partition<'_> {
    fn parse<'a>(nvr: &'a [u8]) -> Result<Partition<'a>> {
        let header = CHRPHeader::parse(&nvr[..16])?;
        if header.name != b"nvram" {
            return Err(Error::ParseError);
        }
        let adler = u32::from_le_bytes(nvr[16..20].try_into().unwrap());
        let generation = u32::from_le_bytes(nvr[20..24].try_into().unwrap());
        let sec1 = Section::parse(&nvr[32..])?;
        let sec2 = Section::parse(&nvr[(32 + sec1.size_bytes())..])?;
        let calc_adler = adler32::adler32(&nvr[20..(32 + sec1.size_bytes() + sec2.size_bytes())]).unwrap();
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
            header, generation,
            common: com.unwrap(),
            system: sys.unwrap()
        })
    }
    fn size_bytes(&self) -> usize {
        return 32 + self.common.size_bytes() + self.system.size_bytes()
    }
    fn serialize(&self, v: &mut Vec<u8>) -> Result<()> {
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
struct Nvram<'a> {
    partitions: [Partition<'a>; 2],
    active: usize
}

impl Nvram<'_> {
    fn parse<'a>(nvr: &'a [u8]) -> Result<Nvram<'a>> {
        let p1 = Partition::parse(&nvr)?;
        let p2 = Partition::parse(&nvr[p1.size_bytes()..])?;
        let active = if p1.generation > p2.generation { 0 } else { 1 };
        let partitions = [p1, p2];
        Ok(Nvram {
            partitions, active
        })
    }
    fn serialize(&self) -> Result<Vec<u8>> {
        let mut v = Vec::with_capacity(self.partitions[0].size_bytes() * 2);
        self.partitions[0].serialize(&mut v)?;
        self.partitions[1].serialize(&mut v)?;
        Ok(v)
    }
    fn prepare_for_write(&mut self) {
        let inactive = 1 - self.active;
        self.partitions[inactive] = self.partitions[self.active].clone();
        self.partitions[inactive].generation += 1;
        self.active = inactive;
    }
}

fn main() {
    real_main().unwrap();
}

fn real_main() -> Result<()> {
    let matches = clap::command!()
        .arg(clap::arg!(-d --device <DEVICE> "Path to the nvram device."))
        .subcommand(
            clap::Command::new("read")
                .about("Read nvram variables")
                .arg(clap::Arg::new("variable").multiple_values(true))
        )
        .subcommand(
            clap::Command::new("delete")
                .about("Delete nvram variables")
                .arg(clap::Arg::new("variable").multiple_values(true))
        )
        .subcommand(
            clap::Command::new("write")
                .about("Write nvram variables")
                .arg(clap::Arg::new("variable=value").multiple_values(true))
        )
        .get_matches();
    let mut file = OpenOptions::new().read(true).write(true)
        .open(matches.get_one::<String>("device").unwrap()).unwrap();
    let mut data = Vec::new();
    file.read_to_end(&mut data).unwrap();
    let mut nv = Nvram::parse(&data)?;
    match matches.subcommand() {
        Some(("read", args)) => {
            let vars = args.get_many::<String>("variable");
            if vars.is_none() {
                let part = &nv.partitions[nv.active];
                for var in part.common.values.values() {
                    print_var("common", var);
                }
                for var in part.system.values.values() {
                    print_var("system", var);
                }
            } else {
                for var in vars.unwrap() {
                    let (part, name) = var.split_once(':').ok_or(Error::MissingPartitionName)?;
                    let v = part_by_name(part, &mut nv)?.values.get(name.as_bytes()).ok_or(Error::VariableNotFound)?;
                    print_var(part, v);
                }
            }
        },
        Some(("write", args)) => {
            let vars = args.get_many::<String>("variable=value");
            nv.prepare_for_write();
            for var in vars.unwrap_or_default() {
                let (key, value) = var.split_once('=').ok_or(Error::MissingValue)?;
                let (part, name) = key.split_once(':').ok_or(Error::MissingPartitionName)?;
                part_by_name(part, &mut nv)?.values.insert(name.as_bytes(), Variable {
                    key: name.as_bytes(),
                    value: Cow::Owned(read_var(value)?)
                });
            }
            file.rewind().unwrap();
            file.write_all(&nv.serialize()?).unwrap();
        },
        Some(("delete", args)) => {
            let vars = args.get_many::<String>("variable");
            nv.prepare_for_write();
            for var in vars.unwrap_or_default() {
                let (part, name) = var.split_once(':').ok_or(Error::MissingPartitionName)?;
                part_by_name(part, &mut nv)?.values.remove(name.as_bytes());
            }
            file.rewind().unwrap();
            file.write_all(&nv.serialize()?).unwrap();
        },
        _ => {}
    }
    Ok(())
}

fn part_by_name<'a, 'b, 'c>(name: &'a str, nv: &'c mut Nvram<'b>) -> Result<&'c mut Section<'b>> {
    match name {
        "common" => Ok(&mut nv.partitions[nv.active].common),
        "system" => Ok(&mut nv.partitions[nv.active].system),
        _ => Err(Error::UnknownPartition)
    }
}

fn read_var(val: &str) -> Result<Vec<u8>> {
    let val = val.as_bytes();
    let mut ret = Vec::new();
    let mut i = 0;
    while i < val.len() {
        if val[i] == b'%' {
            ret.push(u8::from_str_radix(unsafe {std::str::from_utf8_unchecked(&val[i+1..i+3])}, 16).map_err(|_| Error::InvalidHex)?);
            i += 2;
        } else {
            ret.push(val[i])
        }
        i += 1;
    }
    Ok(ret)
}

fn print_var(section: &str, var: &Variable) {
    let mut value = String::new();
    for c in UnescapeIter::new(var.value.iter().copied()) {
        if (c as char).is_ascii() && !(c as char).is_ascii_control() {
            value.push(c as char);
        } else {
            value.push_str(&format!("%{:02x}", c));
        }
    }
    println!("{}:{}={}", section, String::from_utf8_lossy(var.key), value);
}
