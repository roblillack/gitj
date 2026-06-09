//! Integration test for image-blob retrieval against the live [`Git2Backend`]:
//! the bytes [`commit_file_blobs`](RepoBackend::commit_file_blobs) and
//! [`working_file_blobs`](RepoBackend::working_file_blobs) return must match the
//! exact blob content git holds for each side, so the graphical diff compares
//! the right two images.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{Repository, Signature, Time};
use journey::backend::{Git2Backend, RepoBackend};
use journey::imagediff::ImageComparison;

fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("journey-{tag}-{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// A `w`×`h` solid-color PNG.
fn png(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(color));
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(img)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

#[test]
fn reads_image_blobs_from_commits_and_working_tree() {
    let dir = scratch_dir("imgblob");
    let repo = Repository::init(&dir).unwrap();
    let sig = Signature::new("Tester", "tester@example.com", &Time::new(1_700_000_000, 0)).unwrap();

    let v1 = png(8, 8, [255, 0, 0, 255]); // red
    let v2 = png(8, 8, [0, 255, 0, 255]); // green
    let v3 = png(8, 8, [0, 0, 255, 255]); // blue

    // Commit v1, then commit v2 over it, so there are two commits and the newest
    // (index 0) has v1 as its parent's blob.
    let commit_file = |content: &[u8], msg: &str, parent: Option<git2::Oid>| -> git2::Oid {
        fs::write(dir.join("logo.png"), content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new("logo.png")).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let parents: Vec<git2::Commit> =
            parent.into_iter().map(|p| repo.find_commit(p).unwrap()).collect();
        let refs: Vec<&git2::Commit> = parents.iter().collect();
        repo.commit(Some("HEAD"), &sig, &sig, msg, &tree, &refs)
            .unwrap()
    };
    let c1 = commit_file(&v1, "add logo\n", None);
    let _c2 = commit_file(&v2, "recolor logo\n", Some(c1));

    let backend = Git2Backend::open(dir.to_str().unwrap()).expect("open repo");

    // Newest commit: new side is v2, old side (its parent) is v1.
    let commit_blobs = backend.commit_file_blobs(0, "logo.png");
    assert_eq!(commit_blobs.new.as_deref(), Some(v2.as_slice()));
    assert_eq!(commit_blobs.old.as_deref(), Some(v1.as_slice()));
    assert!(
        ImageComparison::from_blobs(&commit_blobs).is_some(),
        "both sides decode to images"
    );

    // First commit (index 1): added — no parent blob.
    let added = backend.commit_file_blobs(1, "logo.png");
    assert_eq!(added.new.as_deref(), Some(v1.as_slice()));
    assert_eq!(added.old, None);

    // Edit the working tree (v3, unstaged): old = index copy (v2), new = disk (v3).
    fs::write(dir.join("logo.png"), &v3).unwrap();
    let unstaged = backend.working_file_blobs("logo.png", false, false);
    assert_eq!(unstaged.old.as_deref(), Some(v2.as_slice()));
    assert_eq!(unstaged.new.as_deref(), Some(v3.as_slice()));

    // Stage it through the backend (so its own index is current, as in the
    // app): old = staged base (HEAD tree, v2), new = index (v3).
    backend.stage("logo.png").expect("stage");
    let staged = backend.working_file_blobs("logo.png", true, false);
    assert_eq!(staged.old.as_deref(), Some(v2.as_slice()));
    assert_eq!(staged.new.as_deref(), Some(v3.as_slice()));

    fs::remove_dir_all(&dir).ok();
}
