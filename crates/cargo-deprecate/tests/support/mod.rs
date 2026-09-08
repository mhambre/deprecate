use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

pub struct Project {
    root: TempDir,
    original: Vec<(String, String)>,
}

impl Project {
    /// Writes a readable multi-file fixture into an isolated temporary project.
    pub fn new(files: &[(&str, &str)]) -> Self {
        let mut project = Self {
            root: tempfile::tempdir().expect("create fixture directory"),
            original: Vec::new(),
        };
        for (path, source) in files {
            project.write(path, source);
            project
                .original
                .push(((*path).to_owned(), project.read(path)));
        }
        project
    }

    /// Expands the local annotation dependency and creates parent directories.
    pub fn write(&self, path: &str, source: &str) {
        let path = self.path(path);
        fs::create_dir_all(path.parent().expect("fixture file has a parent"))
            .expect("create fixture directories");
        let annotation_crate = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .expect("resolve annotation crate");
        let escaped = serde_json::to_string(&annotation_crate.to_string_lossy())
            .expect("escape annotation crate path");
        let source = source.replace("\"$DEPRECATE\"", &escaped);
        fs::write(&path, source)
            .unwrap_or_else(|error| panic!("write {}: {error}", path.display()));
    }

    /// Reads a project-relative UTF-8 fixture file.
    pub fn read(&self, path: &str) -> String {
        let path = self.path(path);
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
    }

    /// Compares complete file contents, including whitespace.
    pub fn assert_file(&self, path: &str, expected: &str) {
        assert_eq!(self.read(path), expected, "fixture file {path}");
    }

    /// Verifies every supplied file still matches its initial contents.
    pub fn assert_unchanged(&self) {
        for (path, source) in &self.original {
            self.assert_file(path, source);
        }
    }

    /// Parses a generated catalog for structural assertions.
    pub fn json(&self, path: &str) -> serde_json::Value {
        serde_json::from_str(&self.read(path)).expect("fixture file is valid JSON")
    }

    /// Runs the built CLI from the fixture root.
    pub fn cli(&self, args: &[&str]) -> Run {
        self.cli_in("", args)
    }

    /// Runs the CLI from a selected workspace member or subdirectory.
    pub fn cli_in(&self, directory: &str, args: &[&str]) -> Run {
        self.run(env!("CARGO_BIN_EXE_cargo-deprecate"), directory, args)
    }

    /// Compiles or tests the fixture using the test suite's Cargo toolchain.
    pub fn cargo(&self, args: &[&str]) -> Run {
        self.run(env!("CARGO"), "", args)
    }

    /// Rejects fixture paths that could escape the temporary directory.
    fn path(&self, path: &str) -> PathBuf {
        assert!(
            Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)),
            "fixture paths must stay inside the project: {path}"
        );
        self.root.path().join(path)
    }

    /// Isolates build output and disables network access for fixture commands.
    fn run(&self, program: &str, directory: &str, args: &[&str]) -> Run {
        let cwd = self
            .path(directory)
            .canonicalize()
            .expect("resolve fixture directory");
        let target = self.path("target");
        let output = Command::new(program)
            .args(args)
            .current_dir(&cwd)
            .env("CARGO_NET_OFFLINE", "true")
            .env("CARGO_TARGET_DIR", target)
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .output()
            .unwrap_or_else(|error| panic!("run {program} {args:?} in {}: {error}", cwd.display()));
        Run {
            command: format!("{program} {args:?} in {}", cwd.display()),
            output,
        }
    }
}

pub struct Run {
    command: String,
    output: Output,
}

impl Run {
    pub fn success(&self) -> &Self {
        assert!(self.output.status.success(), "{self}");
        self
    }

    pub fn failure(&self) -> &Self {
        assert!(!self.output.status.success(), "expected failure\n{self}");
        self
    }

    pub fn stdout(&self, text: &str) -> &Self {
        assert!(
            String::from_utf8_lossy(&self.output.stdout).contains(text),
            "missing stdout {text:?}\n{self}"
        );
        self
    }

    pub fn stderr(&self, text: &str) -> &Self {
        assert!(
            String::from_utf8_lossy(&self.output.stderr).contains(text),
            "missing stderr {text:?}\n{self}"
        );
        self
    }

    pub fn stdout_excludes(&self, text: &str) -> &Self {
        assert!(
            !String::from_utf8_lossy(&self.output.stdout).contains(text),
            "unexpected stdout {text:?}\n{self}"
        );
        self
    }
}

impl fmt::Display for Run {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            self.command,
            self.output.status,
            String::from_utf8_lossy(&self.output.stdout),
            String::from_utf8_lossy(&self.output.stderr)
        )
    }
}
