use std::env;
use std::ffi::OsString;
use std::process::{Command, ExitCode};

const HELP: &str = "Usage: cargo pons export [--example <name>] [--manifest-path <path>]\n\nRuns a cargo example and writes its stdout, so a frontend command can produce IR.\n\nOptions:\n  --example <name>        Example to run (default pons_export)\n  --manifest-path <path>  Cargo manifest of the project\n";

fn main() -> ExitCode {
    match run(env::args_os().skip(1).collect()) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("cargo-pons: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<OsString>) -> Result<ExitCode, String> {
    let args = match args.first().and_then(|arg| arg.to_str()) {
        Some("pons") => &args[1..],
        _ => &args[..],
    };
    let Some(command) = args.first().and_then(|arg| arg.to_str()) else {
        return Err("no command given; run \"cargo pons --help\" for usage".into());
    };
    if matches!(command, "--help" | "-h" | "help") {
        print!("{HELP}");
        return Ok(ExitCode::SUCCESS);
    }
    if command != "export" {
        return Err(format!(
            "unknown command {command:?}; run \"cargo pons --help\" for usage"
        ));
    }

    let mut example: Option<OsString> = None;
    let mut manifest: Option<OsString> = None;
    let mut index = 1;
    while index < args.len() {
        let Some(flag) = args[index].to_str() else {
            return Err(format!("export argument {index} is not valid UTF-8"));
        };
        match flag {
            "--example" if example.is_some() => {
                return Err("--example given more than once".into());
            }
            "--example" => {
                index += 1;
                example = Some(value_for("--example", args.get(index))?);
            }
            "--manifest-path" if manifest.is_some() => {
                return Err("--manifest-path given more than once".into());
            }
            "--manifest-path" => {
                index += 1;
                manifest = Some(value_for("--manifest-path", args.get(index))?);
            }
            other => {
                return Err(format!("unexpected export argument {other:?}"));
            }
        }
        index += 1;
    }

    let cargo = env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo"));
    let mut child = Command::new(cargo);
    child
        .arg("run")
        .arg("--quiet")
        .arg("--example")
        .arg(example.unwrap_or_else(|| OsString::from("pons_export")));
    if let Some(manifest) = manifest {
        child.arg("--manifest-path").arg(manifest);
    }
    let status = child
        .status()
        .map_err(|error| format!("cannot run cargo: {error}"))?;
    Ok(match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => ExitCode::from(code as u8),
        None => ExitCode::from(1),
    })
}

fn value_for(flag: &str, value: Option<&OsString>) -> Result<OsString, String> {
    let value = value.ok_or_else(|| format!("{flag} requires a value"))?;
    if value.to_string_lossy().starts_with('-') {
        return Err(format!(
            "{flag} expects a value, not the flag {:?}",
            value.to_string_lossy()
        ));
    }
    Ok(value.clone())
}
