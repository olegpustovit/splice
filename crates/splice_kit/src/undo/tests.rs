use super::*;

fn entry(payload: u64, label: &'static str, merge: Merge, time_ms: u64) -> Entry {
    Entry {
        label: Cow::Borrowed(label),
        merge,
        time_ms,
        payload,
    }
}

#[test]
fn history_merges_gestures_and_bursts_until_their_boundaries() {
    let burst = Merge::Burst {
        key: 7,
        window_ms: 250,
    };
    let other_burst = Merge::Burst {
        key: 8,
        window_ms: 250,
    };
    for (previous, next, at, seal, target) in [
        (Merge::Step, Merge::Step, 1100, false, 11),
        (Merge::Gesture(7), Merge::Gesture(7), 900, false, 10),
        (Merge::Gesture(7), Merge::Gesture(8), 1100, false, 11),
        (Merge::Gesture(7), Merge::Gesture(7), 1100, true, 11),
        (burst, burst, 1249, false, 10),
        (burst, burst, 1250, false, 11),
        (burst, burst, 999, false, 11),
        (burst, burst, 1100, true, 11),
        (burst, other_burst, 1100, false, 11),
    ] {
        let mut stack = UndoStack::new(10, 3, usize::MAX);
        stack.push(entry(11, "Move", previous, 1000));
        if seal {
            stack.seal();
        }
        stack.push(entry(12, "Nudge", next, at));
        let label = if target == 10 { "Move" } else { "Nudge" };
        assert_eq!(stack.titles(), (Some(label), None));
        assert_eq!(stack.undo(), Some(target), "{previous:?} {next:?} {at}");
        assert_eq!(stack.titles().1, Some(label));
        assert_eq!(stack.redo(), Some(12));
    }

    let mut stack = UndoStack::new(0, 4, usize::MAX);
    for (payload, at) in [(1, 1000), (2, 1200), (3, 1449)] {
        stack.push(entry(payload, "Nudge", burst, at));
    }
    assert_eq!(stack.undo(), Some(0));
    assert_eq!(stack.redo(), Some(3));
    stack.push(entry(4, "Nudge", burst, 1450));
    assert_eq!(stack.undo(), Some(3));
}

#[test]
fn history_repair_preserves_redo_and_labels_and_new_edits_branch() {
    let mut stack = UndoStack::new(0, 3, usize::MAX);
    stack.push(entry(1, "Startup repair", Merge::Repair, 0));
    assert_eq!(stack.titles(), (None, None));
    stack.push(entry(2, "Agent: Add clip", Merge::Step, 1));
    stack.push(entry(3, "Move", Merge::Gesture(7), 2));
    stack.push(entry(4, "Layout repair", Merge::Repair, 3));
    stack.push(entry(5, "Move again", Merge::Gesture(7), 4));
    assert_eq!(stack.titles(), (Some("Move"), None));
    assert_eq!(stack.undo(), Some(2));
    stack.push(entry(6, "Layout repair", Merge::Repair, 5));
    stack.push(entry(6, "No change", Merge::Step, 6));
    assert_eq!(stack.titles(), (Some("Agent: Add clip"), Some("Move")));
    assert_eq!(stack.redo(), Some(5));
    assert_eq!(stack.undo(), Some(6));
    assert_eq!(stack.undo(), Some(1));
    assert_eq!(stack.redo(), Some(6));
    stack.push(entry(7, "Branch", Merge::Step, 7));
    assert_eq!(stack.redo(), None);
    assert_eq!(stack.undo(), Some(6));
    assert_eq!(stack.titles(), (Some("Agent: Add clip"), Some("Branch")));
}

#[test]
fn history_caps_retention_and_reuses_the_ring_without_allocating() {
    let mut stack = UndoStack::new(0, 3, usize::MAX);
    splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(
        &mut || {
            for payload in 1..=64 {
                stack.push(entry(payload, "Edit", Merge::Step, payload));
            }
            assert_eq!(
                (stack.undo(), stack.undo(), stack.undo(), stack.undo()),
                (Some(63), Some(62), Some(61), None)
            );
            assert_eq!(
                (stack.redo(), stack.redo(), stack.redo(), stack.redo()),
                (Some(62), Some(63), Some(64), None)
            );
        },
    );

    let slot = size_of::<Slot>();
    let mut stack = UndoStack::new(0, 20, slot * 2);
    let mut owned = String::with_capacity(slot * 2);
    owned.push_str("Agent: Add clip");
    stack.push(entry(1, "Edit", Merge::Step, 1));
    stack.push(Entry {
        label: Cow::Owned(owned),
        merge: Merge::Step,
        time_ms: 2,
        payload: 2,
    });
    assert_eq!(stack.titles().0, Some("Agent: Add clip"));
    assert_eq!(stack.undo(), Some(1));
    assert_eq!(stack.undo(), None);
    assert_eq!(stack.redo(), Some(2));
    for payload in 3..6 {
        stack.push(entry(payload, "Edit", Merge::Step, payload));
    }
    assert_eq!(
        (stack.undo(), stack.undo(), stack.undo()),
        (Some(4), Some(3), None)
    );
    stack.push(entry(6, "Replace future", Merge::Step, 6));
    stack.push(entry(7, "Edit", Merge::Step, 7));
    assert_eq!(
        (stack.undo(), stack.undo(), stack.undo()),
        (Some(6), Some(3), None)
    );

    let mut disabled = UndoStack::new(0, 0, usize::MAX);
    disabled.push(entry(1, "Edit", Merge::Step, 1));
    assert_eq!(disabled.undo(), None);
}
