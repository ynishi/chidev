//! The project: where its root is, the settings files that belong to it, and where its store is kept. The
//! settings are `chidev.toml` in the project's root (the project's choices, kept with it) and
//! `.chidev/chidev.local.toml` (the person's own, not kept with the project). Both are read here, once, and the
//! readers of the adapters (the policy, the external checks, the measure runner) are given what a section of them
//! says, so no reader opens a settings file or knows its name.
//!
//! A file is a set of top-level tables, the sections. This module is the one place that lists them and says which
//! file may hold which: `store`, `checks`, `external` and `measure` in `chidev.toml`; `store` and `checks` in the
//! person's file, because a person may change a level or where the store is on their machine but cannot register a
//! program to run or a command to build with. What is inside a section is its reader's. A key that is not a section
//! of the file it is in, and a section whose value is not a table, is refused with the file's path.
//!
//! ```toml
//! [store]
//! path = "var/chidev.db"   # relative to the project's root; an absolute path is kept as it is
//! ```
//!
//! # The root
//!
//! The root is the nearest directory, from the one `chidev` runs in and then each parent in turn, that has a
//! `chidev.toml` *file* (it may be empty; a directory of that name marks nothing). Paths in the files and paths
//! inside an operation (`source_file`, `code_file`) are relative to it.
//!
//! - There is no fallback to the current directory. A command that could start from a directory without a
//!   `chidev.toml` would silently read other settings and open another store, depending on where it happened to
//!   run; it stops instead, and says what marks a root.
//! - There is no boundary such as `.git`: chidev depends on no Git, and a project need not be in one.
//! - The settings files were `checks.toml` and `.chidev/checks.local.toml` before. They are not read, and one in a
//!   directory from the start up to the root stops the command with the name to give it, rather than being dropped
//!   in silence (a directory with only `checks.toml` would otherwise join a parent's project).
//! - A parent's `chidev.toml` is found when the project has none, as Cargo finds a parent's `Cargo.toml`. A project
//!   inside another is told apart by giving it a `chidev.toml` of its own.
//! - An argument on the command line (a file to run, `--store`) is relative to the directory the command runs in,
//!   as the shell completes it. A path inside an operation or a settings file is relative to the root, so the files
//!   an operation names mean the same from any directory of the project.
//!
//! # The store
//!
//! Where the store is kept is, first to last: `--store` on the command line (the command's, not decided here), the
//! `[store] path` of the person's file, the `[store] path` of `chidev.toml`, and `.chidev/chidev.db` under the
//! root. The person's file comes before the project's because where a database lives changes from one machine to
//! the next, and the project's value is only the default for everyone.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use thiserror::Error;

/// The project's settings file, the one that marks the project's root.
pub const FILE: &str = "chidev.toml";

/// The person's settings file, relative to the project's root.
pub const PERSON_FILE: &str = ".chidev/chidev.local.toml";

/// Where the store is kept when no file says, relative to the project's root.
const DEFAULT_STORE: &str = ".chidev/chidev.db";

/// The names the settings files had before, each with the name it has now. They are no longer read, and one found on
/// the way to the root stops the search: left alone, its directory would join a parent's project, or its settings
/// would silently be dropped.
const RENAMED: [(&str, &str); 2] = [
    ("checks.toml", FILE),
    (".chidev/checks.local.toml", PERSON_FILE),
];

/// A top-level table of a settings file. Each reader asks the project for its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    /// Where the store is kept.
    Store,
    /// The levels and parameters of the checks.
    Checks,
    /// The checks registered as programs.
    External,
    /// How a project is built, tested and measured.
    Measure,
}

impl Section {
    const ALL: [Section; 4] = [
        Section::Store,
        Section::Checks,
        Section::External,
        Section::Measure,
    ];

    /// The key of the table in a file.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Section::Store => "store",
            Section::Checks => "checks",
            Section::External => "external",
            Section::Measure => "measure",
        }
    }

    fn of(key: &str) -> Option<Section> {
        Section::ALL.into_iter().find(|s| s.name() == key)
    }
}

/// One of the two settings files.
#[derive(Debug, Clone, Copy)]
enum Settings {
    Project,
    Person,
}

impl Settings {
    /// Whether this file may hold the section.
    fn allows(self, section: Section) -> bool {
        match self {
            Settings::Project => match section {
                Section::Store | Section::Checks | Section::External | Section::Measure => true,
            },
            Settings::Person => match section {
                Section::Store | Section::Checks => true,
                Section::External | Section::Measure => false,
            },
        }
    }
}

/// Why the project cannot be found or its settings cannot be read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProjectError {
    /// No directory from `start` upwards has a `chidev.toml`.
    #[error(
        "no chidev.toml in {} or any parent directory; one (it may be empty) marks the project's root",
        start.display()
    )]
    NotFound {
        /// Where the search began, as it was given.
        start: PathBuf,
    },
    /// A settings file under the name it had before is in a directory from `start` up to the root.
    #[error("{}: this file is no longer read; its settings are now in `{now}`, so rename it", path.display())]
    Renamed {
        /// The file under its old name.
        path: PathBuf,
        /// Its name now, relative to the directory it marks.
        now: &'static str,
    },
    /// A settings file exists and cannot be read.
    #[error("{}: {message}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// What the system said.
        message: String,
    },
    /// A settings file is not what a settings file is.
    #[error("{}: {message}", path.display())]
    Invalid {
        /// The file.
        path: PathBuf,
        /// What is wrong.
        message: String,
    },
}

/// A project: its root, its settings read once, and where its store is. See the module's documentation for how each
/// is decided.
#[derive(Debug, Clone)]
pub struct Project {
    root: PathBuf,
    file: PathBuf,
    person_file: PathBuf,
    project: toml::Table,
    person: toml::Table,
    store: PathBuf,
}

/// The project that `start` is in: the nearest of `start` and its parents that has a `chidev.toml`, with its settings
/// read. No `chidev.toml` anywhere up is an error, not the current directory.
pub fn find(start: &Path) -> Result<Project, ProjectError> {
    // Walk the real path, so `..` and a relative `start` lead to the parents they mean; a `start` that does not
    // exist is walked from its absolute form and finds what its existing parents have (a relative one walked as given
    // would end at the empty path, which names no directory).
    let from = start
        .canonicalize()
        .or_else(|_| std::path::absolute(start))
        .unwrap_or_else(|_| start.to_path_buf());
    let mut found = None;
    for dir in from.ancestors() {
        if let Some((old, now)) = RENAMED.iter().find(|(old, _)| dir.join(old).is_file()) {
            return Err(ProjectError::Renamed {
                path: dir.join(old),
                now,
            });
        }
        if dir.join(FILE).is_file() {
            found = Some(dir);
            break;
        }
    }
    let Some(dir) = found else {
        return Err(ProjectError::NotFound {
            start: start.to_path_buf(),
        });
    };
    let root = dir.canonicalize().map_err(|e| ProjectError::Read {
        path: dir.to_path_buf(),
        message: e.to_string(),
    })?;
    let file = root.join(FILE);
    let person_file = root.join(PERSON_FILE);
    let project = read(&file, Settings::Project)?.unwrap_or_default();
    let person = read(&person_file, Settings::Person)?.unwrap_or_default();
    let store = store_of(&person, &person_file)?
        .or(store_of(&project, &file)?)
        .map_or_else(|| root.join(DEFAULT_STORE), |path| root.join(path));
    Ok(Project {
        root,
        file,
        person_file,
        project,
        person,
        store,
    })
}

impl Project {
    /// The project's root, a real path.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project's settings file, for a message that says where a setting is wrong.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Where the store is kept, when the command line does not say: an absolute path, or one under the root.
    pub fn store(&self) -> &Path {
        &self.store
    }

    /// The person's settings file, whether or not it is there.
    pub(crate) fn person_file(&self) -> &Path {
        &self.person_file
    }

    /// What `section` says in the project's file.
    pub(crate) fn section(&self, section: Section) -> Option<&toml::Value> {
        self.project.get(section.name())
    }

    /// What `section` says in the person's file.
    pub(crate) fn person_section(&self, section: Section) -> Option<&toml::Value> {
        self.person.get(section.name())
    }
}

/// The table `path` holds, or `None` when the file is not there.
fn read(path: &Path, settings: Settings) -> Result<Option<toml::Table>, ProjectError> {
    let invalid = |message: String| ProjectError::Invalid {
        path: path.to_path_buf(),
        message,
    };
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(ProjectError::Read {
                path: path.to_path_buf(),
                message: e.to_string(),
            });
        }
    };
    let table: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| invalid(e.to_string()))?;
    for (key, value) in &table {
        let Some(section) = Section::of(key).filter(|s| settings.allows(*s)) else {
            let there = Section::ALL
                .into_iter()
                .filter(|s| settings.allows(*s))
                .map(|s| format!("`{}`", s.name()))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(invalid(format!(
                "`{key}` is not a section of this file; there are {there}"
            )));
        };
        if !value.is_table() {
            return Err(invalid(format!("`{}` is a table", section.name())));
        }
    }
    Ok(Some(table))
}

/// The `[store] path` of `table`, read from the file at `path`.
fn store_of(table: &toml::Table, path: &Path) -> Result<Option<String>, ProjectError> {
    let invalid = |message: String| ProjectError::Invalid {
        path: path.to_path_buf(),
        message,
    };
    let Some(store) = table.get(Section::Store.name()).and_then(|v| v.as_table()) else {
        return Ok(None);
    };
    if let Some(key) = store.keys().find(|k| k.as_str() != "path") {
        return Err(invalid(format!(
            "`store.{key}` is not a setting; there is `path`"
        )));
    }
    match store.get("path") {
        None => Ok(None),
        Some(toml::Value::String(path)) if !path.is_empty() => Ok(Some(path.clone())),
        Some(_) => Err(invalid(
            "`store.path` is a path, a string that is not empty".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A fresh directory under the system's temporary one, removed when dropped.
    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "chidev-project-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn write(&self, relative: &str, text: &str) {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }

        fn real(&self) -> PathBuf {
            self.0.canonicalize().unwrap()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn invalid(result: Result<Project, ProjectError>) -> String {
        match result.unwrap_err() {
            ProjectError::Invalid { message, .. } => message,
            other => panic!("not Invalid: {other:?}"),
        }
    }

    #[test]
    fn the_root_is_the_start_when_it_has_the_file() {
        let dir = Dir::new();
        dir.write(FILE, "");
        let project = find(&dir.0).unwrap();
        assert_eq!(project.root(), dir.real());
        assert_eq!(project.file(), dir.real().join(FILE));
    }

    #[test]
    fn the_root_is_found_in_a_parent_and_the_nearest_wins() {
        let dir = Dir::new();
        dir.write(FILE, "");
        dir.write("a/b/keep", "");
        assert_eq!(find(&dir.0.join("a/b")).unwrap().root(), dir.real());
        dir.write("a/chidev.toml", "");
        assert_eq!(
            find(&dir.0.join("a/b")).unwrap().root(),
            dir.real().join("a")
        );
        assert_eq!(find(&dir.0).unwrap().root(), dir.real());
    }

    #[test]
    fn a_directory_named_like_the_file_marks_nothing() {
        let dir = Dir::new();
        dir.write(FILE, "");
        std::fs::create_dir_all(dir.0.join("a").join(FILE)).unwrap();
        assert_eq!(find(&dir.0.join("a")).unwrap().root(), dir.real());
    }

    #[test]
    fn a_settings_file_under_its_old_name_on_the_way_up_stops_the_search() {
        let dir = Dir::new();
        dir.write(FILE, "");
        dir.write("a/checks.toml", "[measure]\n");
        assert_eq!(
            find(&dir.0.join("a")).unwrap_err(),
            ProjectError::Renamed {
                path: dir.real().join("a/checks.toml"),
                now: FILE
            }
        );
        dir.write(".chidev/checks.local.toml", "");
        let error = find(&dir.0).unwrap_err();
        assert_eq!(
            error,
            ProjectError::Renamed {
                path: dir.real().join(".chidev/checks.local.toml"),
                now: PERSON_FILE
            }
        );
        assert!(error.to_string().ends_with(
            "this file is no longer read; its settings are now in `.chidev/chidev.local.toml`, so rename it"
        ));
        // above the root is not the project's, and is not looked at
        std::fs::remove_file(dir.0.join(".chidev/checks.local.toml")).unwrap();
        dir.write("a/chidev.toml", "");
        std::fs::remove_file(dir.0.join("a/checks.toml")).unwrap();
        dir.write("checks.toml", "");
        assert_eq!(find(&dir.0.join("a")).unwrap().root(), dir.real().join("a"));
    }

    #[test]
    fn a_missing_start_is_walked_from_its_existing_parents() {
        let dir = Dir::new();
        dir.write(FILE, "");
        assert_eq!(find(&dir.0.join("gone/deeper")).unwrap().root(), dir.real());
        // a relative one is walked from where it is meant to be, not given up at the empty path
        let relative = Path::new("chidev-project-missing/deeper");
        let here = std::env::current_dir().unwrap();
        match here.ancestors().find(|d| d.join(FILE).is_file()) {
            Some(root) => assert_eq!(find(relative).unwrap().root(), root.canonicalize().unwrap()),
            None => assert_eq!(
                find(relative).unwrap_err(),
                ProjectError::NotFound {
                    start: relative.to_path_buf()
                }
            ),
        }
    }

    #[test]
    fn no_file_anywhere_is_not_found_and_the_message_shows_the_start_as_given() {
        let dir = Dir::new();
        if dir.0.ancestors().any(|d| d.join(FILE).is_file()) {
            eprintln!("skipped: a {FILE} above the temporary directory marks a root");
            return;
        }
        let start = dir.0.join("a/../b");
        let error = find(&start).unwrap_err();
        assert_eq!(
            error,
            ProjectError::NotFound {
                start: start.clone()
            }
        );
        assert_eq!(
            error.to_string(),
            format!(
                "no chidev.toml in {} or any parent directory; one (it may be empty) marks the project's root",
                start.display()
            )
        );
    }

    #[test]
    fn a_section_that_is_not_known_is_refused_by_name_in_either_file() {
        let dir = Dir::new();
        dir.write(FILE, "[provider]\nx = 1\n");
        let message = invalid(find(&dir.0));
        assert!(message.contains("`provider` is not a section"), "{message}");
        dir.write(FILE, "");
        dir.write(PERSON_FILE, "[provider]\nx = 1\n");
        let message = invalid(find(&dir.0));
        assert!(message.contains("`provider` is not a section"), "{message}");
    }

    #[test]
    fn the_person_may_not_register_programs_or_measure() {
        let dir = Dir::new();
        dir.write(FILE, "");
        for section in [Section::External, Section::Measure] {
            dir.write(PERSON_FILE, &format!("[{}]\n", section.name()));
            let message = invalid(find(&dir.0));
            assert!(
                message.contains(&format!("`{}` is not a section", section.name())),
                "{message}"
            );
            assert!(message.contains("`store`, `checks`"), "{message}");
        }
        dir.write(PERSON_FILE, "[checks]\n[store]\n");
        assert!(find(&dir.0).is_ok());
    }

    #[test]
    fn the_project_may_hold_every_section_and_each_is_a_table() {
        let dir = Dir::new();
        dir.write(FILE, "[store]\n[checks]\n[external.\"ext.a\"]\n[measure]\n");
        let project = find(&dir.0).unwrap();
        for section in Section::ALL {
            assert!(project.section(section).is_some(), "{}", section.name());
            assert_eq!(Section::of(section.name()), Some(section));
        }
        assert!(project.person_section(Section::Checks).is_none());
        dir.write(FILE, "measure = 1\n");
        let message = invalid(find(&dir.0));
        assert!(message.contains("`measure` is a table"), "{message}");
        dir.write(FILE, "[checks\n");
        assert!(invalid(find(&dir.0)).contains("expected"));
    }

    #[test]
    fn the_store_is_the_persons_then_the_projects_then_the_default() {
        let dir = Dir::new();
        let real = dir.real();
        dir.write(FILE, "");
        assert_eq!(
            find(&dir.0).unwrap().store(),
            real.join(".chidev/chidev.db")
        );
        dir.write(FILE, "[store]\npath = \"var/project.db\"\n");
        assert_eq!(find(&dir.0).unwrap().store(), real.join("var/project.db"));
        dir.write(PERSON_FILE, "[store]\npath = \"mine.db\"\n");
        assert_eq!(find(&dir.0).unwrap().store(), real.join("mine.db"));
        dir.write(FILE, "");
        assert_eq!(find(&dir.0).unwrap().store(), real.join("mine.db"));
    }

    #[test]
    fn an_absolute_store_is_kept_and_a_relative_one_is_from_the_root_not_the_start() {
        let dir = Dir::new();
        dir.write("sub/keep", "");
        let elsewhere = std::env::temp_dir().join("chidev-project-elsewhere.db");
        dir.write(
            FILE,
            &format!("[store]\npath = {:?}\n", elsewhere.to_str().unwrap()),
        );
        assert_eq!(find(&dir.0.join("sub")).unwrap().store(), elsewhere);
        dir.write(FILE, "[store]\npath = \"x/y.db\"\n");
        assert_eq!(
            find(&dir.0.join("sub")).unwrap().store(),
            dir.real().join("x/y.db")
        );
    }

    #[test]
    fn the_store_table_has_only_a_path_that_is_a_string() {
        let dir = Dir::new();
        for (text, expect) in [
            ("[store]\nurl = \"x\"\n", "`store.url` is not a setting"),
            ("[store]\npath = 3\n", "`store.path` is a path"),
            ("[store]\npath = \"\"\n", "`store.path` is a path"),
        ] {
            dir.write(FILE, text);
            let message = invalid(find(&dir.0));
            assert!(message.contains(expect), "{text:?} gave {message:?}");
            dir.write(FILE, "");
            dir.write(PERSON_FILE, text);
            let message = invalid(find(&dir.0));
            assert!(message.contains(expect), "{text:?} gave {message:?}");
            std::fs::remove_file(dir.0.join(PERSON_FILE)).unwrap();
        }
    }
}
