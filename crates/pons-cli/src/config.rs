use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use pons_plugin::PluginPin;

use crate::CliError;
use crate::options;

pub(crate) const FILE_NAME: &str = "pons.toml";
pub(crate) const DOCUMENT_TABLE: &str = "project";
pub(crate) const DOCUMENT: &str = "ir";
pub(crate) const NAME: &str = "name";
pub(crate) const FRONTEND_TABLE: &str = "frontend";
pub(crate) const KIND: &str = "kind";
pub(crate) const COMMAND: &str = "command";
pub(crate) const EMIT_TABLE: &str = "emit";
pub(crate) const OUT_DIR: &str = "out_dir";

const TOP_LEVEL: &[&str] = &[EMIT_TABLE, FRONTEND_TABLE, "plugins", DOCUMENT_TABLE];
const PROJECT: &[&str] = &[DOCUMENT, NAME];
const FRONTEND: &[&str] = &[KIND, COMMAND];
const PLUGINS: &[&str] = &["search", "pin"];
const PIN: &[&str] = &["path", "sha256"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrontendKind {
    Axum,
    Spring,
}

impl FrontendKind {
    fn parse(word: &str) -> Option<Self> {
        match word {
            "axum" => Some(Self::Axum),
            "spring" => Some(Self::Spring),
            _ => None,
        }
    }

    pub(crate) fn default_command(self) -> &'static str {
        match self {
            Self::Axum => "cargo pons export",
            Self::Spring => "./gradlew --quiet ponsExport",
        }
    }
}

impl std::fmt::Display for FrontendKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Axum => "axum",
            Self::Spring => "spring",
        })
    }
}

pub(crate) struct Frontend {
    kind: FrontendKind,
    command: String,
}

impl Frontend {
    pub(crate) fn kind(&self) -> FrontendKind {
        self.kind
    }

    pub(crate) fn command(&self) -> &str {
        &self.command
    }
}

pub(crate) struct Config {
    file: PathBuf,
    document: Option<PathBuf>,
    frontend: Option<Frontend>,
    targets: BTreeMap<String, toml::Table>,
    search: Vec<PathBuf>,
    pins: BTreeMap<String, PluginPin>,
}

impl Config {
    pub(crate) fn file(&self) -> &Path {
        &self.file
    }

    pub(crate) fn directory(&self) -> &Path {
        self.file
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
    }

    pub(crate) fn document(&self) -> Option<&Path> {
        self.document.as_deref()
    }

    pub(crate) fn frontend(&self) -> Option<&Frontend> {
        self.frontend.as_ref()
    }

    pub(crate) fn target(&self, name: &str) -> Target<'_> {
        Target::with_values(
            Some(&self.file),
            name,
            self.targets.get(name).cloned().unwrap_or_default(),
        )
    }

    pub(crate) fn search(&self) -> &[PathBuf] {
        &self.search
    }

    pub(crate) fn pin(&self, target: &str) -> Option<&PluginPin> {
        self.pins.get(target)
    }
}

pub(crate) struct Target<'a> {
    file: Option<&'a Path>,
    pub(crate) target: String,
    pub(crate) values: toml::Table,
}

impl Target<'_> {
    pub(crate) fn empty(target: String) -> Self {
        Target::with_values(None, &target, toml::Table::new())
    }

    pub(crate) fn name(&self) -> String {
        format!("{EMIT_TABLE}.{}", self.target)
    }

    pub(crate) fn with_values<'a>(
        file: Option<&'a Path>,
        target: &str,
        values: toml::Table,
    ) -> Target<'a> {
        Target {
            file,
            target: target.to_owned(),
            values,
        }
    }

    pub(crate) fn get(&self, key: &str) -> Option<&toml::Value> {
        self.values.get(key)
    }

    pub(crate) fn error(&self, message: &str) -> CliError {
        match self.file {
            Some(file) => {
                CliError::Usage(format!("{}: [{}] {message}", file.display(), self.name()))
            }
            None => CliError::Usage(message.to_owned()),
        }
    }
}

pub(crate) fn discover(document: Option<&Path>) -> Result<Option<Config>, CliError> {
    let mut starts: Vec<PathBuf> = document
        .and_then(Path::parent)
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| vec![parent.to_path_buf()])
        .unwrap_or_default();
    if let Ok(current) = std::env::current_dir() {
        starts.push(current);
    }
    for start in starts {
        for directory in start.ancestors() {
            let file = directory.join(FILE_NAME);
            if file.is_file() {
                return load(file).map(Some);
            }
        }
    }
    Ok(None)
}

pub(crate) fn load(file: PathBuf) -> Result<Config, CliError> {
    let source = fs::read_to_string(&file)
        .map_err(|error| CliError::Failure(format!("cannot read {}: {error}", file.display())))?;
    let parsed = source
        .parse::<toml::Table>()
        .map_err(|error| CliError::Failure(format!("{}: {error}", file.display())))?;
    let directory = file
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
        .to_path_buf();
    let mut config = Config {
        file,
        document: None,
        frontend: None,
        targets: BTreeMap::new(),
        search: Vec::new(),
        pins: BTreeMap::new(),
    };
    for (key, value) in &parsed {
        match key.as_str() {
            DOCUMENT_TABLE => {
                config.document =
                    project(&config.file, value)?.map(|document| under(&directory, document))
            }
            FRONTEND_TABLE => config.frontend = Some(frontend(&config.file, value)?),
            "plugins" => {
                let plugins = plugins(&config.file, value, &directory)?;
                config.search = plugins.search;
                config.pins = plugins.pins;
            }
            EMIT_TABLE => config.targets = targets(&config.file, value, &directory)?,
            other => return Err(unknown_key(&config.file, "", other, TOP_LEVEL)),
        }
    }
    Ok(config)
}

fn under(directory: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        directory.join(path)
    }
}

fn project(file: &Path, value: &toml::Value) -> Result<Option<PathBuf>, CliError> {
    let table = table(file, DOCUMENT_TABLE, value)?;
    check_keys(file, DOCUMENT_TABLE, table, PROJECT)?;
    let Some(value) = table.get(DOCUMENT) else {
        return Ok(None);
    };
    let Some(document) = value.as_str() else {
        return Err(config_error(
            file,
            DOCUMENT_TABLE,
            &format!(
                "{DOCUMENT} is {}; it is a path to an IR document",
                kind(value)
            ),
        ));
    };
    Ok(Some(PathBuf::from(document)))
}

fn frontend(file: &Path, value: &toml::Value) -> Result<Frontend, CliError> {
    let table = table(file, FRONTEND_TABLE, value)?;
    check_keys(file, FRONTEND_TABLE, table, FRONTEND)?;
    let frontend_kind = match table.get(KIND) {
        None => {
            return Err(config_error(
                file,
                FRONTEND_TABLE,
                "has no kind; it is axum or spring",
            ));
        }
        Some(value) => {
            let Some(word) = value.as_str() else {
                return Err(config_error(
                    file,
                    FRONTEND_TABLE,
                    &format!("{KIND} is {}; it is axum or spring", kind(value)),
                ));
            };
            FrontendKind::parse(word).ok_or_else(|| {
                config_error(
                    file,
                    FRONTEND_TABLE,
                    &format!("{KIND} is {word:?}; it is axum or spring"),
                )
            })?
        }
    };
    let command = match table.get(COMMAND) {
        None => frontend_kind.default_command().to_owned(),
        Some(value) => {
            let Some(command) = value.as_str() else {
                return Err(config_error(
                    file,
                    FRONTEND_TABLE,
                    &format!("{COMMAND} is {}; it is a shell command", kind(value)),
                ));
            };
            if command.is_empty() {
                return Err(config_error(
                    file,
                    FRONTEND_TABLE,
                    &format!("{COMMAND} is empty; it is a shell command"),
                ));
            }
            command.to_owned()
        }
    };
    Ok(Frontend {
        kind: frontend_kind,
        command,
    })
}

struct Plugins {
    search: Vec<PathBuf>,
    pins: BTreeMap<String, PluginPin>,
}

fn plugins(file: &Path, value: &toml::Value, directory: &Path) -> Result<Plugins, CliError> {
    let table = table(file, "plugins", value)?;
    check_keys(file, "plugins", table, PLUGINS)?;
    let search = match table.get("search") {
        None => Vec::new(),
        Some(value) => {
            let Some(entries) = value.as_array() else {
                return Err(config_error(
                    file,
                    "plugins",
                    &format!("search is {}; it is an array of directories", kind(value)),
                ));
            };
            let mut search = Vec::new();
            for entry in entries {
                let Some(entry) = entry.as_str() else {
                    return Err(config_error(
                        file,
                        "plugins",
                        &format!("search holds {}; it is a path to a directory", kind(entry)),
                    ));
                };
                search.push(under(directory, PathBuf::from(entry)));
            }
            search
        }
    };
    let pins = match table.get("pin") {
        None => BTreeMap::new(),
        Some(value) => pins(file, value, directory)?,
    };
    Ok(Plugins { search, pins })
}

fn pins(
    file: &Path,
    value: &toml::Value,
    directory: &Path,
) -> Result<BTreeMap<String, PluginPin>, CliError> {
    let pins_table = table(file, "plugins.pin", value)?;
    let mut pins = BTreeMap::new();
    for (target, value) in pins_table {
        let name = format!("plugins.pin.{target}");
        if target.is_empty()
            || !target
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(config_error(
                file,
                "plugins.pin",
                &format!("{target:?} cannot be a target name; a name is letters, digits, - and _"),
            ));
        }
        let pin_table = table(file, &name, value)?;
        check_keys(file, &name, pin_table, PIN)?;
        let path = match pin_table.get("path") {
            None => {
                return Err(config_error(
                    file,
                    &name,
                    "has no path; it is the executable to pin",
                ));
            }
            Some(value) => value.as_str().ok_or_else(|| {
                config_error(
                    file,
                    &name,
                    &format!("path is {}; it is a path to an executable", kind(value)),
                )
            })?,
        };
        let path = under(directory, PathBuf::from(path));
        if !path.is_file() {
            return Err(config_error(
                file,
                &name,
                &format!("path {} is not a file", path.display()),
            ));
        }
        let checksum = match pin_table.get("sha256") {
            None => {
                return Err(config_error(
                    file,
                    &name,
                    "has no sha256; it is the checksum of the pinned executable",
                ));
            }
            Some(value) => value.as_str().ok_or_else(|| {
                config_error(
                    file,
                    &name,
                    &format!("sha256 is {}; it is 64 hexadecimal characters", kind(value)),
                )
            })?,
        };
        let pin =
            PluginPin::new(path, checksum).map_err(|error| config_error(file, &name, &error))?;
        pins.insert(target.clone(), pin);
    }
    Ok(pins)
}

fn targets(
    file: &Path,
    value: &toml::Value,
    directory: &Path,
) -> Result<BTreeMap<String, toml::Table>, CliError> {
    let emit = table(file, EMIT_TABLE, value)?;
    let mut targets: BTreeMap<String, toml::Table> = BTreeMap::new();
    for (name, value) in emit {
        let named = format!("{EMIT_TABLE}.{name}");
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(config_error(
                file,
                EMIT_TABLE,
                &format!("{name:?} cannot be a target name; a name is letters, digits, - and _"),
            ));
        }
        let table = table(file, &named, value)?;
        if let Some(allowed) = options::vocabulary(name) {
            let mut allowed: Vec<&str> = allowed.iter().map(|(key, _)| *key).collect();
            allowed.push(OUT_DIR);
            check_keys(file, &named, table, &allowed)?;
        }
        let mut values = table.clone();
        if let Some(value) = values.get_mut(OUT_DIR)
            && let Some(path) = value.as_str().map(PathBuf::from)
        {
            *value = toml::Value::String(under(directory, path).to_string_lossy().into_owned());
        }
        let target = Target::with_values(Some(file), name, values.clone());
        options::check_values(&target)?;
        targets.insert(name.clone(), values);
    }
    Ok(targets)
}

fn table<'a>(file: &Path, name: &str, value: &'a toml::Value) -> Result<&'a toml::Table, CliError> {
    value
        .as_table()
        .ok_or_else(|| config_error(file, name, &format!("is {}; it is a table", kind(value))))
}

fn check_keys(
    file: &Path,
    name: &str,
    table: &toml::Table,
    allowed: &[&str],
) -> Result<(), CliError> {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(unknown_key(file, name, key, allowed));
        }
    }
    Ok(())
}

pub(crate) fn unknown_key(file: &Path, name: &str, key: &str, allowed: &[&str]) -> CliError {
    config_error(
        file,
        name,
        &format!("has no key {key:?}; it is {}", allowed.join(", ")),
    )
}

pub(crate) fn config_error(file: &Path, name: &str, message: &str) -> CliError {
    let place = if name.is_empty() {
        String::new()
    } else {
        format!("[{name}] ")
    };
    CliError::Usage(format!("{}: {place}{message}", file.display()))
}

pub(crate) fn kind(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::String(_) => "a string",
        toml::Value::Integer(_) => "an integer",
        toml::Value::Float(_) => "a float",
        toml::Value::Boolean(_) => "a boolean",
        toml::Value::Datetime(_) => "a date and time",
        toml::Value::Array(_) => "an array",
        toml::Value::Table(_) => "a table",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(directory: &Path, source: &str) -> Result<Config, CliError> {
        let file = directory.join(FILE_NAME);
        fs::write(&file, source).unwrap();
        load(file)
    }

    fn refusal(source: &str) -> String {
        let directory = tempfile::tempdir().unwrap();
        match loaded(directory.path(), source) {
            Ok(_) => panic!("{source:?} must be refused"),
            Err(error) => error.message(),
        }
    }

    #[test]
    fn a_misspelled_key_names_the_file_the_table_and_the_key() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("nested").join(FILE_NAME);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "[emit.ts]\nfieldcase = \"camel\"\n").unwrap();

        let error = load(file.clone())
            .err()
            .expect("a misspelled option is refused");
        assert!(
            matches!(error, CliError::Usage(_)),
            "a misspelled option is a mistake in the arguments: {error:?}"
        );
        let message = error.message();
        assert!(
            message
                == format!(
                    "{}: [emit.ts] has no key \"fieldcase\"; it is mode, errors, namespaces, \
                 field_case, newtypes, int64, date_mode, runtime, validation, out_dir",
                    file.display()
                ),
            "{message}"
        );
    }

    #[test]
    fn every_table_refuses_a_key_it_does_not_hold() {
        for (source, expected) in [
            ("emmit = 1\n", "has no key \"emmit\""),
            (
                "[emit.openapi]\nout_dirs = \"x\"\n",
                "has no key \"out_dirs\"",
            ),
            ("[project]\noutput = \"x\"\n", "has no key \"output\""),
            ("[plugins]\npaths = [\"x\"]\n", "has no key \"paths\""),
        ] {
            let message = refusal(source);
            assert!(message.contains(expected), "{source}: {message}");
        }
    }

    #[test]
    fn a_value_outside_the_option_vocabulary_is_refused() {
        let message = refusal("[emit.ts]\nfield_case = \"Camel\"\n");
        assert!(
            message.ends_with("[emit.ts] field_case is \"Camel\"; it is wire or camel"),
            "{message}"
        );

        let message = refusal("[emit.ts]\nmode = \"sloppy\"\n");
        assert!(
            message.ends_with("[emit.ts] mode is \"sloppy\"; it is strict or lossy"),
            "{message}"
        );

        let message = refusal("[emit.python]\nmode = \"sloppy\"\n");
        assert!(
            message.ends_with("[emit.python] mode is \"sloppy\"; it is strict or lossy"),
            "the cli keys are the cli's own in every table: {message}"
        );

        let message = refusal("[emit.ts]\nout_dir = true\n");
        assert!(
            message.ends_with("[emit.ts] out_dir is a boolean; it is a path to a directory"),
            "{message}"
        );

        let message = refusal("[emit.ts]\nint64 = 12\n");
        assert!(
            message.ends_with("[emit.ts] int64 is an integer; it is bigint or string or number"),
            "{message}"
        );
    }

    #[test]
    fn a_table_that_is_not_a_table_is_refused() {
        let message = refusal("emit = 3\n");
        assert!(
            message.ends_with("[emit] is an integer; it is a table"),
            "{message}"
        );

        let message = refusal("[emit]\nts = 3\n");
        assert!(
            message.ends_with("[emit.ts] is an integer; it is a table"),
            "{message}"
        );

        let message = refusal("project = 3\n");
        assert!(
            message.ends_with("[project] is an integer; it is a table"),
            "{message}"
        );
    }

    #[test]
    fn a_target_name_no_executable_can_carry_is_refused() {
        let message = refusal("[emit.\"my target\"]\nout_dir = \"x\"\n");
        assert!(
            message.ends_with(
                "[emit] \"my target\" cannot be a target name; a name is letters, digits, - and _"
            ),
            "{message}"
        );
    }

    #[test]
    fn a_date_and_time_is_refused_where_an_emitter_reads_json() {
        let message = refusal("[emit.python]\ncutoff = 1979-05-27T07:32:00Z\n");
        assert!(
            message.ends_with(
                "[emit.python] cutoff is a date and time; an emitter reads options as JSON"
            ),
            "{message}"
        );
    }

    #[test]
    fn a_file_that_is_not_toml_names_the_file_and_the_line() {
        let message = refusal("[emit.ts\n");
        assert!(message.contains("TOML parse error at line"), "{message}");
        assert!(
            message.ends_with(FILE_NAME) || message.contains(FILE_NAME),
            "{message}"
        );
    }

    #[test]
    fn a_path_in_the_file_is_relative_to_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        fs::create_dir(&root).unwrap();
        let config = loaded(
            &root,
            "[project]\nir = \"contract/api.json\"\n\n[emit.ts]\nout_dir = \"out\"\n\n[plugins]\nsearch = [\"tools\"]\n",
        )
        .unwrap();

        assert_eq!(
            config.document(),
            Some(root.join("contract/api.json").as_path())
        );
        assert_eq!(
            config.target("ts").get(OUT_DIR).and_then(|v| v.as_str()),
            Some(root.join("out").to_str().unwrap())
        );
        assert_eq!(config.search(), [root.join("tools")]);
    }

    #[test]
    fn an_absolute_path_in_the_file_is_left_alone() {
        let directory = tempfile::tempdir().unwrap();
        let absolute = directory.path().join("elsewhere");
        let config = loaded(
            directory.path(),
            &format!("[project]\nir = {:?}\n", absolute.to_str().unwrap()),
        )
        .unwrap();
        assert_eq!(config.document(), Some(absolute.as_path()));
    }

    #[test]
    fn a_target_with_no_table_reads_as_an_empty_one() {
        let directory = tempfile::tempdir().unwrap();
        let config = loaded(directory.path(), "[emit.ts]\n").unwrap();
        let target = config.target("openapi");
        assert!(target.values.is_empty());
        assert_eq!(target.name(), "emit.openapi");
    }

    #[test]
    fn the_frontend_defaults_its_command_and_reads_an_override() {
        let directory = tempfile::tempdir().unwrap();
        let config = loaded(directory.path(), "[frontend]\nkind = \"axum\"\n").unwrap();
        let frontend = config.frontend().expect("the table names a frontend");
        assert_eq!(frontend.kind(), FrontendKind::Axum);
        assert_eq!(frontend.command(), "cargo pons export");

        let config = loaded(directory.path(), "[frontend]\nkind = \"spring\"\n").unwrap();
        let frontend = config.frontend().expect("the table names a frontend");
        assert_eq!(frontend.kind(), FrontendKind::Spring);
        assert_eq!(frontend.command(), "./gradlew --quiet ponsExport");

        let config = loaded(
            directory.path(),
            "[frontend]\nkind = \"axum\"\ncommand = \"./tools/export.sh\"\n",
        )
        .unwrap();
        assert_eq!(
            config
                .frontend()
                .expect("the table names a frontend")
                .command(),
            "./tools/export.sh"
        );
    }

    #[test]
    fn the_frontend_refuses_what_it_does_not_know() {
        for (source, expected) in [
            (
                "[frontend]\nkind = \"warp\"\n",
                "[frontend] kind is \"warp\"; it is axum or spring",
            ),
            (
                "[frontend]\nkind = \"axum\"\nrunner = \"x\"\n",
                "[frontend] has no key \"runner\"; it is kind, command",
            ),
            (
                "[frontend]\nkind = 3\n",
                "[frontend] kind is an integer; it is axum or spring",
            ),
            (
                "[frontend]\nkind = \"axum\"\ncommand = true\n",
                "[frontend] command is a boolean; it is a shell command",
            ),
            (
                "[frontend]\nkind = \"axum\"\ncommand = \"\"\n",
                "[frontend] command is empty; it is a shell command",
            ),
            (
                "[frontend]\ncommand = \"x\"\n",
                "[frontend] has no kind; it is axum or spring",
            ),
            ("frontend = 3\n", "[frontend] is an integer; it is a table"),
        ] {
            let message = refusal(source);
            assert!(message.ends_with(expected), "{source}: {message}");
        }
    }

    #[test]
    fn the_project_name_is_a_documented_key_that_does_not_supply_the_document() {
        let directory = tempfile::tempdir().unwrap();
        let config = loaded(
            directory.path(),
            "[project]\nname = \"acme\"\nir = \"api/ir.json\"\n",
        )
        .unwrap();
        assert_eq!(
            config.document(),
            Some(directory.path().join("api/ir.json").as_path())
        );
    }

    #[test]
    fn the_config_knows_the_directory_it_was_loaded_from() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("project");
        fs::create_dir(&root).unwrap();
        let config = loaded(&root, "[frontend]\nkind = \"axum\"\n").unwrap();
        assert_eq!(config.directory(), root.as_path());
        assert_eq!(config.file(), root.join(FILE_NAME).as_path());
    }

    #[test]
    fn a_table_the_config_does_not_hold_still_names_itself() {
        let target = Target::empty("python".into());
        assert!(target.get("mode").is_none());
        assert_eq!(target.error("no file here").message(), "no file here");
    }
}
