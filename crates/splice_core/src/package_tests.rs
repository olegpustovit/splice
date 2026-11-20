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
    // SPLICE_BLESS=1 rewrites the sample in the current format (reviewed as a diff).
    if std::env::var_os("SPLICE_BLESS").is_some() {
        fs::copy(path.join("project.json"), sample.join("project.json")).unwrap();
    }
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
    let locked = Package::open(&path).unwrap_err();
    assert_eq!(locked.code, "LOCKED");
    // The holder is named so an agent can route its call through it.
    assert!(
        locked
            .message
            .contains(&format!("process {}", std::process::id())),
        "{}",
        locked.message
    );
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

/// 0.3 → 0.4 (D12): motion lanes load as camera lanes, motion items as zoom segments that hold
/// their end scale, and click-zoom effects are dropped (intent.autoZoom makes explicit zooms).
#[test]
fn a_0_3_project_migrates_motion_click_zoom_and_unanimated_keys() {
    let temp = Temp::new("migrate-0.3");
    let path = temp.project();
    fs::create_dir_all(&path).unwrap();
    let project = r#"{"format":"splice/1","rev":0,"hash":"0000000000000001","serial":9,"fps":30,"size":[1080,1920],"aspect":"9:16","access":"live",
"effects":[{"id":"fx_1","type":"clickZoom","params":{"scale":1.6}},{"id":"fx_5","type":"window","params":{"padding":{"keys":[{"at":0,"value":0.12,"ease":"linear"},{"at":9,"value":0.2,"ease":"linear"}]}}}],
"spine":{"id":"lan_spine","name":"Video","effects":[]},
"lanes":[{"id":"lan_2","name":"Motion","kind":"motion","effects":[]}],
"clips":[{"id":"clp_3","media":null,"start":0,"length":120,"source":0,"speed":1,"fadeIn":0,"fadeOut":0,"volume":{"mic":1,"system":0.8},"enabled":{},"crop":null,"fit":"fit","effects":[]}],
"items":[{"id":"ovl_4","kind":"motion","lane":"lan_2","anchor":{"pinned":true,"frame":10},"length":60,"params":{"from":1,"to":1.4,"focus":{"x":0.25,"y":0.75},"ease":true},"effects":[]}],
"markers":[]}"#;
    fs::write(path.join("project.json"), project).unwrap();
    fs::write(
        path.join("media.json"),
        r#"{"format":"splice/1","media":[]}"#,
    )
    .unwrap();
    let (_, loaded) = Package::open(&path).unwrap();
    assert_eq!(loaded.lanes[0].kind, splice_model::LaneKind::Camera);
    // clickZoom stays as inert data (nothing renders it; 0.3 journals replay onto it); keys on
    // a non-keyable parameter keep the first value (T36.4).
    assert_eq!(
        loaded
            .effects
            .iter()
            .map(|e| e.kind.as_str())
            .collect::<Vec<_>>(),
        ["clickZoom", "window"]
    );
    assert_eq!(
        loaded.effects[1].params[0].value,
        splice_model::EffectValue::Number(0.12)
    );
    let slot = loaded.items.order()[0];
    let splice_model::ItemKind::Zoom(zoom) = loaded.items.data(slot).unwrap().kind else {
        panic!("motion became a zoom")
    };
    assert_eq!((zoom.amount, zoom.focus), (1.4, Some([0.25, 0.75])));
}

/// Review #4: a zoom item outside the camera's range is refused on load with the rule, never
/// opened and then failing every render.
#[test]
fn zoom_items_out_of_range_are_refused_on_load() {
    for (params, field) in [
        (r#"{"amount":7}"#, "zoom.amount"),
        (r#"{"amount":2,"focus":{"x":1.5,"y":0.5}}"#, "zoom.focus"),
        (r#"{"amount":2,"spring":3}"#, "zoom.mode/spring"),
    ] {
        let temp = Temp::new("zoom-range");
        let path = temp.project();
        fs::create_dir_all(&path).unwrap();
        let project = format!(
            r#"{{"format":"splice/1","rev":0,"hash":"0000000000000001","serial":9,"fps":30,"size":[1080,1920],"aspect":"9:16","access":"live",
"effects":[],"spine":{{"id":"lan_spine","name":"Video","effects":[]}},
"lanes":[{{"id":"lan_2","name":"Camera","kind":"camera","effects":[]}}],
"clips":[{{"id":"clp_3","media":null,"start":0,"length":120,"source":0,"speed":1,"fadeIn":0,"fadeOut":0,"volume":{{"mic":1,"system":0.8}},"enabled":{{}},"crop":null,"fit":"fit","effects":[]}}],
"items":[{{"id":"ovl_4","kind":"zoom","lane":"lan_2","anchor":{{"pinned":true,"frame":10}},"length":60,"params":{params},"effects":[]}}],
"markers":[]}}"#
        );
        fs::write(path.join("project.json"), project).unwrap();
        fs::write(
            path.join("media.json"),
            r#"{"format":"splice/1","media":[]}"#,
        )
        .unwrap();
        let error = Package::open(&path).unwrap_err();
        assert!(error.message.contains(field), "{params}: {}", error.message);
    }
}
