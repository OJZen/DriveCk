use std::{
    env,
    io::{self, IsTerminal as _, Write as _},
    process::ExitCode,
};

use driveck_core::{
    ProgressUpdate, TargetInfo, ValidationOptions, collect_targets, discover_target, format_bytes,
    format_report_text, report_has_failures, save_report, validate_target_with_callbacks,
};

#[derive(Default)]
struct CliOptions {
    show_help: bool,
    list_only: bool,
    assume_yes: bool,
    seed: Option<u64>,
    output_path: Option<String>,
    target_path: Option<String>,
}

pub fn run_from_env() -> ExitCode {
    let args = env::args().collect::<Vec<_>>();
    run_with_args_internal(&args, false)
}

pub fn run_with_args(args: &[String]) -> ExitCode {
    run_with_args_internal(args, true)
}

fn run_with_args_internal(args: &[String], supports_gui: bool) -> ExitCode {
    match run(args, supports_gui) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String], supports_gui: bool) -> Result<ExitCode, String> {
    let options = parse_options(args)?;
    if options.show_help {
        let program = args.first().map(String::as_str).unwrap_or("driveck");
        print_usage(program, supports_gui);
        return Ok(ExitCode::SUCCESS);
    }

    if options.list_only {
        let targets = collect_targets().map_err(|error| error.message)?;
        print_targets(&targets);
        return Ok(ExitCode::SUCCESS);
    }

    let target_path = options
        .target_path
        .as_deref()
        .ok_or_else(|| "A device path is required unless --list is used.".to_string())?;
    let target = discover_target(target_path).map_err(|error| error.message)?;
    confirm_block_device(&target, options.assume_yes)?;

    let show_progress = io::stderr().is_terminal();
    let mut progress = |update: ProgressUpdate| {
        if show_progress {
            eprint!(
                "\r{:<12} {:>3}/{}",
                update.phase, update.current, update.total
            );
            if update.final_update {
                eprintln!();
            }
            let _ = io::stderr().flush();
        }
    };

    let report = validate_target_with_callbacks(
        &target,
        &ValidationOptions { seed: options.seed },
        Some(&mut progress),
        None,
    )
    .map_err(|error| error.message)?;

    let text = format_report_text(&target, &report);
    print!("{text}");
    if let Some(path) = options.output_path.as_deref() {
        save_report(path, &target, &report).map_err(|error| error.message)?;
    }

    Ok(if report_has_failures(&report) {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    })
}

fn parse_options(args: &[String]) -> Result<CliOptions, String> {
    let mut options = CliOptions::default();
    let mut index = 1usize;

    while index < args.len() {
        match args[index].as_str() {
            "--list" | "-l" => {
                options.list_only = true;
            }
            "--yes" | "-y" => {
                options.assume_yes = true;
            }
            "--help" | "-h" => {
                options.show_help = true;
            }
            "--output" | "-o" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--output requires a path.".to_string())?;
                options.output_path = Some(value.clone());
            }
            "--seed" => {
                index += 1;
                let value = args
                    .get(index)
                    .ok_or_else(|| "--seed requires a number.".to_string())?;
                options.seed = Some(parse_seed(value)?);
            }
            value if value.starts_with('-') => {
                return Err(format!("Unknown option: {value}"));
            }
            value => {
                if options.target_path.is_some() {
                    return Err("Only one target path may be provided.".to_string());
                }
                options.target_path = Some(value.to_string());
            }
        }
        index += 1;
    }

    if !options.show_help && !options.list_only && options.target_path.is_none() {
        return Err("A device path is required unless --list is used.".to_string());
    }

    Ok(options)
}

fn parse_seed(value: &str) -> Result<u64, String> {
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        return u64::from_str_radix(hex, 16).map_err(|_| format!("Invalid --seed value: {value}"));
    }
    if let Some(bin) = value
        .strip_prefix("0b")
        .or_else(|| value.strip_prefix("0B"))
    {
        return u64::from_str_radix(bin, 2).map_err(|_| format!("Invalid --seed value: {value}"));
    }
    if let Some(oct) = value
        .strip_prefix("0o")
        .or_else(|| value.strip_prefix("0O"))
    {
        return u64::from_str_radix(oct, 8).map_err(|_| format!("Invalid --seed value: {value}"));
    }
    value
        .parse::<u64>()
        .map_err(|_| format!("Invalid --seed value: {value}"))
}

fn confirm_block_device(target: &TargetInfo, assume_yes: bool) -> Result<(), String> {
    if !target.is_block_device || assume_yes {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        return Err(format!(
            "Refusing to touch block device {} without --yes in a non-interactive session.",
            target.path
        ));
    }

    eprintln!(
        "About to validate block device {} ({}{}{}{}{}{}{}).\nThe validator temporarily overwrites sampled regions and restores them afterwards.\nContinue? [y/N]: ",
        target.path,
        format_bytes(target.size_bytes),
        if !target.vendor.is_empty() { ", " } else { "" },
        target.vendor,
        if !target.model.is_empty() { " " } else { "" },
        target.model,
        if target.is_usb { ", usb" } else { "" },
        if target.is_removable {
            ", removable"
        } else {
            ""
        }
    );
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|_| "Failed to read confirmation response.".to_string())?;
    match input.chars().next() {
        Some('y' | 'Y') => Ok(()),
        _ => Err("Validation cancelled.".to_string()),
    }
}

fn print_targets(targets: &[TargetInfo]) {
    println!(
        "{:<16} {:<12} {:<10} {:<10} MODEL",
        "PATH", "SIZE", "STATE", "TRANSPORT"
    );
    if targets.is_empty() {
        println!("No removable or USB whole-disk targets found.");
        return;
    }

    for target in targets {
        let transport = match (target.is_usb, target.is_removable) {
            (true, true) => "usb,rem",
            (true, false) => "usb",
            (false, true) => "rem",
            (false, false) => "-",
        };
        println!(
            "{:<16} {:<12} {:<10} {:<10} {}{}{}",
            target.path,
            format_bytes(target.size_bytes),
            if target.is_mounted {
                "mounted"
            } else {
                "ready"
            },
            transport,
            target.vendor,
            if !target.vendor.is_empty() && !target.model.is_empty() {
                " "
            } else {
                ""
            },
            target.model
        );
    }
}

fn example_device_path() -> &'static str {
    if cfg!(windows) {
        r"\\.\PhysicalDrive2"
    } else {
        "/dev/sdb"
    }
}

fn print_usage(program: &str, supports_gui: bool) {
    let example_device = example_device_path();
    let gui_example = if supports_gui {
        format!("\n  {program} --gui")
    } else {
        String::new()
    };
    let gui_option = if supports_gui {
        "\n      --gui           Force GUI mode when supported by the executable."
    } else {
        ""
    };
    println!(
        "Usage:\n  {0} --list\n  {0} [--yes] [--seed N] [--output FILE] DEVICE\n\nExamples:\n  {0} --list\n  {0} --yes {1}\n  {0} --yes --output report.txt {1}{2}\n\nOptions:\n  -l, --list          List removable/USB whole-disk targets.\n  -o, --output FILE   Write the text report to FILE in addition to stdout.\n  -y, --yes           Skip the destructive-operation confirmation prompt.\n      --seed N        Use a fixed 64-bit seed for deterministic sample data.{3}\n  -h, --help          Show this help text.",
        program, example_device, gui_example, gui_option
    );
}
