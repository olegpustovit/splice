use super::*;
use crate::tests::TestDirectory;
use splice_core::history::WriteOptions;

/// A 16-bit mono PCM WAV of `seconds` at 8 kHz (a quiet tone), with optional INFO title and
/// artist.
fn wav(path: &Path, seconds: f64, info: Option<(&str, &str)>) {
    let rate = 8000u32;
    let samples = (seconds * f64::from(rate)) as usize;
    let mut data = Vec::with_capacity(samples * 2);
    for i in 0..samples {
        let value =
            ((i as f64 * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin() * 3000.0) as i16;
        data.extend_from_slice(&value.to_le_bytes());
    }
    let mut list = Vec::new();
    if let Some((title, artist)) = info {
        list.extend_from_slice(b"INFO");
        for (id, value) in [(b"INAM", title), (b"IART", artist)] {
            let mut bytes = value.as_bytes().to_vec();
            bytes.push(0);
            if bytes.len() % 2 == 1 {
                bytes.push(0);
            }
            list.extend_from_slice(id);
            list.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            list.extend_from_slice(&bytes);
        }
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    let riff = 4 + 24 + 8 + data.len() + if list.is_empty() { 0 } else { 8 + list.len() };
    out.extend_from_slice(&(riff as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    if !list.is_empty() {
        out.extend_from_slice(b"LIST");
        out.extend_from_slice(&(list.len() as u32).to_le_bytes());
        out.extend_from_slice(&list);
    }
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&data);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, out).unwrap();
}

/// A library with five tracks: tagged and untagged WAVs, a nested folder with its own sidecar,
/// an unlicensed file and a non-audio file.
fn library(root: &Path) -> PathBuf {
    let dir = root.join("Music");
    wav(&dir.join("calm/sunrise.wav"), 30.0, None);
    wav(
        &dir.join("calm/rain_on_glass.wav"),
        8.0,
        Some(("Rain on Glass", "Bo")),
    );
    wav(&dir.join("upbeat/drive.wav"), 12.0, None);
    wav(&dir.join("upbeat/sprint.wav"), 20.0, None);
    wav(&dir.join("mystery.wav"), 4.0, None);
    fs::write(dir.join("notes.txt"), "not audio").unwrap();
    fs::write(
        dir.join("tracks.json"),
        r#"{"artist":"Library Artist",
            "tracks":{
              "calm/sunrise.wav":{"title":"Sunrise","artist":"Ada","mood":["calm","warm"],"bpm":72,"key":"C major","loudness":-20,"license":"CC0-1.0","tags":["piano"]},
              "calm/rain_on_glass.wav":{"mood":"calm","license":"CC0-1.0","bpm":90}
            }}"#,
    )
    .unwrap();
    fs::write(
        dir.join("upbeat/tracks.json"),
        r#"{"license":"CC0-1.0","mood":"upbeat",
            "tracks":{"drive.wav":{"title":"Drive","bpm":120,"loudness":-12},
                      "sprint.wav":{"title":"Sprint","bpm":128,"mood":["upbeat","energetic"]}}}"#,
    )
    .unwrap();
    dir
}

#[test]
fn music_library_location_follows_env_then_setting_then_default() {
    let home = Some(PathBuf::from("/home/ada"));
    let path = |env: Option<&str>, setting: Option<&str>| {
        resolve(env.map(OsString::from), home.clone(), setting)
    };
    assert_eq!(
        path(Some("/srv/music"), Some(r#"{"library":"/ignored"}"#)).unwrap(),
        Path::new("/srv/music")
    );
    assert_eq!(
        path(Some("~/Tracks"), None).unwrap(),
        Path::new("/home/ada/Tracks")
    );
    assert_eq!(path(Some("Tracks"), None).unwrap_err().code, "BAD_ARGS");
    assert_eq!(
        path(None, Some(r#"{"library":"/data/cc0"}"#)).unwrap(),
        Path::new("/data/cc0")
    );
    assert_eq!(path(None, Some("{}")).unwrap_err().code, "BAD_ARGS");
    assert_eq!(
        path(Some(""), None).unwrap(),
        Path::new("/home/ada/Music/Splice")
    );
    assert_eq!(resolve(None, None, None).unwrap_err().code, "NOT_FOUND");
    let config = config_dir(home.clone(), Some("/xdg".into())).unwrap();
    if cfg!(target_os = "macos") {
        assert_eq!(
            config,
            Path::new("/home/ada/Library/Application Support/Splice")
        );
    } else {
        assert_eq!(config, Path::new("/xdg/splice"));
        assert_eq!(
            config_dir(home, None).unwrap(),
            Path::new("/home/ada/.config/splice")
        );
    }
}

#[test]
fn music_catalog_reads_sidecars_tags_and_wav_lengths() {
    let root = TestDirectory::new("music-catalog");
    let dir = library(&root.0);
    let tracks = catalog(&dir).unwrap();
    let ids: Vec<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "calm/rain_on_glass.wav",
            "calm/sunrise.wav",
            "mystery.wav",
            "upbeat/drive.wav",
            "upbeat/sprint.wav"
        ]
    );
    let sunrise = &tracks[1];
    assert_eq!(
        (sunrise.title.as_str(), sunrise.artist.as_deref()),
        ("Sunrise", Some("Ada"))
    );
    assert_eq!(sunrise.moods, ["calm", "warm"]);
    assert_eq!(sunrise.tags, ["piano"]);
    assert_eq!(
        (
            sunrise.bpm,
            sunrise.key.as_deref(),
            sunrise.license.as_str()
        ),
        (Some(72.0), Some("C major"), "CC0-1.0")
    );
    // WAV headers give lengths everywhere (macOS measures the same value natively).
    assert!((sunrise.length.unwrap() - 30.0).abs() < 0.01, "{sunrise:?}");
    // File tags fill what the sidecar entry leaves out; folder defaults come last.
    let rain = &tracks[0];
    assert_eq!(
        (
            rain.title.as_str(),
            rain.artist.as_deref(),
            rain.moods.clone()
        ),
        ("Rain on Glass", Some("Bo"), vec!["calm".to_string()])
    );
    let mystery = &tracks[2];
    assert_eq!(
        (mystery.title.as_str(), mystery.artist.as_deref()),
        ("mystery", Some("Library Artist"))
    );
    assert_eq!(mystery.license, "unknown");
    // The nearer sidecar's folder defaults win; entries still override them.
    let (drive, sprint) = (&tracks[3], &tracks[4]);
    assert_eq!(
        (drive.license.as_str(), drive.moods.clone(), drive.loudness),
        ("CC0-1.0", vec!["upbeat".to_string()], Some(-12.0))
    );
    assert_eq!(sprint.moods, ["upbeat", "energetic"]);
    if !cfg!(target_os = "macos") {
        assert_eq!(rain.loudness, None, "nothing measures loudness here");
        assert!(
            !dir.join(CACHE).exists(),
            "nothing measured, nothing cached"
        );
    }

    // A broken sidecar is named, and a missing library says how to configure one.
    fs::write(dir.join("upbeat/tracks.json"), "{").unwrap();
    let error = catalog(&dir).unwrap_err();
    assert_eq!(error.code, "BAD_MEDIA");
    assert!(
        error.message.contains("upbeat/tracks.json"),
        "{}",
        error.message
    );
    let error = catalog(&root.0.join("Nope")).unwrap_err();
    assert_eq!(error.code, "NOT_FOUND");
    assert!(error.fix.contains("SPLICE_MUSIC"), "{}", error.fix);
}

#[test]
fn music_search_ranks_and_filters() {
    let root = TestDirectory::new("music-search");
    let dir = library(&root.0);
    let search = |args: &str| search(&dir, &splice_json::parse(args).unwrap()).unwrap();
    let ids = |result: &Json| -> Vec<String> {
        result
            .field("tracks")
            .and_then(Json::as_array)
            .unwrap()
            .iter()
            .map(|t| t.field("track").and_then(Json::as_str).unwrap().to_owned())
            .collect()
    };
    let all = search("{}");
    assert_eq!(all.field("total").and_then(Json::as_u64), Some(5));
    assert_eq!(
        all.field("moods"),
        Some(&splice_json::parse(r#"["calm","energetic","upbeat","warm"]"#).unwrap())
    );
    assert_eq!(
        all.field("note").is_some(),
        !cfg!(target_os = "macos"),
        "{all:?}"
    );
    // A title hit outranks a mood or path hit.
    assert_eq!(
        ids(&search(r#"{"query":"rain"}"#)),
        ["calm/rain_on_glass.wav"]
    );
    assert_eq!(
        ids(&search(r#"{"query":"calm"}"#)),
        ["calm/rain_on_glass.wav", "calm/sunrise.wav"]
    );
    assert_eq!(
        ids(&search(r#"{"query":"sunrise ada"}"#)),
        ["calm/sunrise.wav"]
    );
    assert!(ids(&search(r#"{"query":"sunrise zebra"}"#)).is_empty());
    assert_eq!(
        ids(&search(r#"{"mood":"Upbeat"}"#)),
        ["upbeat/drive.wav", "upbeat/sprint.wav"]
    );
    // Tempo within 10 %: the closer one ranks first; unknown tempos rank last.
    assert_eq!(
        ids(&search(r#"{"bpm":126}"#)),
        ["upbeat/sprint.wav", "upbeat/drive.wav", "mystery.wav"]
    );
    assert_eq!(
        ids(&search(r#"{"minLength":10,"maxLength":25}"#)),
        ["upbeat/drive.wav", "upbeat/sprint.wav"]
    );
    let first = search(r#"{"query":"calm","limit":1}"#);
    assert_eq!(ids(&first), ["calm/rain_on_glass.wav"]);
    assert_eq!(first.field("total").and_then(Json::as_u64), Some(2));
    let track = &first.field("tracks").and_then(Json::as_array).unwrap()[0];
    assert_eq!(
        track.field("license").and_then(Json::as_str),
        Some("CC0-1.0")
    );
    assert_eq!(track.field("length").and_then(Json::as_f64), Some(8.0));
    let error = super::search(&dir, &splice_json::parse(r#"{"mod":"calm"}"#).unwrap()).unwrap_err();
    assert_eq!(error.code, "UNKNOWN_PARAM");
    assert!(error.message.contains("\"mood\""), "{}", error.message);
}

#[test]
fn music_tempo_and_id3_tags() {
    let beats: Vec<f64> = (0..16).map(|i| f64::from(i) * 0.5).collect();
    assert_eq!(tempo(&beats), Some(120.0));
    // Every other beat detected still reads 120 (folded from 60).
    let half: Vec<f64> = (0..12).map(|i| f64::from(i) * 1.0).collect();
    assert_eq!(tempo(&half), Some(120.0));
    assert_eq!(tempo(&beats[..5]), None);

    let frame = |id: &[u8], body: &[u8]| {
        let mut out = id.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&[0, 0]);
        out.extend_from_slice(body);
        out
    };
    let mut frames = frame(b"TIT2", b"\x03Night Drive");
    frames.extend(frame(b"TPE1", b"\x01\xff\xfeB\x00o\x00"));
    frames.extend(frame(b"TBPM", b"\x00104"));
    frames.extend(frame(b"TKEY", b"\x00Am"));
    let mut tag = b"ID3\x03\x00\x00".to_vec();
    let size = frames.len();
    tag.extend_from_slice(&[
        (size >> 21) as u8 & 0x7f,
        (size >> 14) as u8 & 0x7f,
        (size >> 7) as u8 & 0x7f,
        size as u8 & 0x7f,
    ]);
    tag.extend(frames);
    assert_eq!(
        id3_tags(&tag),
        Tags {
            title: Some("Night Drive".into()),
            artist: Some("Bo".into()),
            bpm: Some(104.0),
            key: Some("Am".into()),
            ..Tags::default()
        }
    );
}

fn sample_store(root: &Path) -> Store {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/projects/sample.splice");
    let package = root.join("sample.splice");
    fs::create_dir_all(package.join("transcripts")).unwrap();
    for file in ["project.json", "media.json", "transcripts/med_screen.json"] {
        fs::copy(source.join(file), package.join(file)).unwrap();
    }
    Store::open(&package).unwrap()
}

/// music.add end to end: the track resolves from the library, lands fitted, ducked and leveled,
/// unknown licenses are refused, the export credits list it, and the journal replays after
/// the library is gone.
#[test]
fn music_add_resolves_credits_and_replays_without_the_library() {
    let root = TestDirectory::new("music-add");
    let dir = library(&root.0);
    let mut store = sample_store(&root.0);
    let session = store.start_session("test", 1).unwrap().id;
    let add = |store: &mut Store, op: &Json| {
        let options = WriteOptions {
            source: splice_core::history::Source::Agent,
            session: Some(session),
            if_revision: Some(store.project().rev),
            ..WriteOptions::default()
        };
        crate::history::apply_batch(store, std::slice::from_ref(op), &options)
    };
    let op = |json: &str| {
        let mut op = splice_json::parse(json).unwrap();
        let extra = fill(&dir, &op).unwrap();
        let Json::Object(fields) = &mut op else {
            unreachable!()
        };
        // As edit.apply does: the catalog's fields replace the op's.
        fields.retain(|(key, _)| !extra.iter().any(|(added, _)| added == key));
        fields.extend(extra);
        op
    };
    // A title stands in for its ID when it is unambiguous; the catalog's credit is filled.
    let sunrise = op(r#"{"cmd":"music.add","track":"Sunrise"}"#);
    assert_eq!(
        sunrise.field("path").and_then(Json::as_str),
        Some(dir.join("calm/sunrise.wav").to_str().unwrap())
    );
    assert_eq!(
        sunrise.field("license").and_then(Json::as_str),
        Some("CC0-1.0")
    );
    assert_eq!(
        sunrise.field("loudness").and_then(Json::as_f64),
        Some(-20.0)
    );
    let result = add(&mut store, &sunrise).unwrap();
    assert_eq!(
        result.report.changes[0].created.len(),
        1,
        "a long track is cut once"
    );

    // An unknown license is refused until the agent says it holds the rights.
    let mystery = op(r#"{"cmd":"music.add","track":"mystery.wav","volume":0.5}"#);
    let error = add(&mut store, &mystery).unwrap_err();
    assert_eq!(error.code, "UNLICENSED");
    let allowed = op(
        r#"{"cmd":"music.add","track":"mystery.wav","volume":0.5,"allowUnlicensed":true,"fit":"trim"}"#,
    );
    add(&mut store, &allowed).unwrap();
    let error = fill(
        &dir,
        &splice_json::parse(r#"{"track":"sunrise.wav"}"#).unwrap(),
    )
    .unwrap_err();
    assert_eq!(error.code, "NOT_FOUND");
    assert!(error.message.contains("\"calm/sunrise.wav\""), "{error:?}");

    assert_eq!(
        credits_of(&store, [0, 660]),
        "Music\n\"Sunrise\" by Ada · License: CC0-1.0 · calm/sunrise.wav\n\
         \"mystery\" by Library Artist · License: unknown · mystery.wav\n"
    );
    // The trimmed track ends at 4 s: an export after it credits only what sounds.
    assert_eq!(
        credits_of(&store, [200, 660]),
        "Music\n\"Sunrise\" by Ada · License: CC0-1.0 · calm/sunrise.wav\n"
    );

    let live = store.project().clone();
    let path = store.path().to_path_buf();
    drop(store);
    fs::remove_dir_all(&dir).unwrap();
    let reopened = Store::open(&path).unwrap();
    assert_eq!(reopened.project().media, live.media);
    let hash = splice_core::history::state_hash;
    assert_eq!(hash(reopened.project()).unwrap(), hash(&live).unwrap());
    reopened.verify().unwrap();
    assert!(credits_of(&reopened, [0, 660]).contains("Sunrise"));
}

fn credits_of(store: &Store, range: [Frame; 2]) -> String {
    credits(store, range).unwrap()
}
