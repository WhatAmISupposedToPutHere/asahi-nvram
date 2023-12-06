// SPDX-License-Identifier: MIT
#![allow(dead_code)]
use asahi_bless::{get_boot_candidates, get_boot_volume, set_boot_volume, BootCandidate, Error};
use clap::Parser;
use std::{
    io::{stdin, stdout, Write},
    process::ExitCode,
};

type Result<T> = std::result::Result<T, Error>;

#[derive(Parser)]
struct Args {
    #[arg(
        short,
        long,
        help = "Use alt-boot-volume to get/set default boot volume"
    )]
    next: bool,

    #[arg(
        short,
        long,
        conflicts_with_all = &["set_boot", "set_boot_macos"],
        help = "List boot volume candidates"
    )]
    list_volumes: bool,

    #[arg(long, value_name = "idx", help = "Set boot volume by index")]
    set_boot: Option<isize>,

    #[arg(
        long,
        conflicts_with = "set_boot",
        help = "Set boot volume to macOS if unambiguous"
    )]
    set_boot_macos: bool,

    #[arg(name = "yes", short, long, help = "Do not ask for confirmation")]
    autoconfirm: bool,
}

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
    let args = Args::parse();

    if args.list_volumes {
        list_boot_volumes(&args)?;
    } else if let Some(idx) = args.set_boot {
        let cands = get_boot_candidates()?;
        set_boot_volume_by_index(&cands, idx - 1, &args)?;
    } else if args.set_boot_macos {
        let cands = get_boot_candidates()?;
        let macos_cands: Vec<_> = cands
            .iter()
            .filter(|c| {
                c.vol_names
                    .first()
                    .map(|n| n.starts_with("Macintosh"))
                    .unwrap_or(false)
            })
            .collect();
        if macos_cands.len() == 1 {
            set_boot_volume_by_ref(&macos_cands[0], &args)?;
        } else {
            eprintln!("ambiguous boot volume");
            return Err(Error::Ambiguous);
        }
    } else {
        interactive_main(&args)?;
    }

    Ok(())
}

fn confirm() -> bool {
    print!("confirm? [y/N]: ");
    stdout().flush().unwrap();
    let mut input = String::new();
    stdin().read_line(&mut input).unwrap();
    input.trim().to_lowercase() == "y"
}

fn list_boot_volumes(args: &Args) -> Result<Vec<BootCandidate>> {
    let cands = get_boot_candidates()?;
    let default_cand = get_boot_volume(args.next)?;
    let mut is_default: &str;
    for (i, cand) in cands.iter().enumerate() {
        if (cand.part_uuid == default_cand.part_uuid) && (cand.vg_uuid == default_cand.vg_uuid) {
            is_default = "*";
        } else {
            is_default = " ";
        }
        println!("{}{}) {}", is_default, i + 1, cand.vol_names.join(", "));
    }
    Ok(cands)
}

fn set_boot_volume_by_index(cands: &[BootCandidate], idx: isize, args: &Args) -> Result<()> {
    if idx < 0 || idx as usize >= cands.len() {
        eprintln!("index out of range");
        return Err(Error::OutOfRange);
    }
    set_boot_volume_by_ref(&cands[idx as usize], args)
}

fn set_boot_volume_by_ref(cand: &BootCandidate, args: &Args) -> Result<()> {
    println!("picked {}", cand.vol_names.join(", "));
    if !args.autoconfirm && !confirm() {
        return Ok(());
    }
    set_boot_volume(cand, args.next)?;
    println!("boot volume set");
    Ok(())
}

fn interactive_main(args: &Args) -> Result<()> {
    let cands = list_boot_volumes(args)?;
    print!("==> ");
    stdout().flush().unwrap();
    let mut input = String::new();
    stdin().read_line(&mut input).unwrap();
    let ix = input.trim().parse::<isize>().unwrap() - 1;
    set_boot_volume_by_index(&cands, ix, args)?;
    Ok(())
}
