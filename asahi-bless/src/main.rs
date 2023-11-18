// SPDX-License-Identifier: MIT
#![allow(dead_code)]
use asahi_bless::{get_boot_candidates, get_boot_volume, set_boot_volume, Error};
use std::{
    env,
    io::{stdin, stdout, Write},
    process::ExitCode,
};

type Result<T> = std::result::Result<T, Error>;

fn main() -> ExitCode {
    match real_main() {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{:?}", e);
            ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<()> {
    let mut next: bool = false;
    for arg in env::args() {
        if arg == "--next" || arg == "-n" {
            next = true;
        }
    }
    let cands = get_boot_candidates().unwrap();
    let default_cand = get_boot_volume(next).unwrap();
    let mut is_default: &str;
    for (i, cand) in cands.iter().enumerate() {
        if (cand.part_uuid == default_cand.part_uuid) && (cand.vg_uuid == default_cand.vg_uuid) {
            is_default = "*";
        } else {
            is_default = " ";
        }
        println!("{}{}) {}", is_default, i + 1, cand.vol_names.join(", "));
    }
    print!("==> ");
    stdout().flush().unwrap();
    let mut input = String::new();
    stdin().read_line(&mut input).unwrap();
    let ix = input.trim().parse::<usize>().unwrap() - 1;
    if ix >= cands.len() {
        eprintln!("index out of range");
        return Err(Error::OutOfRange);
    };
    set_boot_volume(&cands[ix], next)?;
    Ok(())
}
