//! Lottie and SVG imports probe through ThorVG on every platform (T45.2). They become image
//! media with codec `lottie` or `svg`: like a still they span any length, and a Lottie loops at
//! its own frame rate (splice_av renders its frames into a picture movie).
use splice_core::commands::CommandError;
use splice_json::Value as Json;
use splice_model::{Media, MediaId, MediaKind, Probe};
use std::{fs, os::unix::fs::MetadataExt, path::Path};

/// The vector codec a path's extension names, if any.
pub(crate) fn codec(path: &str) -> Option<&'static str> {
    let (_, extension) = path.rsplit_once('.')?;
    match extension.to_ascii_lowercase().as_str() {
        "json" | "lot" => Some("lottie"),
        "svg" => Some("svg"),
        _ => None,
    }
}

/// Opens `request` (absolute, or relative to the package) as Lottie or SVG.
pub(crate) fn probe(
    package: &Path,
    request: &str,
    codec: &'static str,
    project_fps: u32,
) -> Result<Media, CommandError> {
    let path = package.join(request);
    let offline = |failure: &dyn std::fmt::Display| {
        CommandError::new(
            "MEDIA_OFFLINE",
            format!("Could not access {}: {failure}", path.display()),
            "Check that the media is available and readable, then try the path again.".into(),
        )
    };
    let absolute = path.canonicalize().map_err(|failure| offline(&failure))?;
    let metadata = fs::metadata(&absolute).map_err(|failure| offline(&failure))?;
    if !metadata.is_file() {
        return Err(bad_media(request, "a regular file".into()));
    }
    let info = thorvg_sys::open(&absolute)
        .map_err(|failure| bad_media(request, failure.message))?
        .info();
    let root = package
        .canonicalize()
        .map_err(|failure| offline(&failure))?;
    let stored = absolute.strip_prefix(&root).unwrap_or(&absolute);
    let stored = stored
        .to_str()
        .ok_or_else(|| bad_media(request, "a UTF-8 path".into()))?;
    #[cfg(target_os = "macos")]
    let bookmark =
        Some(apple_sys::media::bookmark(&absolute).map_err(|failure| offline(&failure))?);
    #[cfg(not(target_os = "macos"))]
    let bookmark = None;
    Ok(Media {
        id: MediaId::from_serial(0),
        kind: MediaKind::Image,
        path: stored.into(),
        bookmark,
        size: metadata.len(),
        mtime: metadata.mtime(),
        probe: Probe {
            fps: f64::from(project_fps),
            frames: 1_000_000_000,
            width: info.width,
            height: info.height,
            audio: Vec::new(),
            codec: Some(codec.into()),
            loudness: None,
        },
        take: None,
    })
}

fn bad_media(request: &str, detail: String) -> CommandError {
    let mut error = CommandError::rule(
        "BAD_MEDIA",
        "path",
        Some(Json::String(request.into())),
        "a Lottie JSON (.json, .lot) or SVG file with a size",
        "Export the animation as Lottie JSON (Bodymovin) or the drawing as SVG and pass that path.",
    );
    error.message = format!("{request} is not usable vector media: {detail}");
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use splice_core::history::{Source, Store, WriteOptions};

    fn fixture(name: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../thorvg_sys/fixtures")
            .join(name)
    }

    #[test]
    fn lottie_and_svg_insert_as_looping_pictures_on_every_platform() {
        let root = std::env::temp_dir().join(format!("splice-vector-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let package = root.join("vector.splice");
        let mut store = Store::create(&package, &splice_model::Project::default()).unwrap();
        fs::create_dir_all(package.join("art")).unwrap();
        fs::copy(fixture("square.json"), package.join("art/square.json")).unwrap();
        let svg = fixture("half.svg");
        let session = store.start_session("test", 1).unwrap().id;
        let ops = splice_json::parse(&format!(
            r#"[{{"cmd":"clip.insert","path":"art/square.json"}},
                {{"cmd":"clip.insert","path":"{}","insert":false,"ref":"art"}},
                {{"cmd":"overlay.add","kind":"media","media":"$art","at":0,"length":60}}]"#,
            svg.display()
        ))
        .unwrap();
        let options = WriteOptions {
            source: Source::Agent,
            session: Some(session),
            if_revision: Some(store.project().rev),
            ..Default::default()
        };
        crate::history::apply_batch(&mut store, ops.as_array().unwrap(), &options).unwrap();
        let media = &store.project().media;
        assert_eq!(media.len(), 2);
        let (lottie, drawing) = (&media[0], &media[1]);
        assert_eq!(lottie.kind, MediaKind::Image);
        assert_eq!(lottie.path, "art/square.json", "stored inside the package");
        assert_eq!(lottie.probe.codec.as_deref(), Some("lottie"));
        assert_eq!([lottie.probe.width, lottie.probe.height], [64, 64]);
        assert_eq!(drawing.probe.codec.as_deref(), Some("svg"));
        assert_eq!([drawing.probe.width, drawing.probe.height], [40, 20]);
        assert!(Path::new(&drawing.path).is_absolute());
        // A still's length on the spine: 5 s.
        let p = store.project();
        assert_eq!(p.clips.timing(p.clips.order()[0]).unwrap().length, 150);
        assert_eq!(p.items.order().len(), 1);
        store.close().unwrap();
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn unreadable_vector_files_name_the_rule() {
        let root = std::env::temp_dir().join(format!("splice-vector-bad-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("data.json"), b"{\"rows\": []}").unwrap();
        let error = probe(&root, "data.json", "lottie", 30).unwrap_err();
        assert_eq!(error.code, "BAD_MEDIA");
        let rule = error.rule.unwrap();
        assert_eq!(rule.param, "path");
        assert_eq!(rule.got, Some(Json::String("data.json".into())));
        assert_eq!(
            probe(&root, "missing.svg", "svg", 30).unwrap_err().code,
            "MEDIA_OFFLINE"
        );
        assert_eq!(codec("a/B.SVG"), Some("svg"));
        assert_eq!(codec("anim.lot"), Some("lottie"));
        assert_eq!(codec("clip.mov"), None);
        fs::remove_dir_all(&root).unwrap();
    }
}
