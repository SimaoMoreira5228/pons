mod support;

use std::fs;

use support::{FIXTURE, pons};

#[test]
fn help_documents_every_exit_code() {
    let output = pons().arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&output.stdout);
    for line in ["0  ", "1  ", "2  ", "3  ", "4  ", "5  "] {
        assert!(help.contains(line), "{line} missing from the help");
    }
    assert!(help.contains("breaking"), "{help}");
    assert!(help.contains("unreadable"), "{help}");
    assert!(help.contains("init [path]"), "{help}");
    assert!(help.contains("export"), "{help}");
    assert!(help.contains("cargo pons export"), "{help}");
}

#[test]
fn malformed_arguments_exit_with_the_usage_code_and_a_one_line_diagnosis() {
    let directory = tempfile::tempdir().unwrap();
    fs::write(directory.path().join("api.json"), FIXTURE).unwrap();

    for (arguments, expected) in [
        (vec![], "no command"),
        (vec!["check", "a", "b"], "check expects one document"),
        (vec!["fmt", "a", "b"], "fmt expects one document"),
        (vec!["diff", "a"], "diff expects two documents"),
        (
            vec!["emit", "--target", "ts"],
            "emit --target ts needs an IR document",
        ),
        (
            vec!["emit", "--target", "ts", "api.json"],
            "emit --target ts needs an output directory",
        ),
        (
            vec!["emit", "--target", "ts", "api.json", "out", "extra"],
            "emit takes at most two positional arguments",
        ),
        (
            vec!["emit", "api.json", "api.json", "api.json", "out"],
            "not \"api.json\"",
        ),
        (
            vec!["emit", "--target", "ts", "api.json", "out", "--nonsense"],
            "--nonsense",
        ),
        (vec!["nonsense", "api.json"], "unknown command \"nonsense\""),
        (vec!["init", "a", "b"], "init takes at most one path"),
        (vec!["export", "extra"], "export takes no arguments"),
    ] {
        let output = pons()
            .args(&arguments)
            .current_dir(directory.path())
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(output.status.code(), Some(4), "{arguments:?}: {stderr}");
        assert!(stderr.starts_with("pons: "), "{arguments:?}: {stderr}");
        assert_eq!(stderr.lines().count(), 1, "{arguments:?}: {stderr}");
        assert!(stderr.contains(expected), "{arguments:?}: {stderr}");
        assert!(!directory.path().join("out").exists(), "{arguments:?}");
    }
}

#[test]
fn help_is_printed_on_stdout() {
    for argument in ["--help", "-h", "help"] {
        let output = pons().arg(argument).output().unwrap();
        assert!(output.status.success(), "{argument}");
        assert!(String::from_utf8_lossy(&output.stdout).starts_with("Usage: pons"));
        assert!(output.stderr.is_empty(), "{argument}");
    }
}
