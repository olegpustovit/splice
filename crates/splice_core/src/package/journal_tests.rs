use super::PackageError;
use super::journal::*;
use splice_json::Value as Json;
use splice_model::{Aspect, Project};
use std::fs::{self, File};
use std::io::{Seek, SeekFrom, Write};
use std::path::PathBuf;

struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("splice-journal-{}-{name}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        Self(dir)
    }

    fn package(&self) -> PathBuf {
        self.0.join("test.splice")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn change_aspect(aspect: Aspect) -> Batch {
    let name = match aspect {
        Aspect::Portrait => "9:16",
        Aspect::Social => "4:5",
        Aspect::Square => "1:1",
        Aspect::Standard => "4:3",
        Aspect::Landscape => "16:9",
    };
    Batch {
        time: 12,
        source: Source::Human,
        session: None,
        label: "Set aspect".into(),
        key: None,
        change: Change::Edit {
            ops: vec![
                splice_json::parse(&format!(r#"{{"cmd":"project.set","aspect":"{name}"}}"#))
                    .unwrap(),
            ],
            mints: Vec::new(),
            imports: Vec::new(),
        },
    }
}

fn replay(
    project: &Project,
    _origin: &Project,
    prefix: &[Record],
) -> Result<Project, PackageError> {
    let Change::Edit { ops, .. } = &prefix.last().unwrap().batch.change else {
        return Err(PackageError::invalid(&"test replay expects an edit"));
    };
    let mut next = project.clone();
    for op in ops {
        next.aspect = match op.field("aspect").and_then(Json::as_str) {
            Some("9:16") => Aspect::Portrait,
            Some("4:5") => Aspect::Social,
            Some("1:1") => Aspect::Square,
            Some("4:3") => Aspect::Standard,
            Some("16:9") => Aspect::Landscape,
            _ => return Err(PackageError::invalid(&"test replay expects an aspect")),
        };
    }
    Ok(next)
}

#[test]
fn journal_crash_at_every_byte_recovers_a_complete_prefix() {
    let temp = Temp::new("crash");
    let path = temp.package();
    let base = Project::default();
    let (mut journal, mut project) = Journal::create(&path, &base).unwrap();
    let journal_path = path.join("journal.ops");
    let mut checkpoints = vec![(0, project.rev, project.hash, project.aspect)];
    for aspect in [Aspect::Square, Aspect::Social, Aspect::Landscape] {
        project.aspect = aspect;
        journal.commit(&mut project, change_aspect(aspect)).unwrap();
        checkpoints.push((
            fs::metadata(&journal_path).unwrap().len() as usize,
            project.rev,
            project.hash,
            aspect,
        ));
    }
    drop(journal);

    let bytes = fs::read(&journal_path).unwrap();
    assert_eq!(checkpoints[1].2, 0x16d9_758e_52e9_0695);
    let mut file = File::options()
        .read(true)
        .write(true)
        .open(&journal_path)
        .unwrap();
    for offset in 0..=bytes.len() {
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(&bytes[..offset]).unwrap();
        file.set_len(offset as u64).unwrap();
        let prefix = super::journal::read_prefix(&mut file, &journal_path, &base).unwrap();
        let recovered = super::journal::recover(&base, &base, prefix, &mut replay, false);
        let &(boundary, revision, hash, aspect) = checkpoints
            .iter()
            .rev()
            .find(|&&(boundary, ..)| boundary <= offset)
            .unwrap();
        assert_eq!(
            (
                recovered.project.rev,
                recovered.project.hash,
                recovered.project.aspect
            ),
            (revision, hash, aspect),
            "offset {offset}"
        );
        assert_eq!(
            recovered.torn_bytes as usize,
            offset - boundary,
            "offset {offset}"
        );
        assert!(recovered.problem.is_none(), "offset {offset}");
        assert_eq!(
            file.metadata().unwrap().len() as usize,
            boundary,
            "offset {offset}"
        );
    }
}

#[test]
fn journal_snapshots_at_sixty_four_and_replays_after_checkpoint() {
    let temp = Temp::new("checkpoint");
    let path = temp.package();
    let (mut journal, mut project) = Journal::create(&path, &Project::default()).unwrap();
    for rev in 1..=65 {
        project.aspect = if rev % 2 == 0 {
            Aspect::Square
        } else {
            Aspect::Landscape
        };
        let batch = change_aspect(project.aspect);
        let checkpoint = journal.commit(&mut project, batch).unwrap();
        assert!(matches!(checkpoint, Checkpoint::Written) == (rev == 64));
    }
    drop(journal);
    let snapshot =
        splice_json::parse(&fs::read_to_string(path.join("project.json")).unwrap()).unwrap();
    assert_eq!(snapshot.field("rev").unwrap().as_u64(), Some(64));
    let mut applied = 0;
    let (journal, recovered) = Journal::open(&path, &mut |project, origin, records| {
        applied += 1;
        replay(project, origin, records)
    })
    .unwrap();
    assert_eq!(applied, 1);
    assert_eq!(journal.records().len(), 65);
    assert_eq!(
        (recovered.rev, recovered.hash, recovered.aspect),
        (65, project.hash, Aspect::Landscape)
    );
    journal.close(&recovered).unwrap();
    let snapshot =
        splice_json::parse(&fs::read_to_string(path.join("project.json")).unwrap()).unwrap();
    assert_eq!(snapshot.field("rev").unwrap().as_u64(), Some(65));
}

#[test]
fn journal_complete_corruption_returns_last_good_project_read_only() {
    let temp = Temp::new("corrupt");
    let path = temp.package();
    let (mut journal, mut project) = Journal::create(&path, &Project::default()).unwrap();
    project.aspect = Aspect::Square;
    journal
        .commit(&mut project, change_aspect(Aspect::Square))
        .unwrap();
    let good_hash = project.hash;
    project.aspect = Aspect::Landscape;
    journal
        .commit(&mut project, change_aspect(Aspect::Landscape))
        .unwrap();
    journal.close(&project).unwrap();

    let file = path.join("journal.ops");
    // Damage a complete record older than project.json, retaining its newline.
    let damaged = fs::read_to_string(&file)
        .unwrap()
        .replace(r#""aspect":"16:9""#, r#""aspect":"4:3""#);
    fs::write(&file, &damaged).unwrap();
    let (mut journal, mut recovered) = Journal::open(&path, &mut replay).unwrap();
    assert_eq!(journal.problem().unwrap().code, "JOURNAL_CORRUPT");
    assert_eq!(
        (recovered.rev, recovered.hash, recovered.aspect),
        (1, good_hash, Aspect::Square)
    );
    assert_eq!(
        journal
            .commit(&mut recovered, change_aspect(Aspect::Standard))
            .unwrap_err()
            .code,
        "JOURNAL_CORRUPT"
    );
    assert_eq!(
        journal.close(&recovered).unwrap_err().code,
        "JOURNAL_CORRUPT"
    );
    assert_eq!(fs::read_to_string(file).unwrap(), damaged);
}

#[test]
fn journal_checkpoint_failure_keeps_the_successful_commit() {
    let temp = Temp::new("checkpoint-failure");
    let path = temp.package();
    let (mut journal, mut project) = Journal::create(&path, &Project::default()).unwrap();
    for _ in 0..63 {
        journal
            .commit(&mut project, change_aspect(Aspect::Portrait))
            .unwrap();
    }
    let blocked = path.join("project.json.tmp");
    fs::create_dir(&blocked).unwrap();
    project.aspect = Aspect::Standard;
    let outcome = journal
        .commit(&mut project, change_aspect(Aspect::Standard))
        .unwrap();
    let Checkpoint::Failed(error) = outcome else {
        panic!("{outcome:?}")
    };
    assert_eq!(error.code, "IO");
    assert_eq!(project.rev, 64);
    let expected_hash = project.hash;
    drop(journal);
    fs::remove_dir(&blocked).unwrap();
    let (journal, recovered) = Journal::open(&path, &mut replay).unwrap();
    assert!(journal.problem().is_none());
    assert_eq!(
        (recovered.rev, recovered.hash, recovered.aspect),
        (64, expected_hash, Aspect::Standard)
    );
}
