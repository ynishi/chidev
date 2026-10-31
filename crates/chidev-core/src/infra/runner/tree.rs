//! The working tree, as the [`WorkingTree`] port: the copies of the placed assets are the files the project's
//! `[[measure.layout]]` places them in, read as text and digested. It lives in the runner because the layout does:
//! a path here is the one the table was loaded with, which was checked to stay under the project's root, and is not
//! checked a second time.
//!
//! It reads files and starts no program. The bytes are digested as they are, after they are known to be UTF-8, which
//! is how the text of a Pin is made (the text of a `code_file` is its bytes), so the digest of a copy and the digest
//! of a Pin are equal exactly when the texts are.

use std::collections::{BTreeMap, BTreeSet};
use std::io::ErrorKind;
use std::path::Path;

use super::ProcessRunner;
use crate::app::ports::measure::BoxFuture;
use crate::app::ports::tree::{TreeError, TreeTarget, WorkingTree};
use crate::domain::vo::digest::Digest;
use crate::domain::vo::tree::CopyRead;

impl WorkingTree for ProcessRunner {
    fn digests_of<'a>(
        &'a self,
        targets: &'a [TreeTarget],
    ) -> BoxFuture<'a, Vec<(TreeTarget, CopyRead)>> {
        Box::pin(async move {
            let placed: Vec<(&TreeTarget, Option<&Path>)> = targets
                .iter()
                .map(|target| (target, self.place_of(target)))
                .collect();
            read_copies(&self.root, &placed).await
        })
    }
}

impl ProcessRunner {
    /// The path, relative to the root, of the file the layout places `target` in; none when it places it nowhere.
    fn place_of(&self, target: &TreeTarget) -> Option<&Path> {
        match target {
            TreeTarget::Interface { context, name } => self
                .config
                .interface(context, name)
                .map(|(layout, _)| layout.path.as_path()),
            TreeTarget::Harness { context, name } => self
                .config
                .harness(context, name)
                .map(|layout| layout.path.as_path()),
        }
    }
}

/// The files to read for `targets`: each path once, whatever the number of targets that name it.
fn plan<'a>(targets: &[(&TreeTarget, Option<&'a Path>)]) -> BTreeSet<&'a Path> {
    targets.iter().filter_map(|(_, path)| *path).collect()
}

/// What was read of the file at `path`, a path relative to `root`: a file that is not there is missing, one that
/// cannot be read or is not UTF-8 text is unreadable, and any other is read.
async fn read_copy(root: &Path, path: &Path) -> CopyRead {
    let shown = path.display().to_string();
    match tokio::fs::read(root.join(path)).await {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => CopyRead::Read(Digest::of(text.as_bytes())),
            Err(_) => CopyRead::Unreadable(TreeError::NotText { path: shown }.to_string()),
        },
        Err(e) if e.kind() == ErrorKind::NotFound => CopyRead::Missing,
        Err(e) => CopyRead::Unreadable(
            TreeError::Io {
                path: shown,
                message: e.to_string(),
            }
            .to_string(),
        ),
    }
}

/// What was read of the copy of each of `targets`, each beside its target, with the path it is laid out at (none:
/// nowhere, so it is unplaced). The files of `root` are read once for the whole call, each path once: several targets
/// that share a file get the same answer from one read.
async fn read_copies(
    root: &Path,
    targets: &[(&TreeTarget, Option<&Path>)],
) -> Vec<(TreeTarget, CopyRead)> {
    let mut read: BTreeMap<&Path, CopyRead> = BTreeMap::new();
    for path in plan(targets) {
        read.insert(path, read_copy(root, path).await);
    }
    targets
        .iter()
        .map(|(target, path)| {
            let copy = path
                .and_then(|path| read.get(path))
                .cloned()
                .unwrap_or(CopyRead::Unplaced);
            ((*target).clone(), copy)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::domain::vo::name::{AssetName, ContextName};

    static NEXT: AtomicUsize = AtomicUsize::new(0);

    /// A directory of its own under cargo's temporary one, removed when it is dropped.
    struct Root(PathBuf);

    impl Root {
        fn new() -> Root {
            let dir = std::env::temp_dir().join(format!(
                "chidev-tree-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Root(dir)
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn interface(name: &str) -> TreeTarget {
        TreeTarget::Interface {
            context: ContextName::try_from("core").unwrap(),
            name: AssetName::try_from(name).unwrap(),
        }
    }

    fn harness(name: &str) -> TreeTarget {
        TreeTarget::Harness {
            context: ContextName::try_from("core").unwrap(),
            name: AssetName::try_from(name).unwrap(),
        }
    }

    #[tokio::test]
    async fn a_file_is_read_missing_or_unreadable_and_a_target_with_no_file_is_unplaced() {
        let root = Root::new();
        std::fs::write(root.0.join("api.rs"), "fn push() {}").unwrap();
        std::fs::write(root.0.join("binary.rs"), [0xff, 0xfe, 0x00]).unwrap();
        std::fs::create_dir(root.0.join("folder")).unwrap();
        let (api, gone, binary, folder, nowhere) = (
            interface("api"),
            interface("gone"),
            interface("binary"),
            interface("folder"),
            harness("nowhere"),
        );
        let said = read_copies(
            &root.0,
            &[
                (&api, Some(Path::new("api.rs"))),
                (&gone, Some(Path::new("gone.rs"))),
                (&binary, Some(Path::new("binary.rs"))),
                (&folder, Some(Path::new("folder"))),
                (&nowhere, None),
            ],
        )
        .await;
        let copies: Vec<&CopyRead> = said.iter().map(|(_, copy)| copy).collect();
        assert_eq!(said[0].0, api, "each answer is beside its target");
        assert_eq!(copies[0], &CopyRead::Read(Digest::of(b"fn push() {}")));
        assert_eq!(copies[1], &CopyRead::Missing);
        assert_eq!(
            copies[2],
            &CopyRead::Unreadable("binary.rs: not UTF-8 text".to_string())
        );
        // a directory is a file that cannot be read, and is not told as a program that is not to be run
        let CopyRead::Unreadable(why) = copies[3] else {
            panic!("a directory is unreadable: {:?}", copies[3]);
        };
        assert!(why.starts_with("folder: "), "{why}");
        assert!(
            !why.contains("program") && !why.contains("executable"),
            "{why}"
        );
        assert_eq!(copies[4], &CopyRead::Unplaced);
    }

    #[tokio::test]
    async fn two_targets_that_name_one_file_read_it_once_and_get_the_same_answer() {
        let root = Root::new();
        std::fs::write(root.0.join("shared.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        let (a, b, c) = (interface("a"), interface("b"), harness("t"));
        let targets = [
            (&a, Some(Path::new("shared.rs"))),
            (&b, Some(Path::new("shared.rs"))),
            (&c, Some(Path::new("other.rs"))),
        ];
        // the plan holds each path once, whatever number of targets name it
        assert_eq!(
            plan(&targets).into_iter().collect::<Vec<_>>(),
            [Path::new("other.rs"), Path::new("shared.rs")]
        );
        let said = read_copies(&root.0, &targets).await;
        assert_eq!(said.len(), 3);
        assert_eq!(said[0].1, said[1].1);
        assert_eq!(
            said[0].1,
            CopyRead::Read(Digest::of(b"fn a() {}\nfn b() {}\n"))
        );
        assert_eq!(said[2].1, CopyRead::Missing);
    }

    #[tokio::test]
    async fn the_digest_of_a_copy_is_the_digest_a_pin_makes_of_the_same_file() {
        let root = Root::new();
        // bytes that a normaliser would change: a CRLF, a missing last line feed, a non-ASCII character
        let text = "fn push() {}\r\n// k\u{e9}\ty";
        std::fs::write(root.0.join("api.rs"), text).unwrap();
        // what `code_file` gives an operation is the file's bytes as a String, and the Pin digests its bytes
        let as_pinned = String::from_utf8(std::fs::read(root.0.join("api.rs")).unwrap()).unwrap();
        let api = interface("api");
        let said = read_copies(&root.0, &[(&api, Some(Path::new("api.rs")))]).await;
        assert_eq!(said[0].1, CopyRead::Read(Digest::of(as_pinned.as_bytes())));
    }
}
