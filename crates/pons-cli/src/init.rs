use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use crate::CliError;

pub(crate) fn run(path: &Path) -> Result<(), CliError> {
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(directory).map_err(|error| {
        CliError::Failure(format!("cannot create {}: {error}", directory.display()))
    })?;
    let name = project_name(directory)?;
    crate::file::write_new(path, &starter(&name))?;
    println!("wrote {}", path.display());
    Ok(())
}

fn project_name(directory: &Path) -> Result<String, CliError> {
    let resolved = fs::canonicalize(directory).map_err(|error| {
        CliError::Failure(format!(
            "cannot resolve the project directory {}: {error}",
            directory.display()
        ))
    })?;
    resolved
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .ok_or_else(|| {
            CliError::Failure(format!(
                "cannot read a project name from {}",
                resolved.display()
            ))
        })
}

fn starter(name: &str) -> String {
    format!(
        "[project]\nname = {name}\nir = \"api/ir.json\"\n\n[frontend]\nkind = \"axum\"\ncommand = \"cargo pons export\"\n\n[emit.ts]\nout_dir = \"web/src/api\"\nmode = \"strict\"\n",
        name = toml_string(name)
    )
}

fn toml_string(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        match character {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            character if character.is_control() => {
                let _ = write!(quoted, "\\u{:04X}", character as u32);
            }
            character => quoted.push(character),
        }
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config;

    #[test]
    fn the_starter_parses_and_names_the_project_directory() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("acme");
        fs::create_dir(&root).unwrap();
        let path = root.join(config::FILE_NAME);

        run(&path).unwrap();

        let source = fs::read_to_string(&path).unwrap();
        assert!(source.contains("name = \"acme\""), "{source}");
        assert!(!root.join("api").exists(), "init creates no api directory");

        let loaded = config::load(path).unwrap();
        assert_eq!(loaded.document(), Some(root.join("api/ir.json").as_path()));
        let frontend = loaded.frontend().expect("the starter names a frontend");
        assert_eq!(frontend.name(), "axum");
        assert_eq!(frontend.command(), "cargo pons export");
    }

    #[test]
    fn an_existing_file_is_left_alone() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(config::FILE_NAME);
        fs::write(&path, "hand written\n").unwrap();

        let error = run(&path).unwrap_err();
        assert_eq!(
            error.message(),
            format!("{} already exists", path.display())
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "hand written\n");
    }

    #[test]
    fn a_name_that_needs_quoting_stays_a_toml_string() {
        assert_eq!(toml_string("plain"), "\"plain\"");
        assert_eq!(toml_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(toml_string("line\nbreak"), "\"line\\nbreak\"");
    }
}
