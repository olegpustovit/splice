use super::super::{Selection, paint_spec, tests::paint_spec_titlebar};
use super::*;
use pxui::{Event, FrameIn, geom::Rect, testing};
use splice_testkit::alloc::Meter;

#[test]
fn keymap_draws_one_keycap_per_chord() {
    let mut ui = crate::viewer_ui().unwrap();
    ui.set_a11y(true);
    let bindings = [
        BindingDescription {
            id: commands::DELETE,
            group: "Edit",
            title: "Delete",
            keys: "⌫ · ⌦".into(),
        },
        BindingDescription {
            id: commands::SHORTCUTS,
            group: "View",
            title: "Keyboard shortcuts",
            keys: "?".into(),
        },
    ];
    let frame = FrameIn::new(480.0, 140.0);
    ui.frame(&frame, &mut |ui| keymap(ui, &bindings, 480.0))
        .unwrap();
    let label = testing::find(&ui, Role::Cell, "⌫ · ⌦").unwrap();
    let fills = ui
        .layers()
        .flat_map(|layer| layer.instances)
        .filter(|instance| {
            instance.color == [SURFACE.r, SURFACE.g, SURFACE.b, SURFACE.a]
                && (instance.rect[3] - instance.rect[1] - 16.0).abs() < 0.1
        })
        .collect::<Vec<_>>();
    assert_eq!(fills.len(), 3);
    assert!((fills[0].rect[1] - label.y).abs() < 0.1);
    assert_eq!(fills[0].rect[1], fills[1].rect[1]);
    assert!((fills[1].rect[0] - fills[0].rect[2] - 3.0).abs() < 0.1);
    assert!(fills[2].rect[1] > fills[1].rect[1]);
}

#[test]
fn fixture_operations_and_live_catalog_checks() {
    let missing = BindingDescription {
        id: splice_kit::CommandId(u32::MAX),
        group: "Test",
        title: "Missing native command",
        keys: "⇧Z".into(),
    };
    let report = check::Report::run(&[missing]);
    assert_eq!(report.keymap, 1);
    assert_eq!(report.tools, tool_catalog().len());
    assert_eq!(report.revision, Some(0));
    assert_eq!(report.examples.len(), built());
    assert!(
        report
            .problems
            .iter()
            .all(|problem| problem.surface != "Registry")
    );
    assert!(
        report
            .problems
            .iter()
            .any(|p| p.name == "Missing native command")
    );
    let result = report
        .examples
        .iter()
        .find(|row| row.command == "item.split")
        .unwrap();
    assert!(
        result
            .result
            .as_ref()
            .unwrap()
            .0
            .to_lowercase()
            .contains("split")
    );
    println!(
        "FIXTURE|{}/{} Core dry runs in {} ms",
        report.passed(),
        report.examples.len(),
        report.milliseconds
    );
    for row in &report.examples {
        if let Err((code, message)) = &row.result {
            println!("REFUSAL|{}|{code}|{message}", row.command);
        }
    }

    assert_eq!(report.passed(), built());
    assert_eq!(report.problems.len(), 1);
    let mut sample = check::load_sample().unwrap();
    let before = splice_core::package::snapshot(&sample);
    let command = splice_core::commands::lookup("clip.join").unwrap();
    let op = splice_json::parse(command.example).unwrap();
    let (summary, _) = check::run_example(&mut sample, command.id, op).unwrap();
    assert!(summary.to_lowercase().contains("join"), "{summary}");
    assert_eq!(splice_core::package::snapshot(&sample), before);
    let refused =
        splice_json::parse(r#"{"cmd":"clip.join","items":["clp_missing","$cut.right"]}"#).unwrap();
    let (code, message) = check::run_example(&mut sample, command.id, refused).unwrap_err();
    assert_eq!(code, "NOT_FOUND");
    assert!(message.contains("clp_missing"));
    assert_eq!(splice_core::package::snapshot(&sample), before);

    let command = splice_core::commands::lookup("html.add").unwrap();
    let op = splice_json::parse(command.example).unwrap();
    assert!(
        sample
            .media
            .iter()
            .all(|medium| medium.path != "media/html-5f1c.mov")
    );
    let preview = check::preview_html(&sample, &[op]).unwrap();
    assert!(preview.dry_run);
    assert_eq!(
        preview.revision,
        before
            .field("rev")
            .and_then(splice_json::Value::as_i64)
            .unwrap() as u64
    );
    let staged = preview.staged.as_ref().unwrap();
    let added = staged
        .media
        .iter()
        .find(|medium| medium.path == "media/html-5f1c.mov")
        .unwrap();
    assert_eq!(
        (added.probe.width, added.probe.height, added.probe.frames),
        (270, 480, 90)
    );
    assert!(preview.report.mints.contains(&added.id.to_string()));
    let [change] = preview.report.changes.as_slice() else {
        panic!("one HTML edit")
    };
    assert_eq!(change.cmd, "html.add");
    let [splice_core::commands::Target::Timeline(splice_model::TimelineId::Item(id))] =
        change.created.as_slice()
    else {
        panic!("a new overlay")
    };
    let slot = staged.items.find(*id).unwrap();
    let splice_model::ItemKind::Media(overlay) = &staged.items.data(slot).unwrap().kind else {
        panic!("a media overlay")
    };
    assert_eq!(overlay.media, added.id);
    assert_eq!(staged.items.timing(slot).unwrap().length, 90);
    let stale = splice_json::parse(r#"{"cmd":"html.add","html":"<p>changed</p>","at":30,"length":90,"movie":"media/html-5f1c.mov"}"#).unwrap();
    let (code, message) = check::run_example(&mut sample, command.id, stale).unwrap_err();
    assert_eq!(code, "MEDIA_OFFLINE");
    assert!(message.contains("No bundled rendered movie"));
    assert_eq!(splice_core::package::snapshot(&sample), before);

    let batch = splice_json::parse(MCP_DRY_RUN).unwrap();
    let ops = batch
        .field("ops")
        .and_then(splice_json::Value::as_array)
        .unwrap();
    let applied = splice_core::edit::apply(
        &mut sample,
        ops,
        splice_core::package::journal::Source::Agent,
        true,
    )
    .unwrap();
    assert_eq!(
        applied.changes.iter().map(|c| c.cmd).collect::<Vec<_>>(),
        ["item.split", "overlay.add", "item.delete"]
    );
    assert!(!applied.side_effects.is_empty());
    assert_eq!(splice_core::package::snapshot(&sample), before);
}

#[test]
fn registry_spec_960() {
    verify(960.0);
}

#[test]
fn registry_spec_800() {
    verify(800.0);
}

#[test]
fn registry_spec_480() {
    verify(480.0);
}

fn verify(width: f32) {
    let keymap = std::env::temp_dir().join(format!(
        "splice-spec-registry-bindings-{}-{width}",
        std::process::id()
    ));
    std::fs::write(&keymap, "F5 = item.split\n").unwrap();
    let keys = commands::Keys::new(Some(&keymap));
    std::fs::remove_file(&keymap).unwrap();
    let shortcut = keys
        .bindings()
        .iter()
        .find(|row| row.id == commands::SPLIT)
        .unwrap()
        .keys
        .clone();
    assert!(shortcut.contains("F5"));
    let mut doc = Document {
        ui: crate::viewer_ui().unwrap(),
        input: FrameIn::new(width, 640.0),
        selection: Selection::default(),
        keys,
        parity: Parity::default(),
        width,
    };
    doc.ui.set_a11y(true);
    doc.draw(&[]);
    doc.draw(&[]);
    let usable = width - if width > 820.0 { 284.0 } else { 32.0 };

    doc.click(Role::Button, "Command registry");
    doc.draw(&[]);
    let heading = doc.geometry(Role::Heading, "Command registry");
    let code = doc.geometry(Role::ScrollArea, "Registry command descriptor");
    assert!((code.y - heading.y - 47.2).abs() < 0.15);
    assert!((code.w - usable).abs() < 0.1 && (code.h - 176.4).abs() < 0.15);
    let table = doc.geometry(Role::Table, "Command map");
    assert!((table.w - usable.max(571.45)).abs() < 0.2);
    assert!(doc.find(Role::Cell, &shortcut).h > 0.0);
    assert!(doc.find(Role::Table, "View commands").y > table.y);
    assert!(doc.find(Role::Table, "Gestures").y > doc.find(Role::Table, "Keymap").y);
    doc.warm("Command registry");
    if width == 480.0 {
        let line = doc.find(Role::Label, "Command {");
        doc.scroll(code.x + 24.0, code.y + 24.0, -4.0, 0.0);
        assert!(doc.find(Role::Label, "Command {").x < line.x - 30.0);
        doc.scroll(width - 12.0, 320.0, 0.0, (240.0 - table.y) / 24.0);
        let before = doc.find(Role::Table, "Command map");
        assert!((228.0..252.0).contains(&before.y), "{before:?}");
        doc.scroll(120.0, before.y + 36.0, -4.0, 0.0);
        let after = doc.find(Role::Table, "Command map");
        assert!(after.x < before.x - 30.0 && (after.y - before.y).abs() < 0.2);
    }

    doc.click(Role::Button, "MCP surface");
    doc.draw(&[]);
    let mcp_heading = doc.geometry(Role::Heading, "MCP surface");
    let mcp = doc.geometry(Role::Table, "MCP tools");
    assert!((mcp.w - usable).abs() < 0.1);
    assert!(mcp.y > mcp_heading.y);
    assert!(doc.find(Role::Cell, "session.end").y > mcp.y);
    for tool in tool_catalog() {
        let rect = doc
            .ui
            .a11y_nodes()
            .iter()
            .find(|node| {
                node.is_role(Role::Cell)
                    && node.rect.y > mcp.y
                    && node.rect.y < mcp.y + mcp.h
                    && (node.rect.x - mcp.x - 8.0).abs() < 0.1
                    && doc.ui.a11y_name_of(node) == tool.name
            })
            .unwrap_or_else(|| panic!("missing MCP row for {}", tool.name))
            .rect;
        assert!((rect.h - 15.4).abs() < 0.1, "{}: {rect:?}", tool.name);
    }
    let purpose = doc.find(Role::ColumnHeader, "PURPOSE");
    let reference_x = match width {
        960.0 => 433.922,
        800.0 => 201.531,
        480.0 => 188.969,
        _ => unreachable!(),
    };
    assert!((purpose.x - reference_x).abs() < 0.5, "{purpose:?}");

    let headings = [
        "Transport and hosting",
        "Addressing and time",
        "Live editing, sessions and undo",
        "Example: edit.apply dry run",
        "Errors",
    ];
    let mut before = mcp.y;
    for title in headings {
        let h = doc.geometry(Role::Heading, title);
        assert!(h.y > before);
        before = h.y;
    }
    let example = doc.geometry(Role::ScrollArea, "edit.apply dry-run arguments");
    assert!((example.w - usable).abs() < 0.1);
    assert!(doc.find(Role::Cell, "STALE_REVISION").y > example.y);
    doc.warm("MCP surface");
    doc.click(Role::Link, "Self-improvement loop");
    doc.draw(&[]);
    assert_eq!(super::super::SECTIONS[doc.selection.section].0, "s-improve");

    doc.click(Role::Button, "Parity check");
    doc.draw(&[]);
    let title = doc.geometry(Role::Heading, "Parity check");
    let button = doc.geometry(Role::Button, "Run parity check");
    assert!((button.w - 133.2).abs() < 0.1 && button.h == 24.0);
    assert!(button.y > title.y + 150.0);
    doc.warm("Parity idle");
    assert!(doc.click(Role::Button, "Run parity check"));
    assert!(doc.parity.advance(true, doc.keys.bindings()));
    doc.draw(&[]);
    assert!(doc.find(Role::Label, "Checking the bundled sample…").y > title.y);
    assert!(!doc.click(Role::Button, "Run parity check"));
    assert!(doc.parity.advance(false, doc.keys.bindings()));
    doc.draw(&[]);
    let Parity::Complete(report) = &doc.parity else {
        panic!("real parity completion")
    };
    let passed = report.passed();
    let examples = report.examples.len();
    assert_eq!(examples, built());
    assert_eq!(passed, examples);
    assert!(report.problems.is_empty());
    assert!(doc.find(Role::Label, "PASS").y > button.y);
    assert!(testing::find(&doc.ui, Role::Table, "Parity problems").is_none());
    doc.geometry(Role::Label, "UI bindings checked");
    doc.geometry(Role::Label, "Core dry runs");
    doc.geometry(Role::Label, "built + planned commands");
    assert!(doc.find(Role::Label, &format!("{passed}/{examples}")).y > button.y);
    assert!(testing::find(&doc.ui, Role::Table, "edit.apply fixture results").is_none());
    doc.warm("Parity complete");
    doc.click(Role::Button, "edit.apply fixture results");
    doc.draw(&[]);
    let results = doc.geometry(Role::Table, "edit.apply fixture results");
    assert!((results.w - usable).abs() < 0.1);
    assert!(doc.find(Role::ColumnHeader, "SIDE EFFECTS").y > title.y);
    doc.warm("Parity expanded");
}

struct Document {
    ui: Ui,
    input: FrameIn<'static>,
    selection: Selection,
    keys: commands::Keys,
    parity: Parity,
    width: f32,
}

impl Document {
    fn draw(&mut self, events: &[Event]) -> bool {
        let mut next = None;
        self.ui
            .frame(&self.input.events(events), &mut |ui| {
                ui.column(Node::column().w(Size::Grow).h(Size::Grow), |ui| {
                    next = paint_spec(
                        ui,
                        self.selection,
                        (self.keys.bindings(), &self.parity),
                        &mut paint_spec_titlebar,
                    )
                    .or(next);
                });
            })
            .unwrap();
        if let Some(next) = next {
            self.selection = next;
        }
        self.selection.take_parity_request()
    }

    fn find(&self, role: Role, name: &str) -> Rect {
        testing::find(&self.ui, role, name).unwrap_or_else(|| panic!("missing {role:?} {name}"))
    }

    fn geometry(&self, role: Role, name: &str) -> Rect {
        let rect = self.find(role, name);
        println!(
            "GEOM|{}|{name}|{}|{}|{}|{}",
            self.width, rect.x, rect.y, rect.w, rect.h
        );
        rect
    }

    fn click(&mut self, role: Role, name: &str) -> bool {
        let nav = role == Role::Button
            && ["Command registry", "MCP surface", "Parity check"].contains(&name);
        let compact = self.width <= 820.0;
        if nav && compact && self.selection.scroll > 0.0 {
            self.scroll(
                self.width - 12.0,
                290.0,
                0.0,
                self.selection.scroll / 24.0 + 1.0,
            );
        }
        let rect = self.find(role, name);
        let x = rect.x + rect.w / 2.0;
        let y = rect.y + rect.h / 2.0;
        if nav && compact && !(6.0..self.width - 6.0).contains(&x) {
            self.draw(&[
                Event::pointer(Event::POINTER_MOVED, 0, 120.0, 48.0),
                Event::pointer(
                    Event::SCROLLED,
                    0,
                    f64::from((self.width / 2.0 - x) / 24.0),
                    0.0,
                ),
            ]);
        } else if !(70.0..625.0).contains(&y) {
            let x = if nav { 120.0 } else { self.width - 24.0 };
            self.draw(&[
                Event::pointer(Event::POINTER_MOVED, 0, x.into(), 290.0),
                Event::pointer(Event::SCROLLED, 0, 0.0, f64::from((300.0 - y) / 24.0)),
            ]);
        }
        self.draw(&[]);
        let rect = self.find(role, name);
        let x = rect.x + rect.w / 2.0;
        let y = rect.y + rect.h / 2.0;
        assert!(
            (0.0..self.width).contains(&x)
                && ((if compact { 0.0 } else { 34.0 })..640.0).contains(&y),
            "{name}: {rect:?}"
        );
        self.draw(&[Event::pointer(
            Event::POINTER_PRESSED,
            0,
            x.into(),
            y.into(),
        )]);
        self.draw(&[Event::pointer(
            Event::POINTER_RELEASED,
            0,
            x.into(),
            y.into(),
        )])
    }

    fn scroll(&mut self, x: f32, y: f32, dx: f32, dy: f32) {
        self.draw(&[
            Event::pointer(Event::POINTER_MOVED, 0, x.into(), y.into()),
            Event::pointer(Event::SCROLLED, 0, dx.into(), dy.into()),
        ]);
        self.draw(&[]);
    }

    fn warm(&mut self, name: &str) {
        for _ in 0..8 {
            self.draw(&[]);
        }
        Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset).warm(&mut || {
            self.draw(&[]);
        });
        println!("WARM|{}|{name}|0 allocations", self.width);
    }
}
