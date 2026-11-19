use super::*;

#[test]
fn stable_ids_round_trip_and_reject_wrong_kind() {
    let id = ClipId::from_serial(105);
    assert_eq!(id.to_string(), "clp_2x");
    assert_eq!("clp_2x".parse(), Ok(id));
    assert!("ovl_2x".parse::<ClipId>().is_err());
    for bad in ["clp_", "clp_00", "clp_2X", "clp_-1", "clp_zzzzzzzzzzzzzz"] {
        assert!(bad.parse::<ClipId>().is_err(), "{bad}");
    }
    assert_eq!(
        ClipId::from_serial(u64::MAX).to_string().parse(),
        Ok(ClipId::from_serial(u64::MAX))
    );
    assert_eq!("w12".parse(), Ok(WordId::legacy(12)));
    let word = WordId::new("med_screen".parse().unwrap(), 12);
    assert_eq!(word.to_string(), "w:screen:12");
    assert_eq!("w:screen:12".parse(), Ok(word));
    assert_eq!(WordId::legacy(7).to_string(), "w7");
    for bad in [
        "w:",
        "w:screen",
        "w:screen:",
        "w:0:1",
        "w:Scr:1",
        "w:screen:01",
        "w::1",
    ] {
        assert!(bad.parse::<WordId>().is_err(), "{bad}");
    }
}

#[test]
fn reserved_clip_and_item_edits_allocate_nothing() {
    let mut clips = Clips::with_capacity(3);
    let mut items = Items::with_capacity(2);
    let first = clips.insert(gap(1, 0)).unwrap();
    let second = clips.insert(gap(2, 30)).unwrap();
    assert_eq!(clips.insert(gap(2, 60)), None);
    assert_eq!(clips.timing(second).unwrap().start, 30);
    assert!(!clips.remove(Slot::MAX));
    assert_eq!(clips.id(Slot::MAX), None);
    let item = items.insert(title(3)).unwrap();
    let meter = splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
    meter.warm(&mut || {
        clips.timing_mut(second).unwrap().start = 0;
        clips.timing_mut(first).unwrap().start = 30;
        clips.sort_by_start();
        assert_eq!(clips.order(), [second, first]);
        assert!(clips.remove(second));
        assert_eq!(clips.timing(second), None);
        assert_eq!(clips.find(ClipId::from_serial(2)), None);
        let replacement = clips.insert(gap(4, 0)).unwrap();
        assert_eq!(replacement, second);
        assert_eq!(clips.find(ClipId::from_serial(4)), Some(second));
        assert_eq!(clips.order(), [first, second]);
        assert!(items.remove(item));
        let replacement = items.insert(title(5)).unwrap();
        assert_eq!(replacement, item);
        assert_eq!(items.find(ItemId::from_serial(5)), Some(item));
        assert!(clips.remove(second));
        assert_eq!(clips.insert(gap(2, 30)), Some(second));
    });

    assert_eq!(clips.find(ClipId::from_serial(4)), None);
    assert_eq!(items.find(ItemId::from_serial(3)), None);
    assert_eq!(items.order(), [item]);
    assert_eq!(items.timing(item).unwrap().length, 9);
}

fn gap(serial: u64, start: Frame) -> Clip {
    Clip {
        id: ClipId::from_serial(serial),
        timing: ClipTiming {
            start,
            source: 0,
            length: 30,
            speed: 1.0,
        },
        data: ClipData::default(),
    }
}

fn title(serial: u64) -> Item {
    Item {
        id: ItemId::from_serial(serial),
        timing: ItemTiming {
            anchor: Anchor::ClipSource {
                clip: ClipId::from_serial(1),
                source: 8,
            },
            length: 9,
        },
        data: ItemData {
            lane: LaneId::from_serial(2),
            kind: ItemKind::Title {
                text: String::new(),
            },
            appearance: Appearance::default(),
        },
    }
}

#[test]
fn half_open_timing_and_spine_defaults() {
    let project = Project::default();
    assert_eq!(project.fps, 30);
    assert_eq!(project.size, [1080, 1920]);
    assert_eq!(project.spine.id.to_string(), "lan_spine");
    let clip = gap(1, 10);
    assert_eq!(clip.timing.end(), Some(40));
    assert!(clip.timing.contains(10));
    assert!(clip.timing.contains(39));
    assert!(!clip.timing.contains(40));
    let overflow = ClipTiming {
        start: i32::MAX,
        ..clip.timing
    };
    assert_eq!(overflow.end(), None);
}
