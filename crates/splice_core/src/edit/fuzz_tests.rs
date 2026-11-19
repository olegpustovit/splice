//! T15.5: decoders and apply never panic on mutated arguments, and a long random op stream over
//! every row keeps every invariant and replays to the same hash.
use super::tests::sample;
use super::*;
use splice_testkit::Xorshift;

fn mutate(rng: &mut Xorshift, value: &Json) -> Json {
    let pick = |rng: &mut Xorshift, n: u64| rng.next_u64() % n;
    let junk = |rng: &mut Xorshift| match rng.next_u64() % 9 {
        0 => Json::Null,
        1 => Json::Bool(rng.next_u64() & 1 == 0),
        2 => Json::Signed(-((rng.next_u64() % 100_000) as i64) - 1),
        3 => Json::Unsigned(rng.next_u64() % 4_000_000_000),
        4 => Json::Float(f64::from(rng.next_u64() as u32) / 7.0 - 1e6),
        5 => Json::String("clp_zz".into()),
        6 => Json::String(String::new()),
        7 => Json::Array(vec![Json::Unsigned(1), Json::String("x".into())]),
        _ => Json::Object(vec![("keys".into(), Json::Array(vec![]))]),
    };
    let Json::Object(fields) = value else {
        return junk(rng);
    };
    let mut out = fields.clone();
    for _ in 0..1 + pick(rng, 3) {
        if out.len() <= 1 {
            break;
        }
        let index = 1 + pick(rng, out.len() as u64 - 1) as usize; // keep "cmd"
        match pick(rng, 4) {
            0 => {
                out.remove(index);
            }
            1 => out[index].1 = junk(rng),
            2 => out[index].1 = mutate(rng, &out[index].1.clone()),
            _ => out.push(("unexpected".into(), junk(rng))),
        }
    }
    Json::Object(out)
}

#[test]
fn mutated_arguments_for_every_row_fail_typed_never_panic() {
    let base = sample();
    let mut rng = Xorshift::new(15_5);
    let mut errors = 0;
    for _ in 0..12 {
        for row in commands::COMMANDS {
            let example = commands::describe(Some(row.id), None)
                .unwrap()
                .field("example")
                .unwrap()
                .clone();
            let op = mutate(&mut rng, &example);
            let mut project = base.clone();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                apply(
                    &mut project,
                    std::slice::from_ref(&op),
                    Source::Agent,
                    false,
                )
            }));
            match outcome {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    assert!(
                        !error.error.code.is_empty() && !error.error.fix.is_empty(),
                        "{op:?}"
                    );
                    errors += 1;
                }
                Err(_) => {
                    let mut text = String::new();
                    splice_json::write(&op, &mut text).unwrap();
                    panic!("{} panicked on {text}", row.id);
                }
            }
        }
    }
    assert!(errors > 300, "mutations should mostly fail: {errors}");
}

#[test]
fn random_stream_over_every_row_keeps_invariants_and_replays() {
    let temp = std::env::temp_dir().join(format!("splice-fuzz-{}.splice", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    let mut store = crate::history::Store::create(&temp, &sample()).unwrap();
    let mut rng = Xorshift::new(2026_1119);
    let examples: Vec<Json> = commands::COMMANDS
        .iter()
        .map(|row| {
            commands::describe(Some(row.id), None)
                .unwrap()
                .field("example")
                .unwrap()
                .clone()
        })
        .collect();
    let options = |time: u64| crate::history::WriteOptions {
        time,
        ..crate::history::WriteOptions::default()
    };
    let (mut applied, mut undone) = (0, 0);
    for time in 1..=2_000u64 {
        let roll = rng.next_u64() % 10;
        let result = if roll == 0 && store.project().rev > 0 {
            let back = crate::history::Scope::To(store.project().rev - 1);
            store.undo(back, &options(time)).map(|_| undone += 1)
        } else {
            let op = &examples[(rng.next_u64() % examples.len() as u64) as usize];
            store
                .apply(std::slice::from_ref(op), &options(time))
                .map(|_| applied += 1)
        };
        // Business errors are fine; every committed state passed the invariant checker.
        if let Err(error) = result {
            assert!(!error.code.is_empty());
        }
    }
    assert!(
        applied > 300 && undone > 50,
        "{applied} applied, {undone} undone"
    );
    let stamp = (store.project().rev, store.project().hash);
    store.close().unwrap();
    let reopened = crate::history::Store::open(&temp).unwrap();
    assert!(reopened.problem().is_none());
    assert_eq!((reopened.project().rev, reopened.project().hash), stamp);
    drop(reopened);
    let _ = std::fs::remove_dir_all(&temp);
}

#[test]
fn a_save_cut_at_any_byte_leaves_the_last_complete_project() {
    let temp = std::env::temp_dir().join(format!("splice-cut-save-{}.splice", std::process::id()));
    let _ = std::fs::remove_dir_all(&temp);
    let store = crate::history::Store::create(&temp, &sample()).unwrap();
    let stamp = (store.project().rev, store.project().hash);
    store.close().unwrap();
    // A crash while writing replaces nothing: the next save's bytes only ever reach the
    // temporary file before the atomic rename.
    let next = std::fs::read(temp.join("project.json")).unwrap();
    for cut in (0..next.len()).step_by((next.len() / 97).max(1)) {
        std::fs::write(temp.join("project.json.tmp"), &next[..cut]).unwrap();
        let store = crate::history::Store::open(&temp).unwrap();
        assert_eq!(
            (store.project().rev, store.project().hash),
            stamp,
            "cut at {cut}"
        );
        store.close().unwrap();
    }
    let _ = std::fs::remove_dir_all(&temp);
}
