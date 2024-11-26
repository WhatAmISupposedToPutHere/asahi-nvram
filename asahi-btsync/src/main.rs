/* SPDX-License-Identifier: MIT */

use std::{
    env,
    fmt::Debug,
    fs,
    fs::OpenOptions,
    io::{self, stdout, Read, Write},
    path::Path,
};

use apple_nvram::{nvram_parse, VarType, Variable};

use ini::Ini;

#[derive(Debug)]
#[allow(dead_code)]
enum Error {
    Parse,
    SectionTooBig,
    ApplyError(std::io::Error),
    VariableNotFound,
    FileIO,
    BluezConfigDirNotFound,
    SliceError,
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

impl From<io::Error> for Error {
    fn from(_e: io::Error) -> Self {
        Error::FileIO
    }
}

impl From<std::array::TryFromSliceError> for Error {
    fn from(_e: std::array::TryFromSliceError) -> Self {
        Error::SliceError
    }
}

type Result<T> = std::result::Result<T, Error>;

fn main() {
    real_main().unwrap();
}

fn real_main() -> Result<()> {
    let matches = clap::command!()
        .arg(clap::arg!(-d --device [DEVICE] "Path to the nvram device."))
        .subcommand(clap::Command::new("list").about("Parse shared Bluetooth keys from nvram"))
        .subcommand(clap::Command::new("list2").about("Parse shared Bluetooth keys from nvram"))
        .subcommand(
            clap::Command::new("sync")
                .about("Sync Bluetooth device information from nvram")
                .arg(clap::arg!(-c --config [CONFIG] "Bluez config path."))
                .arg(clap::Arg::new("variable").multiple_values(true)),
        )
        .subcommand(
            clap::Command::new("dump").about("Dump binary Bluetooth device info from nvram"),
        )
        .get_matches();

    let default_name = "/dev/mtd/by-name/nvram".to_owned();
    let default_config = "/var/lib/bluetooth".to_owned();
    let bt_var = "BluetoothUHEDevices";

    let mut file = OpenOptions::new()
        .read(true)
        .open(matches.get_one::<String>("device").unwrap_or(&default_name))
        .unwrap();
    let mut data = Vec::new();
    file.read_to_end(&mut data).unwrap();
    let mut nv = nvram_parse(&data)?;
    let active = nv.active_part_mut();
    let bt_devs = active
        .get_variable(bt_var.as_bytes(), VarType::System)
        .ok_or(Error::VariableNotFound)?;

    let bt_var2 = "BluetoothInfo";
    let bt_devs2 = active
        .get_variable(bt_var2.as_bytes(), VarType::System)
        .ok_or(Error::VariableNotFound)?;

    match matches.subcommand() {
        Some(("list2", _args)) => {
            print_btkeys2(bt_devs2).expect("Failed to parse bt device info");
        }
        Some(("list", _args)) => {
            print_btkeys(bt_devs).expect("Failed to parse bt device info");
        }
        Some(("sync", args)) => {
            sync_btkeys(
                bt_devs, bt_devs2,
                args.get_one::<String>("config").unwrap_or(&default_config),
            )
            .expect("Failed to sync bt device info");
        }
        Some(("dump", _args)) => {
            dump(bt_devs).expect("Failed to dump bt device info");
        }
        _ => {
            print_btkeys(bt_devs).expect("Failed to parse bt device info");
        }
    }
    Ok(())
}

fn dump(var: &dyn Variable) -> Result<()> {
    stdout().write_all(&var.value())?;
    Ok(())
}

struct BtDevice {
    mac: [u8; 6],
    class: u16,
    name: String,
    vendor_id: Option<u16>,
    product_id: Option<u16>,
    irk: Option<[u8; 16]>, // IdentityResolvingKey.Key
    remote_ltk: Option<[u8; 16]>, // LongTermKey.Key
    remote_rand: Option<u64>, // LongTermKey.Rand
    peripheral_ltk: Option<[u8; 16]>, // PeripheralLongTermKey.Key, SlaveLongTermKey.Key
    ediv: Option<u16>, // LongTermKey.EDiv
    pairing_key: Option<[u8; 16]>,
}

struct BtInfo {
    mac: [u8; 6],
    devices: Vec<BtDevice>,
}

fn read_le_u16(input: &mut &[u8]) -> Result<u16> {
    let (int_bytes, rest) = input.split_at(std::mem::size_of::<u16>());
    *input = rest;
    Ok(u16::from_le_bytes(int_bytes.try_into()?))
}

fn parse_bt_device2(input: &mut &[u8]) -> Result<BtDevice> {
    let mut name: Option<String> = None;
    let mut product_id: Option<u16> = None;
    let mut vendor_id: Option<u16> = None;
    let mut mac: Option<[u8; 6]> = None;
    let mut irk: Option<[u8; 16]> = None;
    //let mut local_ltk: Option<[u8; 16]> = None;
    let mut remote_ltk: Option<[u8; 16]> = None;
    let mut remote_rand: Option<u64> = None;
    let mut peripheral_ltk: Option<[u8; 16]> = None;
    let mut ediv: Option<u16> = None;

    while input.len() != 0 {
        let field_type= input[0];
        let field_len = input[1] as usize;
        *input = &input[2..];
        let (field_bytes, remain) = input.split_at(field_len);
        *input = remain;
        match field_type {
            0x02 => { // Name
                name = Some(String::from_utf8_lossy(&field_bytes[..field_len]).to_string());
            }
            0x04 => { // Product
                product_id = Some(u16::from_le_bytes(field_bytes.try_into().expect("unexpected Product length")));
            }
            0x05 => { // Vendor
                vendor_id = Some(u16::from_le_bytes(field_bytes.try_into().expect("unexpected Vendor length")));
            }
            0x08 => { // IdentityResolvingKey.Key
                irk = Some(field_bytes.try_into().expect("unexpected IRK length"));
            }
            0x09 => {
                // Remote long term key, with one extra byte in front (perhaps a
                // type of some sort).
                remote_ltk = Some(field_bytes[1..].try_into().expect("unexpected Remote LTK length"));
            }
            0x0a => { // LongTermKey.EDiv
                ediv = Some((field_bytes[0] as u16) | (field_bytes[1] as u16) << 8);
            }
            0x0b => { // LongTermKey.Rand
                remote_rand = Some(u64::from_le_bytes(field_bytes.try_into().expect("unexpected Rand length")));
            }
            0x0e => {
                // Mac address, with one extra byte in the beginning.
                // My guess is that this first byte indicates the address type
                // (static or public), we currently assume static addresses.
                mac = Some(field_bytes[1..].try_into().expect("unexpected MAC length"));
            }
            0x0f => {
                // Seems to be the end of the device list?
                println!("---");
                break
            }
            0x10 => {
                // Peripheral/slave long term key, similar to the remote long
                // term key.
                // When the first byte is 1, the key is all zeroes. When it is
                // 2, it has a value.
                if field_bytes[0] == 2 {
                    peripheral_ltk = Some(field_bytes[1..].try_into().expect("unexpected Peripheral LTK length"));
                }
            }
            _ => {
                println!("found field: {} {} {:?}", field_type, field_len, field_bytes);
            }
        }
    }

    // TODO
    //let irk = None
    //let mac: [u8; 6] = [0; 6];
    let class: u16 = 0;
    //let vendor_id: u16 = 0;
    //let product_id: u16 = 0;

    Ok(BtDevice {
        mac: mac.unwrap_or([0; 6]),
        vendor_id,
        product_id,
        class,
        name: name.unwrap_or("".to_string()),
        irk,
        remote_ltk,
        remote_rand,
        peripheral_ltk,
        pairing_key: None,
        ediv,
    })
}

fn parse_bt_device(input: &mut &[u8]) -> Result<BtDevice> {
    // parse MAC
    let (mac_bytes, remain) = input.split_at(6_usize);
    *input = remain;
    let mac: [u8; 6] = mac_bytes.try_into().expect("end of bytes");

    let class = read_le_u16(input)?;

    // skip 2 bytes
    *input = &input[2..];

    // parse device name (u16_le length + \0 terminated utf-8 string)
    let name_len = read_le_u16(input)? as usize;
    let (name_bytes, remain) = input.split_at(name_len);
    *input = remain;
    let name = String::from_utf8_lossy(&name_bytes[..name_len - 1]).to_string();

    // parse pairing key
    let (key_bytes, remain) = input.split_at(16);
    *input = remain;
    let key: [u8; 16] = key_bytes.try_into().expect("end of bytes");

    // parse product / vendor id
    let product_id = read_le_u16(input)?;
    let vendor_id = read_le_u16(input)?;

    // skip 2 unknown trailing bytes
    *input = &input[2..];

    Ok(BtDevice {
        mac,
        class,
        name,
        vendor_id: Some(vendor_id),
        product_id: Some(product_id),
        irk: None,
        pairing_key: Some(key),
        remote_ltk: None,
        remote_rand: None,
        peripheral_ltk: None,
        ediv: None,
    })
}

fn parse_bt_info(var: &dyn Variable) -> Result<BtInfo> {
    let data = var.value();

    assert!(data.len() >= 8);
    let adapter_mac: [u8; 6] = data[0..6].try_into()?;
    let num_devices = data[6];
    assert!(data[7] == 0x04);

    let mut dev_data = &data[8..];

    let mut devices: Vec<BtDevice> = Vec::new();
    for _n in 0..num_devices {
        devices.push(parse_bt_device(&mut dev_data)?);
    }

    Ok(BtInfo {
        mac: adapter_mac,
        devices,
    })
}

fn parse_bt_info2(var: &dyn Variable) -> Result<BtInfo> {
    let data = var.value();

    assert!(data.len() >= 8);
    //let adapter_mac: [u8; 6] = data[0..6].try_into()?;
    let adapter_mac: [u8; 6] = [0; 6];
    //let num_devices = data[0];
    //assert!(data[7] == 0x04);

    let mut dev_data = &data[0..];

    let mut devices: Vec<BtDevice> = Vec::new();
    for _n in 0..2 {
        devices.push(parse_bt_device2(&mut dev_data)?);
        //break
    }

    Ok(BtInfo {
        mac: adapter_mac,
        devices,
    })
}

fn format_mac(mac: &[u8; 6]) -> Result<String> {
    Ok(mac
        .iter()
        .map(|x| format!("{x:02X}"))
        .collect::<Vec<String>>()
        .join(":"))
}

fn format_irk(irk: &Option<[u8; 16]>) -> String {
    match irk {
        Some(irk) => {
            irk
            .iter()
            .map(|x| format!("{x:02X}"))
            .collect::<Vec<String>>()
            .join(":")
        },
        None => String::from("None"),
    }
}

fn format_key_reversed(key: &[u8; 16]) -> Result<String> {
    Ok(key.iter().map(|x| format!("{x:02X}")).rev().collect())
}

fn format_key(key: &[u8; 16]) -> Result<String> {
    Ok(key.iter().map(|x| format!("{x:02X}")).collect())
}

fn print_btkeys2(var: &dyn Variable) -> Result<()> {
    let info = parse_bt_info2(var)?;

    for dev in info.devices {
        println!(
            "ID {:04x}:{:04x} {} ({}) IRK={} LongTermKey.EDiv={}",
            dev.vendor_id.unwrap_or(0),
            dev.product_id.unwrap_or(0),
            dev.name,
            format_mac(&dev.mac)?,
            format_irk(&dev.irk),
            dev.ediv.unwrap_or(0)
        );
    }
    Ok(())
}

fn print_btkeys(var: &dyn Variable) -> Result<()> {
    let info = parse_bt_info(var)?;

    for dev in info.devices {
        println!(
            "ID {:04x}:{:04x} {} ({})",
            dev.vendor_id.unwrap_or(0),
            dev.product_id.unwrap_or(0),
            dev.name,
            format_mac(&dev.mac)?
        );
    }
    Ok(())
}

fn sync_btkeys(var: &dyn Variable, var2: &dyn Variable, config: &String) -> Result<()> {
    let config_path = Path::new(config);

    if !config_path.is_dir() {
        return Err(Error::BluezConfigDirNotFound);
    }

    let info = parse_bt_info2(var2)?;

    let adapter_path = config_path.join(format_mac(&info.mac)?);

    if !adapter_path.is_dir() {
        fs::create_dir(adapter_path.clone())?;
    }

    for dev in info.devices {
        let dev_path = adapter_path.join(format_mac(&dev.mac)?);

        if !dev_path.is_dir() {
            fs::create_dir(dev_path.clone())?;
        }

        let info_file = dev_path.as_path().join("info");

        if info_file.exists() {
            continue;
        }

        let mut info = Ini::new();

        // The ini format is documented here:
        // https://github.com/bluez/bluez/blob/master/doc/settings-storage.txt

        info.with_section(Some("General"))
            .set("Name", dev.name)
            .set("AddressType", "static") // TODO: read from NVRAM
            .set("SupportedTechnologies", "LE;")
            .set("Trusted", "true")
            .set("Blocked", "false")
            .set("WakeAllowed", "true");
        info.with_section(Some("IdentityResolvingKey"))
            .set("Key", format_key(&dev.irk.unwrap())?);
        if dev.remote_ltk.is_some() {
            info.with_section(Some("LongTermKey"))
                .set("Key", format_key(&dev.remote_ltk.unwrap())?)
                .set("Authenticated", "1")
                .set("EncSize", "16")
                .set("EDiv", format!("{}", dev.ediv.unwrap()))
                .set("Rand", format!("{}", dev.remote_rand.unwrap()));
        }
        if dev.peripheral_ltk.is_some() {
            // There's also SlaveLongTermKey but it got deprecated in favor of
            // PeripheralLongTermKey. See:
            // https://github.com/bluez/bluez/commit/1a04dc35b3b2896b398d4352a34d5ae6db04e4f8
            info.with_section(Some("PeripheralLongTermKey"))
                .set("Key", format_key(&dev.peripheral_ltk.unwrap())?)
                .set("Authenticated", "2")
                .set("EncSize", "16")
                .set("EDiv", "0")
                .set("Rand", "0");
        }
        if dev.vendor_id.is_some() {
            info.with_section(Some("DeviceID"))
                .set("Vendor", format!("{}", dev.vendor_id.unwrap()))
                .set("Product", format!("{}", dev.product_id.unwrap()));
        }
        info.write_to_file(info_file)?;

        println!("{}", format_mac(&dev.mac)?);
    }
    Ok(())
}
