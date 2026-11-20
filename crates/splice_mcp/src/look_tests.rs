//! T46.3: look.save writes its file only for a committed batch, and look.apply {file} fills
//! the effects from it (the journal keeps them, so replay never reads the file).
use super::*;
use crate::tests::TestDirectory;
use splice_core::history::{Source, WriteOptions};

fn sample_store(root: &Path, name: &str) -> Store {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
    let package = root.join(name);
    fs::create_dir_all(package.join("transcripts")).unwrap();
    for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
        fs::copy(source.join(file), package.join(file)).unwrap();
    }
    Store::open(&package).unwrap()
}

/// The store's agent session, started on first use.
fn session(store: &mut Store) -> splice_model::SessionId {
    store.start_session("test", 1).unwrap().id
}

fn edit(store: &mut Store, ops: &str, dry_run: bool) -> Result<Json, StoreError> {
    let ops = splice_json::parse(ops).unwrap();
    let options = WriteOptions {
        source: Source::Agent,
        session: Some(session(store)),
        if_revision: Some(store.project().rev),
        dry_run,
        ..WriteOptions::default()
    };
    crate::history::apply_batch(store, ops.as_array().unwrap(), &options)
        .map(crate::history::edit_result_json)
}

#[test]
fn look_files_save_on_commit_and_apply_in_another_project() {
    let root = TestDirectory::new("look-files");
    let mut source = sample_store(&root.0, "source.splice");
    let style = r##"{"cmd":"effect.set","target":"project","type":"window","params":{"background":"image","image":"med_logo","blur":0.5,"padding":0.12}},
        {"cmd":"effect.set","target":"clp_take","type":"camera","params":{"mirror":true,"layout":{"keys":[{"at":0,"value":"bubble"},{"at":60,"value":"full"}]}}}"##;
    edit(&mut source, &format!("[{style}]"), false).unwrap();
    let file = root.0.join("brand.json");
    let save = format!(
        r#"[{{"cmd":"look.save","target":"project","file":"{}"}}]"#,
        file.display()
    );
    // A dry run writes nothing; a commit writes the file and leaves the project as it was.
    edit(&mut source, &save, true).unwrap();
    assert!(!file.exists());
    let before = source.project().clone();
    edit(&mut source, &save, false).unwrap();
    assert_eq!(source.project().effects, before.effects);
    let look = splice_json::parse(&fs::read_to_string(&file).unwrap()).unwrap();
    let window = look
        .field("effects")
        .and_then(|e| e.field("window"))
        .unwrap();
    // The image is the logo's absolute path, so another project imports it.
    let image = window.field("image").and_then(Json::as_str).unwrap();
    assert_eq!(Path::new(image), source.path().join("assets/logo.png"));
    // A relative file lands in the package; a $ref target resolves inside the batch.
    edit(
        &mut source,
        r#"[{"cmd":"effect.set","target":"clp_take","type":"cursor","params":{"size":2},"ref":"c"},
            {"cmd":"look.save","target":"clp_take","file":"looks/take.json"}]"#,
        false,
    )
    .unwrap();
    let take = fs::read_to_string(source.path().join("looks/take.json")).unwrap();
    assert!(
        take.contains("\"camera\"") && take.contains("\"cursor\""),
        "{take}"
    );

    // Another project: look.apply {file} fills effects (the image path is offline on Linux,
    // so apply the take's camera and cursor look there; the window is checked on macOS).
    let mut target = sample_store(&root.0, "target.splice");
    let applied = edit(
        &mut target,
        &format!(
            r#"[{{"cmd":"look.apply","target":"clp_take","file":"{}"}}]"#,
            source.path().join("looks/take.json").display()
        ),
        false,
    )
    .unwrap();
    let summary = applied.field("changes").unwrap().as_array().unwrap()[0]
        .field("summary")
        .and_then(Json::as_str)
        .unwrap()
        .to_owned();
    assert!(summary.contains("camera, cursor"), "{summary}");
    // The journal keeps the filled effects: replay never reads the file.
    let journal = fs::read(target.path().join("journal.ops")).unwrap();
    assert!(
        journal.windows(9).any(|w| w == b"\"effects\""),
        "the journal keeps the filled effects"
    );
    // A missing or foreign file is refused with the rule.
    let error = edit(
        &mut target,
        r#"[{"cmd":"look.apply","target":"project","file":"nope.json"}]"#,
        false,
    )
    .unwrap_err();
    assert_eq!(error.code, "BAD_LOOK");
}
