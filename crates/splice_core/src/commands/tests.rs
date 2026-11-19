use super::*;

#[test]
fn registry_matches_prototype_ids_order_and_status() {
    let prototype = include_str!("../../../../docs/reference/core.js");
    let built: Vec<_> = prototype
        .lines()
        .filter_map(|line| line.strip_prefix("cmd(\"")?.split('"').next())
        .collect();
    let (_, planned) = prototype.split_once("const PLANNED = [").unwrap();
    let planned: Vec<_> = planned
        .lines()
        .take_while(|line| !line.starts_with("];"))
        .filter_map(|line| line.trim().strip_prefix("[\"")?.split('"').next())
        .collect();
    assert_eq!((built.len(), planned.len()), (41, 20));
    // 0.4 replaces the prototype's scale animations with camera segments (D12): motion.add and
    // motion.set take their places as zoom.add and zoom.set, and the prototype's planned click
    // "zoom.set" (settings) became intent.autoZoom's job.
    let renamed = |id: &str| {
        RETIRED
            .iter()
            .find(|(old, ..)| *old == id)
            .map_or(id.to_owned(), |(_, new, _)| (*new).to_owned())
    };
    let built: Vec<String> = built.into_iter().map(renamed).collect();
    let planned: Vec<String> = planned
        .into_iter()
        .filter(|id| *id != "zoom.set")
        .map(renamed)
        .collect();
    let extensions: Vec<String> = EXTENSIONS.iter().map(|id| (*id).to_owned()).collect();
    assert_eq!(
        COMMANDS
            .iter()
            .map(|row| row.id.to_owned())
            .collect::<Vec<_>>(),
        [built, planned, extensions].concat()
    );
    assert_eq!(
        COMMANDS
            .iter()
            .filter(|row| row.tier == Tier::First)
            .count(),
        20
    );
    // 0.3 built the prototype's planned rows (less its click-zoom settings, now
    // intent.autoZoom's job) and 16 Splice rows; nothing stays planned.
    assert_eq!(
        COMMANDS
            .iter()
            .filter(|row| row.tier == Tier::Third)
            .count(),
        35
    );
    assert!(COMMANDS.iter().all(|row| row.tier != Tier::Planned));
}

#[test]
fn every_command_has_a_documented_decodable_example_and_distinct_variant() {
    let mut variants = Vec::new();
    let mut json = String::new();
    for row in COMMANDS {
        let described = describe(Some(row.id), None).unwrap();
        let example = described.field("example").unwrap();
        splice_json::write(example, &mut json).unwrap();
        assert_eq!(row.example, json, "{}", row.id);
        assert_eq!(example.field("cmd").and_then(Json::as_str), Some(row.id));
        assert!(!row.title.is_empty());
        assert!(!row.group.is_empty());
        assert!(!row.ui.is_empty());
        assert!(described.field("params").unwrap().as_object().is_some());
        let op = row
            .decode(example, &mut DecodeScratch::default())
            .unwrap_or_else(|error| panic!("{}: {error}", row.id));
        let variant = std::mem::discriminant(&op);
        assert!(!variants.contains(&variant), "{}", row.id);
        variants.push(variant);
        if row.tier == Tier::Planned {
            assert_eq!(decode(example).unwrap_err().code, "PLANNED");
        } else {
            assert_eq!(decode(example).unwrap().op, op);
        }
    }

    let effect = describe(Some("effect.add"), None).unwrap();
    assert_eq!(effect.field("cut").unwrap().as_str(), Some("0.2"));
    let effects = effect.field("effectTypes").unwrap();
    assert_eq!(effects.as_object().unwrap().len(), 23);
    assert!(
        effects
            .field("window")
            .unwrap()
            .field("params")
            .unwrap()
            .field("padding")
            .is_some()
    );
    assert_eq!(
        describe(Some("caption.set"), None)
            .unwrap()
            .field("presets")
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        22
    );

    let matches = describe(None, Some("FrAMing")).unwrap();
    let matches = matches.field("commands").unwrap().as_array().unwrap();
    assert_eq!(
        matches
            .iter()
            .map(|row| row.field("id").unwrap().as_str().unwrap())
            .collect::<Vec<_>>(),
        ["zoom.add", "zoom.set", "clip.crop", "crop.keys"]
    );
}

#[test]
fn decoders_preserve_refs_and_nullable_edit_intent() {
    let entry = splice_json::parse(
        r#"{"cmd":"item.set","items":"$split.right","name":null,"fadeOut":8,"ref":"$edit"}"#,
    )
    .unwrap();
    let decoded = decode(&entry).unwrap();
    assert_eq!(decoded.bind.as_deref(), Some("$edit"));
    let Op::ItemSet {
        items,
        name,
        color,
        fade_out,
        ..
    } = decoded.op
    else {
        panic!()
    };
    assert_eq!(
        items,
        [Target::Ref(Reference {
            name: "$split".into(),
            part: RefPart::FirstCreated
        })]
    );
    assert_eq!(name, Some(None));
    assert_eq!(color, None);
    assert_eq!(fade_out, Some(8));

    let entry = splice_json::parse(r#"{"cmd":"overlay.add","kind":"media","media":"med_broll","anchor":{"item":"$split.right","edge":"end","offset":2}}"#).unwrap();
    let decoded = decode(&entry).unwrap();
    let Op::OverlayAdd {
        anchor: Some(AnchorArg::At(FrameArg::Item { item, edge, offset })),
        ..
    } = decoded.op
    else {
        panic!()
    };
    assert!(matches!(item, Target::Ref(_)));
    assert_eq!(edge, splice_model::Edge::End);
    assert_eq!(offset, 2);
}

#[test]
fn decoders_reject_unknown_and_invalid_arguments() {
    for (input, code, field) in [
        (r#"{"cmd":"item.splitt"}"#, "UNKNOWN_COMMAND", "item.splitt"),
        (
            r#"{"cmd":"item.split","item":"clp_take","at":12.5}"#,
            "BAD_ARGS",
            "at",
        ),
        (
            r#"{"cmd":"item.split","item":"clp_take","at":2147483648}"#,
            "BAD_ARGS",
            "at",
        ),
        (
            r#"{"cmd":"item.split","item":"$cut.bad","at":12}"#,
            "BAD_ARGS",
            "item",
        ),
        (
            r#"{"cmd":"item.split","item":"clp_take","at":12,"position":0}"#,
            "UNKNOWN_PARAM",
            "position",
        ),
        (
            r#"{"cmd":"item.trim","item":"clp_take","edge":"start","delta":1}"#,
            "BAD_ARGS",
            "edge",
        ),
        (
            r#"{"cmd":"clip.insert","media":"med_screen","path":"/tmp/clip.mov"}"#,
            "BAD_ARGS",
            "path",
        ),
        (
            r#"{"cmd":"overlay.add","kind":"media","at":10}"#,
            "BAD_ARGS",
            "media",
        ),
        (
            r#"{"cmd":"lane.create","kind":"spine"}"#,
            "BAD_ARGS",
            "kind",
        ),
        (
            r#"{"cmd":"marker.add","at":{"marker":"mrk_2","offset":false}}"#,
            "BAD_ARGS",
            "offset",
        ),
        (
            r#"{"cmd":"project.set","aspect":"landscape"}"#,
            "BAD_ARGS",
            "aspect",
        ),
        (
            r#"{"cmd":"clip.speed","items":["clp_take"],"speed":0.1}"#,
            "BAD_ARGS",
            "speed",
        ),
    ] {
        let error = decode(&splice_json::parse(input).unwrap()).unwrap_err();
        assert_eq!(error.code, code, "{input}");
        assert!(error.message.contains(field), "{input}: {error}");
        assert!(!error.fix.is_empty());
        // T36.7: every BAD_ARGS names the broken rule.
        if code == "BAD_ARGS" {
            let rule = error
                .rule
                .as_ref()
                .unwrap_or_else(|| panic!("{input}: no rule"));
            assert!(
                !rule.param.is_empty() && !rule.expected.is_empty(),
                "{input}"
            );
        }
    }
    let typo =
        decode(&splice_json::parse(r#"{"cmd":"item.split","item":"clp_take","att":12}"#).unwrap())
            .unwrap_err();
    assert_eq!(
        (typo.code, typo.rule.unwrap().did_you_mean.as_deref()),
        ("UNKNOWN_PARAM", Some("at"))
    );
    let import = splice_json::parse(r#"{"cmd":"clip.insert","path":"/tmp/clip.mov"}"#).unwrap();
    assert!(
        matches!(decode(&import).unwrap().op, Op::ClipInsert { source: InsertSource::Path(path), .. } if path == "/tmp/clip.mov")
    );

    let error = describe(Some("clip.inser"), None).unwrap_err();
    assert!(error.valid.contains(&"clip.insert"));
}

/// Eval friction: agents passed bare ref names and multi-word describe queries.
#[test]
fn bare_refs_bind_with_a_dollar_and_describe_matches_any_term() {
    let op = splice_json::parse(r#"{"cmd":"marker.add","at":3,"ref":"beat"}"#).unwrap();
    assert_eq!(decode(&op).unwrap().bind.as_deref(), Some("$beat"));
    let op = splice_json::parse(r#"{"cmd":"marker.add","at":3,"ref":"$beat"}"#).unwrap();
    assert_eq!(decode(&op).unwrap().bind.as_deref(), Some("$beat"));
    let found = describe(None, Some("effect color marker split")).unwrap();
    let ids: Vec<_> = found
        .field("commands")
        .and_then(Json::as_array)
        .unwrap()
        .iter()
        .filter_map(|row| row.field("id").and_then(Json::as_str))
        .collect();
    for id in ["effect.set", "marker.add", "item.split"] {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
    let music = describe(None, Some("music")).unwrap();
    assert!(format!("{music:?}").contains("clip.insert"), "{music:?}");
}
