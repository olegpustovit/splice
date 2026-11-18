use crate::package::Package;
use splice_model::Project;
use std::fs;
use std::path::{Path, PathBuf};

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("splice-{}-{name}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn project(&self) -> PathBuf {
        self.0.join("new.splice")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn package_round_trip_preserves_hash_and_every_sample_file() {
    let temp = Temp::new("roundtrip");
    let sample =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
    let path = temp.project();
    fs::create_dir_all(path.join("transcripts")).unwrap();
    let transcript = "transcripts/med_screen.json";
    for file in ["project.json", "media.json", transcript] {
        fs::copy(sample.join(file), path.join(file)).unwrap();
    }
    let (mut package, first) = Package::open(&path).unwrap();
    assert_eq!(first.clips.order().len(), 3);
    assert_eq!(first.items.order().len(), 15);
    assert_eq!(first.media.len(), 6);
    assert_ne!(first.hash, 0);
    assert_eq!(first.transcripts[0].words.len(), 36);
    package.save(&first).unwrap();
    drop(package);
    let (_, loaded) = Package::open(&path).unwrap();
    assert_eq!(first.hash, loaded.hash);
    for file in ["project.json", "media.json", transcript] {
        assert_eq!(
            fs::read(path.join(file)).unwrap(),
            fs::read(sample.join(file)).unwrap(),
            "{file}"
        );
    }
}

#[test]
fn package_writer_lock_has_owner_and_releases() {
    let temp = Temp::new("lock");
    let path = temp.project();
    let held = Package::create(&path, &Project::default()).unwrap();
    let owner = fs::read_to_string(path.join("lock")).unwrap();
    let owner = splice_json::parse(&owner).unwrap();
    assert_eq!(
        owner.field("pid").unwrap().as_u64(),
        Some(u64::from(std::process::id()))
    );
    assert!(!owner.field("host").unwrap().as_str().unwrap().is_empty());
    assert!(owner.field("since").unwrap().as_u64().unwrap() > 0);
    assert_eq!(Package::open(&path).unwrap_err().code, "LOCKED");
    drop(held);
    assert!(Package::open(&path).is_ok());
}

#[test]
fn package_refuses_newer_major_and_unknown_fields() {
    let temp = Temp::new("versions");
    let path = temp.project();
    drop(Package::create(&path, &Project::default()).unwrap());
    let file = path.join("project.json");
    let original = fs::read_to_string(&file).unwrap();
    fs::write(&file, original.replace("splice/1", "splice/2")).unwrap();
    assert_eq!(Package::open(&path).unwrap_err().code, "FORMAT_TOO_NEW");
    fs::write(&file, original.replacen('{', "{\"typo\":0,", 1)).unwrap();
    assert_eq!(Package::open(&path).unwrap_err().code, "FORMAT_UNKNOWN_KEY");
}

#[test]
fn unsafe_lock_links_leave_target_untouched() {
    let temp = Temp::new("lock-link");
    let path = temp.project();
    drop(Package::create(&path, &Project::default()).unwrap());
    let lock = path.join("lock");
    let target = temp.0.join("unrelated");
    fs::write(&target, "keep me").unwrap();
    fs::remove_file(&lock).unwrap();
    std::os::unix::fs::symlink(&target, &lock).unwrap();
    assert_eq!(Package::open(&path).unwrap_err().code, "BAD_PACKAGE");
    assert_eq!(fs::read_to_string(&target).unwrap(), "keep me");
}

#[test]
fn package_save_keeps_published_snapshot_until_valid_replacement() {
    let temp = Temp::new("atomic");
    let path = temp.project();
    let mut project = Project::default();
    let mut package = Package::create(&path, &project).unwrap();
    let snapshot = path.join("project.json");
    let original = fs::read(&snapshot).unwrap();
    let leftover = path.join("project.json.tmp");
    fs::write(&leftover, b"partial").unwrap();

    project.fps = 0;
    assert_eq!(package.save(&project).unwrap_err().code, "BAD_PACKAGE");
    assert_eq!(fs::read(&snapshot).unwrap(), original);
    assert_eq!(fs::read(&leftover).unwrap(), b"partial");

    project.fps = 25;
    fs::remove_file(&leftover).unwrap();
    fs::create_dir(&leftover).unwrap();
    assert_eq!(package.save(&project).unwrap_err().code, "IO");
    assert_eq!(fs::read(&snapshot).unwrap(), original);

    fs::remove_dir(&leftover).unwrap();
    fs::write(&leftover, b"partial").unwrap();
    package.save(&project).unwrap();
    assert_eq!(package.load().unwrap().fps, 25);
    assert!(!leftover.exists());
    assert_eq!(Package::create(&path, &project).unwrap_err().code, "EXISTS");
}

#[test]
fn package_recovers_abandoned_lock_record() {
    let temp = Temp::new("abandoned");
    let path = temp.project();
    drop(Package::create(&path, &Project::default()).unwrap());
    let lock = path.join("lock");
    fs::write(&lock, r#"{"pid":9999,"host":"old-host","since":1}"#).unwrap();
    let (package, _) = Package::open(&path).unwrap();
    let owner = splice_json::parse(&fs::read_to_string(&lock).unwrap()).unwrap();
    assert_eq!(
        owner.field("pid").unwrap().as_u64(),
        Some(u64::from(std::process::id()))
    );
    drop(package);
    assert_eq!(fs::metadata(lock).unwrap().len(), 0);
}
