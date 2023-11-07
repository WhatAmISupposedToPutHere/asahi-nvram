// SPDX-License-Identifier: MIT
#![allow(dead_code)]
use asahi_bless::{get_boot_candidates, set_boot_volume, Error};
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
    for (i, cand) in cands.iter().enumerate() {
        println!("{}) {}", i + 1, cand.vol_names.join(", "));
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
