// SPDX-License-Identifier: MIT
#![allow(dead_code)]
use asahi_bless::{get_boot_candidates, get_boot_volume, set_boot_volume, BootCandidate, Error, Volume};
use clap::Parser;
use std::{
    io::{stdin, stdout, Write},
    num::{IntErrorKind, NonZeroUsize},
    process::ExitCode,
};

#[cfg(target_os = "macos")]
compile_error!("asahi-bless will only work on linux, if you are on macos, use system `bless` instead");

type Result<T> = std::result::Result<T, Error>;

#[derive(Parser)]
#[command(version)]
struct Args {
    #[arg(
        short,
        long,
        help = "Set boot volume for next boot only"
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
    set_boot: Option<NonZeroUsize>,

    #[arg(
        long,
        conflicts_with = "set_boot",
        help = "Set boot volume to macOS if unambiguous"
    )]
    set_boot_macos: bool,

    #[arg(name = "yes", short, long, help = "Do not ask for confirmation")]
    autoconfirm: bool,
}

fn error_to_string(e: Error) -> String {
    match e {
        Error::Ambiguous => "Ambiguous boot volume".to_string(),
        Error::OutOfRange => "Index out of range".to_string(),
        Error::Parse => "Unable to parse current nvram contents".to_string(),
        Error::SectionTooBig => "Ran out of space on nvram".to_string(),
        Error::ApplyError(e) => format!("Failed to save new nvram contents, try running with sudo? Inner error: {:?}", e),
        Error::NvramReadError(e) => format!("Failed to read nvram contents, try running with sudo? Inner error: {:?}", e),
        Error::DiskReadError(e) => format!("Failed to collect boot candidates, try running with sudo? Inner error: {:?}", e)
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Error: {}", error_to_string(e));
            ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<()> {
    let args = Args::parse();

    if args.list_volumes {
        list_boot_volumes(&args)?;
    } else if let Some(idx) = args.set_boot {
        let cand = get_boot_candidates()?
            .into_iter()
            .nth(idx.get() - 1)
            .ok_or(Error::OutOfRange)?;
        set_boot_volume_by_ref(&cand, &args, false)?;
    } else if args.set_boot_macos {
        let cands = get_boot_candidates()?;
        let macos_cands: Vec<_> = cands
            .iter()
            .filter(|c| {
                c.volumes
                    .first()
                    .map(|n| n.name.starts_with("Macintosh"))
                    .unwrap_or(false)
            })
            .collect();
        if macos_cands.len() == 1 {
            set_boot_volume_by_ref(&macos_cands[0], &args, false)?;
        } else {
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

fn get_vg_name(vg: &[Volume]) -> &str {
    for v in vg {
        if v.is_system {
            return &v.name;
        }
    }
    &vg[0].name
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
        println!("{}{}) {}", is_default, i + 1, get_vg_name(&cand.volumes));
    }
    Ok(cands)
}

fn set_boot_volume_by_ref(cand: &BootCandidate, args: &Args, interactive: bool) -> Result<()> {
    if !interactive {
        let as_what = if !args.next {
            "default boot target"
        } else {
            "boot target for next boot only"
        };
        println!("Will set volume {} as the {}", get_vg_name(&cand.volumes), as_what);
    }
    if !args.autoconfirm && !interactive {
        if !confirm() {
            return Ok(());
        }
    }
    set_boot_volume(cand, args.next)?;
    Ok(())
}

fn interactive_main(args: &Args) -> Result<()> {
    let cands = list_boot_volumes(args)?;
    println!("\nEnter a number to select a boot volume:");

    let mut input = String::new();
    let index = loop {
        print!("==> ");
        stdout().flush().unwrap();

        input.clear();
        stdin().read_line(&mut input).unwrap();

        match input.trim().parse::<usize>() {
            Ok(i @ 1..) if i <= cands.len() => break i - 1,
            Err(e) if e.kind() == &IntErrorKind::Empty => {
                eprintln!("No volume selected. Leaving unchanged.");
                return Ok(());
            },
            _ => eprintln!("Enter a number from 1 to {}", cands.len()),
        }
    };

    set_boot_volume_by_ref(&cands[index], args, true)
}
