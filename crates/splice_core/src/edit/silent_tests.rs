//! Silent takes (PLAN.md T41): activity basis, steps, typing speed-ups and free-text cues.
use super::tests::sample;
use super::*;

/// The sample without its transcript: a silent screen take (clp_take, frames 120–540).
fn silent() -> Project {
    let mut project = sample();
    project.transcripts.clear();
    project
}

fn run(project: &mut Project, ops: &str) -> Result<ApplyReport, ApplyError> {
    let ops = splice_json::parse(ops).unwrap();
    apply(project, ops.as_array().unwrap(), Source::Agent, false)
}

fn clip(project: &Project, id: &str) -> ClipTiming {
    let id: ClipId = id.parse().unwrap();
    project
        .clips
        .timing(project.clips.find(id).unwrap())
        .unwrap()
}

/// T41.3: `auto` falls back to activity on a silent take; `transcript` refuses with a fix that
/// names basis:"activity"; `activity` cuts idle stretches even where words exist.
#[test]
fn smart_cuts_take_an_activity_basis() {
    let mut project = silent();
    let error = run(&mut project, r#"[{"cmd":"cut.pauses"}]"#).unwrap_err();
    assert_eq!(error.error.code, "NO_TRANSCRIPT");
    assert!(error.error.fix.contains("basis:\"activity\""), "{error:?}");
    // Busy at source 0–30 and 150–180 (timeline 120–150 and 270–300) and 400–420.
    let activity = r#""activity":[{"media":"med_screen","from":0,"to":30},{"media":"med_screen","from":150,"to":180},{"media":"med_screen","from":400,"to":420}]"#;
    let error = run(
        &mut project.clone(),
        &format!(r#"[{{"cmd":"cut.pauses","basis":"transcript",{activity}}}]"#),
    )
    .unwrap_err();
    assert_eq!(error.error.code, "NO_TRANSCRIPT");
    let report = run(
        &mut project,
        &format!(r#"[{{"cmd":"cut.pauses",{activity}}}]"#),
    )
    .unwrap();
    // Idle 150–270 (120 frames) and 300–520 (220): 2 s or more, cut keeping 8 frames a side.
    assert_eq!(
        report.changes[0].summary,
        "Cut 2 idle stretch(es), 308 frames"
    );
    assert_eq!(clip(&project, "clp_end").start, 540 - 308);
    // With words on the timeline, basis:"activity" still measures activity.
    let mut spoken = sample();
    let report = run(
        &mut spoken,
        &format!(r#"[{{"cmd":"intent.pace","basis":"activity",{activity}}}]"#),
    )
    .unwrap();
    assert!(report.changes[0].summary.ends_with("(activity)"));
    // fitDuration cuts idle first, then the shortest bursts.
    let mut fit = silent();
    run(
        &mut fit,
        &format!(r#"[{{"cmd":"intent.fitDuration","seconds":12,{activity}}}]"#),
    )
    .unwrap();
    let end = clip(&fit, "clp_end");
    assert_eq!(end.start + end.length, 360);
    // No activity anywhere: NO_ACTIVITY, with the fix.
    let error = run(
        &mut silent(),
        r#"[{"cmd":"cut.pauses","basis":"activity"}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "NO_ACTIVITY");
}

/// T41.2: a `retake:` marker drops the attempt since the step before it; each step keeps
/// [step − lead, last activity + settle]; the summary (and so a dry run) lists the steps.
#[test]
fn steps_keep_each_step_and_drop_retake_attempts() {
    let mut project = silent();
    project.markers.clear();
    for (serial, at, name) in [
        (7, 145, "Open settings"),
        (8, 300, "retake: Open settings"),
        (9, 410, "Save"),
    ] {
        project.markers.push(Marker {
            id: MarkerId::from_serial(serial),
            at,
            name: name.into(),
            color: None,
        });
    }
    let ops = r#"[{"cmd":"intent.steps","item":"clp_take","activity":[{"from":30,"to":60},{"from":100,"to":130},{"from":190,"to":220},{"from":300,"to":330}],"lead":12,"settle":30,"maxGap":45}]"#;
    let parsed = splice_json::parse(ops).unwrap();
    let preview = apply(
        &mut project.clone(),
        parsed.as_array().unwrap(),
        Source::Agent,
        true,
    )
    .unwrap();
    assert_eq!(
        preview.changes[0].summary,
        "Kept 2 step(s) of clp_take, cut 268 frames, dropped 1 retake attempt(s): \"Open settings\" at 300 keeps 300–370; \"Save\" at 410 keeps 398–480"
    );
    let report = run(&mut project, ops).unwrap();
    assert_eq!(report.changes[0].summary, preview.changes[0].summary);
    // Kept: 300–370 and 398–480 of the take, back to back after the opener.
    assert_eq!(clip(&project, "clp_end").start, 120 + 70 + 82);
    // Without markers, bursts more than maxGap apart are the steps.
    let mut bursts = silent();
    bursts.markers.clear();
    let report = run(
        &mut bursts,
        r#"[{"cmd":"intent.steps","item":"clp_take","activity":[{"from":30,"to":60},{"from":100,"to":130},{"from":280,"to":310}],"lead":12,"settle":30,"maxGap":45}]"#,
    )
    .unwrap();
    assert_eq!(
        report.changes[0].summary,
        "Kept 2 step(s) of clp_take, cut 206 frames: \"activity\" at 150 keeps 138–280; \"activity\" at 400 keeps 388–460"
    );
    // A take without activity inside the clip is refused, never emptied.
    let error = run(
        &mut silent(),
        r#"[{"cmd":"intent.steps","item":"clp_take","activity":[{"from":500,"to":530}]}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "NO_ACTIVITY");
}

/// T41.4: typing runs become eased speed keys and the clip shortens to show exactly the same
/// source (never past its media); short runs and a second ramp are refused.
#[test]
fn speed_typing_writes_eased_rate_keys_within_the_media() {
    let mut project = silent();
    run(
        &mut project,
        r#"[{"cmd":"intent.speedTyping","item":"clp_take","typing":[{"from":60,"to":180},{"from":200,"to":215},{"from":300,"to":390}],"factor":4}]"#,
    )
    .unwrap();
    let slot = project.clips.find("clp_take".parse().unwrap()).unwrap();
    let data = project.clips.data(slot).unwrap();
    let speed = data
        .appearance
        .effects
        .iter()
        .find(|effect| effect.kind == "speed")
        .unwrap();
    let EffectValue::Keys(keys) = &speed.params[0].value else {
        panic!("rate keys")
    };
    let shape: Vec<(Frame, f64, Ease)> = keys.iter().map(|k| (k.at, k.value, k.ease)).collect();
    // 120 frames at 4×: two 6-frame ramps cover 30, a 23-frame middle 92 (rounded); the
    // 15-frame run is under minRun (1 s) and keeps its speed; 90 frames take 6 + 15 + 6.
    assert_eq!(
        shape,
        [
            (60, 1.0, Ease::InOut),
            (66, 4.0, Ease::Linear),
            (89, 4.0, Ease::InOut),
            (95, 1.0, Ease::Linear),
            (213, 1.0, Ease::InOut),
            (219, 4.0, Ease::Linear),
            (234, 4.0, Ease::InOut),
            (240, 1.0, Ease::Linear),
        ]
    );
    let timing = clip(&project, "clp_take");
    // The new clip shows the old 420 source frames, to within one frame, never more.
    let shown = keys::integrate(keys, f64::from(timing.length));
    assert!(shown <= 420.0 && shown > 419.0, "{shown}");
    assert_eq!(clip(&project, "clp_end").start, 120 + timing.length);
    let error = run(
        &mut project,
        r#"[{"cmd":"intent.speedTyping","item":"clp_take","typing":[{"from":60,"to":180}]}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "RETIMED");
    let error = run(
        &mut silent(),
        r#"[{"cmd":"intent.speedTyping","item":"clp_take","typing":[{"from":60,"to":80}]}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.code, "NO_TYPING");
    let error = run(
        &mut silent(),
        r#"[{"cmd":"intent.speedTyping","item":"clp_take","typing":[{"from":60,"to":180}],"factor":20}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.rule.unwrap().param, "factor");
}

/// T41.5: free-text cues need no transcript: caption.add and SRT import work on a silent take,
/// keep their timing through settle, retype with caption.set and round-trip the package.
#[test]
fn free_text_cues_work_on_a_silent_take() {
    let mut project = silent();
    project.items.order().to_vec().into_iter().for_each(|slot| {
        if matches!(
            project.items.data(slot).unwrap().kind,
            ItemKind::Caption { .. }
        ) {
            project.items.remove(slot);
        }
    });
    let report = run(
        &mut project,
        r#"[{"cmd":"caption.add","text":"Open the settings","from":150,"to":210,"ref":"cue"},
            {"cmd":"captions.import","lane":"lan_cap","text":"1\n00:00:08,000 --> 00:00:09,500\nPick a theme\n\n2\n00:00:30,000 --> 00:00:31,000\nPast the end\n"},
            {"cmd":"caption.set","item":"$cue","text":"Open Settings"}]"#,
    )
    .unwrap();
    assert!(
        report.changes[1].summary.contains("1 as free text"),
        "{:?}",
        report.changes[1]
    );
    let Target::Timeline(TimelineId::Item(added)) = report.changes[0].created[0] else {
        panic!("a caption item")
    };
    let cues: Vec<(String, Frame, Frame)> = project
        .items
        .order()
        .iter()
        .filter_map(|&slot| {
            let ItemKind::Caption {
                text: Some(text), ..
            } = &project.items.data(slot).unwrap().kind
            else {
                return None;
            };
            let span = time::item_span(&project, slot).unwrap();
            Some((text.clone(), span.start, span.end))
        })
        .collect();
    assert_eq!(
        cues,
        [
            ("Open Settings".to_owned(), 150, 210),
            ("Pick a theme".to_owned(), 240, 285),
        ]
    );
    // Word operations name the rule; trims work like any timed item.
    let error = run(
        &mut project.clone(),
        &format!(r#"[{{"cmd":"caption.split","item":"{added}","word":"w2"}}]"#),
    )
    .unwrap_err();
    assert_eq!(error.error.code, "WRONG_KIND");
    run(
        &mut project,
        &format!(r#"[{{"cmd":"item.trim","item":"{added}","edge":"out","to":200}}]"#),
    )
    .unwrap();
    let slot = project.items.find(added).unwrap();
    assert_eq!(project.items.timing(slot).unwrap().length, 50);
    // A cue past the timeline is refused with the rule.
    let error = run(
        &mut project.clone(),
        r#"[{"cmd":"caption.add","text":"Late","from":600,"to":700}]"#,
    )
    .unwrap_err();
    assert_eq!(error.error.rule.unwrap().param, "to");
    // The package keeps the text and reads it back.
    let encoded = crate::package::project::encode(&project);
    let mut bytes = String::new();
    splice_json::write(&encoded, &mut bytes).unwrap();
    assert!(bytes.contains(r#""words":[],"edits":[],"text":"Open Settings""#));
    let decoded = crate::package::project::decode(&splice_json::parse(&bytes).unwrap()).unwrap();
    assert_eq!(crate::package::project::encode(&decoded), encoded);
}
