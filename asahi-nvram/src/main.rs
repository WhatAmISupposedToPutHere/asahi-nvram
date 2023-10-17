// SPDX-License-Identifier: MIT
use std::{
    borrow::Cow,
    env,
    fmt::Debug,
    fs::OpenOptions,
    io::{Read, Seek, Write},
};

use apple_nvram::{erase_if_needed, nvram_parse, VarType};

#[derive(Debug)]
enum Error {
    Parse,
    SectionTooBig,
    ApplyError(std::io::Error),
    MissingPartitionName,
    MissingValue,
    VariableNotFound,
    UnknownPartition,
    InvalidHex,
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

fn main() -> Result<()> {
    real_main()
}

fn real_main() -> Result<()> {
    let matches = clap::command!()
        .arg(clap::arg!(-d --device [DEVICE] "Path to the nvram device."))
        .subcommand(
            clap::Command::new("read")
                .about("Read nvram variables")
                .arg(clap::Arg::new("variable").multiple_values(true)),
        )
        .subcommand(
            clap::Command::new("delete")
                .about("Delete nvram variables")
                .arg(clap::Arg::new("variable").multiple_values(true)),
        )
        .subcommand(
            clap::Command::new("write")
                .about("Write nvram variables")
                .arg(clap::Arg::new("variable=value").multiple_values(true)),
        )
        .get_matches();
    let default_name = "/dev/mtd0".to_owned();
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(matches.get_one::<String>("device").unwrap_or(&default_name))
        .unwrap();
    let mut data = Vec::new();
    file.read_to_end(&mut data).unwrap();
    let mut nv = nvram_parse(&data)?;
    match matches.subcommand() {
        Some(("read", args)) => {
            let active = nv.active_part_mut();
            let vars = args.get_many::<String>("variable");
            if let Some(vars) = vars {
                for var in vars {
                    let (_part, name) = var.split_once(':').ok_or(Error::MissingPartitionName)?;
                    let v = active
                        .get_variable(name.as_bytes())
                        .ok_or(Error::VariableNotFound)?;
                    println!("{}", v);
                }
            } else {
                for var in active.variables() {
                    println!("{}", var);
                }
                // for part in nv.partitions() {
                //     println!("{}", part);
                //     for var in part.variables() {
                //         println!("{}", var);
                //     }
                //     println!("========================================================")
                // }
            }
        }
        Some(("write", args)) => {
            let vars = args.get_many::<String>("variable=value");
            nv.prepare_for_write();
            let active = nv.active_part_mut();
            for var in vars.unwrap_or_default() {
                let (key, value) = var.split_once('=').ok_or(Error::MissingValue)?;
                let (part, name) = key.split_once(':').ok_or(Error::MissingPartitionName)?;
                let typ = part_by_name(part)?;
                active.insert_variable(name.as_bytes(), Cow::Owned(read_var(value)?), typ);
            }
            file.rewind().unwrap();
            let data = nv.serialize()?;
            erase_if_needed(&file, data.len());
            file.write_all(&data).unwrap();
        }
        Some(("delete", args)) => {
            let vars = args.get_many::<String>("variable");
            nv.prepare_for_write();
            let active = nv.active_part_mut();
            for var in vars.unwrap_or_default() {
                let (part, name) = var.split_once(':').ok_or(Error::MissingPartitionName)?;
                let typ = part_by_name(part)?;
                active.remove_variable(name.as_bytes(), typ);
            }
            file.rewind().unwrap();
            let data = nv.serialize()?;
            erase_if_needed(&file, data.len());
            file.write_all(&data).unwrap();
        }
        _ => {}
    }
    Ok(())
}

fn part_by_name(name: &str) -> Result<VarType> {
    match name {
        "common" => Ok(VarType::Common),
        "system" => Ok(VarType::System),
        _ => Err(Error::UnknownPartition),
    }
}

fn read_var(val: &str) -> Result<Vec<u8>> {
    let val = val.as_bytes();
    let mut ret = Vec::new();
    let mut i = 0;
    while i < val.len() {
        if val[i] == b'%' {
            ret.push(
                u8::from_str_radix(
                    unsafe { std::str::from_utf8_unchecked(&val[i + 1..i + 3]) },
                    16,
                )
                .map_err(|_| Error::InvalidHex)?,
            );
            i += 2;
        } else {
            ret.push(val[i])
        }
        i += 1;
    }
    Ok(ret)
}
