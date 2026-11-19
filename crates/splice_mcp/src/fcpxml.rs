//! FCPXML 1.10 export (PLAN.md T21.3, D8): the spine as asset clips and gaps, transitions, media
//! overlays and sounds as connected clips, titles as Basic Titles, markers. Only the cut is
//! promised to survive; Splice looks (frames, cursor, captions styling) stay in Splice.
use splice_eval::Evaluator;
use splice_model::*;
use std::fmt::Write;

fn time(frames: f64, fps: f64) -> String {
    // Rational seconds on the media's own clock; integer frames stay exact.
    let rate = fps.round().max(1.0) as i64;
    let n = frames.round() as i64;
    if n % rate == 0 {
        format!("{}s", n / rate)
    } else {
        format!("{n}/{rate}s")
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn url(path: &std::path::Path) -> String {
    let text = path.display().to_string();
    let mut out = String::from("file://");
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
    out
}

/// The project as FCPXML text. `package` resolves relative media paths.
pub(super) fn write(project: &Project, package: &std::path::Path, name: &str) -> String {
    let fps = f64::from(project.fps);
    let evaluator = Evaluator::prepare(project).ok();
    let mut out = String::new();
    let [w, h] = project.size;
    let _ = writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    let _ = writeln!(out, "<!DOCTYPE fcpxml>");
    let _ = writeln!(out, r#"<fcpxml version="1.10">"#);
    let _ = writeln!(out, "  <resources>");
    let _ = writeln!(
        out,
        r#"    <format id="r0" frameDuration="{}" width="{w}" height="{h}"/>"#,
        time(1.0, fps)
    );
    for (index, media) in project.media.iter().enumerate() {
        if media.kind == MediaKind::Compound {
            continue;
        }
        let path = package.join(&media.path);
        let file = match media.take.as_ref().and_then(|t| t.screen.as_deref()) {
            Some(screen) if path.is_dir() => path.join(screen),
            Some(screen) => path.with_file_name(screen),
            None => path,
        };
        let video = !matches!(media.kind, MediaKind::Audio);
        let audio = !media.probe.audio.is_empty() || media.kind == MediaKind::Audio;
        let _ = writeln!(
            out,
            r#"    <asset id="a{index}" name="{}" start="0s" duration="{}" hasVideo="{}" hasAudio="{}" format="r0"><media-rep kind="original-media" src="{}"/></asset>"#,
            escape(&media.id.to_string()),
            time(
                f64::from(media.probe.frames.min(10_000_000)),
                media.probe.fps
            ),
            u8::from(video),
            u8::from(audio),
            url(&file)
        );
    }
    let _ = writeln!(
        out,
        r#"    <effect id="e0" name="Basic Title" uid=".../Titles.localized/Bumper:Opener.localized/Basic Title.localized/Basic Title.moti"/>"#
    );
    let _ = writeln!(
        out,
        r#"    <effect id="e1" name="Cross Dissolve" uid="FxPlug:4731E73A-8DAC-4113-9A30-AE85B1761265"/>"#
    );
    let _ = writeln!(out, "  </resources>");
    let end = project
        .clips
        .order()
        .last()
        .and_then(|&s| project.clips.timing(s))
        .map_or(0, |t| t.start + t.length);
    let _ = writeln!(
        out,
        r#"  <library><event name="Splice"><project name="{}">"#,
        escape(name)
    );
    let _ = writeln!(
        out,
        r#"    <sequence format="r0" duration="{}" tcStart="0s" tcFormat="NDF" audioLayout="stereo" audioRate="48k"><spine>"#,
        time(f64::from(end), fps)
    );
    let asset = |id: MediaId| project.media.iter().position(|m| m.id == id);
    // Items connect to the spine clip they start over; offsets are on that clip's clock.
    let mut items: Vec<(Frame, Frame, Slot)> = project
        .items
        .order()
        .iter()
        .filter_map(|&slot| {
            let span = evaluator
                .as_ref()?
                .extent(TimelineId::Item(project.items.id(slot)?))?;
            Some((span[0], span[1], slot))
        })
        .collect();
    items.sort_by_key(|i| i.0);
    let mut at = 0;
    for &slot in project.clips.order() {
        let timing = project.clips.timing(slot).expect("live clip");
        let data = project.clips.data(slot).expect("live clip");
        if timing.start > at {
            let _ = writeln!(
                out,
                r#"      <gap offset="{}" duration="{}"/>"#,
                time(f64::from(at), fps),
                time(f64::from(timing.start - at), fps)
            );
        }
        let media = data
            .media
            .and_then(|m| project.media.iter().find(|x| x.id == m));
        let media_fps = media.map_or(fps, |m| m.probe.fps);
        let source = f64::from(timing.source) / media_fps * fps;
        if data
            .appearance
            .effects
            .iter()
            .any(|e| e.kind == "transition")
        {
            let length = evaluator
                .as_ref()
                .and_then(|e| e.transition(project.clips.id(slot).expect("live clip")))
                .map_or(15, |t| t.length);
            let _ = writeln!(
                out,
                r#"      <transition name="Cross Dissolve" offset="{}" duration="{}"><filter-video ref="e1" name="Cross Dissolve"/></transition>"#,
                time(f64::from(timing.start - length / 2), fps),
                time(f64::from(length), fps)
            );
        }
        let (open, close) = match media.and_then(|m| asset(m.id)) {
            Some(index) if media.is_some_and(|m| m.kind != MediaKind::Compound) => (
                format!(
                    r#"      <asset-clip ref="a{index}" name="{}" offset="{}" start="{}" duration="{}">"#,
                    escape(data.appearance.name.as_deref().unwrap_or("")),
                    time(f64::from(timing.start), fps),
                    time(source, fps),
                    time(f64::from(timing.length), fps)
                ),
                "      </asset-clip>",
            ),
            _ => (
                format!(
                    r#"      <gap offset="{}" duration="{}">"#,
                    time(f64::from(timing.start), fps),
                    time(f64::from(timing.length), fps)
                ),
                "      </gap>",
            ),
        };
        let _ = writeln!(out, "{open}");
        let clip_end = timing.start + timing.length;
        let local = |frame: Frame| time(source + f64::from(frame - timing.start), fps);
        let mut lane = 0;
        for &(start, stop, item) in items
            .iter()
            .filter(|i| i.0 >= timing.start && i.0 < clip_end)
        {
            lane += 1;
            let data = project.items.data(item).expect("live item");
            let duration = time(f64::from(stop - start), fps);
            match &data.kind {
                ItemKind::Media(overlay) => {
                    if let Some(index) = asset(overlay.media) {
                        let rate = project.media.get(index).map_or(fps, |m| m.probe.fps);
                        let _ = writeln!(
                            out,
                            r#"        <asset-clip ref="a{index}" lane="{lane}" offset="{}" start="{}" duration="{duration}"/>"#,
                            local(start),
                            time(f64::from(overlay.source) / rate * fps, fps)
                        );
                    }
                }
                ItemKind::Sound(sound) => {
                    if let Some(index) = asset(sound.media) {
                        let rate = project.media.get(index).map_or(fps, |m| m.probe.fps);
                        let _ = writeln!(
                            out,
                            r#"        <asset-clip ref="a{index}" lane="-{lane}" offset="{}" start="{}" duration="{duration}" audioRole="music"/>"#,
                            local(start),
                            time(f64::from(sound.source) / rate * fps, fps)
                        );
                    }
                }
                ItemKind::Title { text } => {
                    let _ = writeln!(
                        out,
                        r#"        <title ref="e0" lane="{lane}" offset="{}" duration="{duration}" name="{}"><text><text-style ref="ts{lane}x{item}">{}</text-style></text><text-style-def id="ts{lane}x{item}"><text-style font="Helvetica" fontSize="60" fontColor="1 1 1 1"/></text-style-def></title>"#,
                        local(start),
                        escape(text),
                        escape(text)
                    );
                }
                _ => {}
            }
        }
        for marker in project
            .markers
            .iter()
            .filter(|m| m.at >= timing.start && m.at < clip_end)
        {
            let _ = writeln!(
                out,
                r#"        <marker start="{}" duration="{}" value="{}"/>"#,
                local(marker.at),
                time(1.0, fps),
                escape(&marker.name)
            );
        }
        let _ = writeln!(out, "{close}");
        at = clip_end;
    }
    let _ = writeln!(out, "    </spine></sequence>");
    let _ = writeln!(out, "  </project></event></library>");
    let _ = writeln!(out, "</fcpxml>");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_exports_the_cut_with_matching_clip_ranges() {
        let project = crate::tests::sample_project();
        let xml = write(
            &project,
            std::path::Path::new("/tmp/sample.splice"),
            "Sample",
        );
        assert!(xml.starts_with("<?xml") && xml.trim_end().ends_with("</fcpxml>"));
        // Every spine clip appears with its exact range.
        for &slot in project.clips.order() {
            let t = project.clips.timing(slot).unwrap();
            let needle = format!(
                r#"offset="{}" start="{}" duration="{}""#,
                time(f64::from(t.start), 30.0),
                time(f64::from(t.source), 30.0),
                time(f64::from(t.length), 30.0)
            );
            assert!(xml.contains(&needle), "{needle}");
        }
        assert_eq!(xml.matches("<marker ").count(), project.markers.len());
        // Balanced elements.
        for tag in ["asset-clip", "gap", "spine", "sequence", "resources"] {
            let open = xml.matches(&format!("<{tag}")).count();
            let closed = xml.matches(&format!("</{tag}>")).count();
            let self_closed = xml
                .lines()
                .filter(|l| {
                    l.trim_start().starts_with(&format!("<{tag}")) && l.trim_end().ends_with("/>")
                })
                .count();
            assert_eq!(open, closed + self_closed, "{tag}");
        }
    }
}
