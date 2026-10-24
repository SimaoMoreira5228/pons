#![allow(dead_code)]

use std::{env, path::PathBuf};

pub fn package(name: &str) -> PathBuf {
    let mut directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        for scope in [
            directory.join("node_modules"),
            directory.join("tooling/node_modules"),
        ] {
            let candidate = scope.join(name);
            if candidate.join("package.json").is_file() {
                return candidate;
            }
        }
        if !directory.pop() {
            panic!(
                "no {name} for the generated TypeScript tests, looked in {:?} and every directory \
                 above it; install the toolchain with `npm --prefix tooling install \
                 --no-package-lock`",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

pub fn bin_directories() -> Vec<PathBuf> {
    let mut directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut directories = Vec::new();
    loop {
        directories.push(directory.join("node_modules/.bin"));
        directories.push(directory.join("tooling/node_modules/.bin"));
        if !directory.pop() {
            return directories;
        }
    }
}
