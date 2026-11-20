use apple_sys::{
    capture::{InputSourceKind, InputState, Recording, Rect},
    events::{Event, InputEvent, InputKind},
    image::{self, CursorImage},
};
use std::{
    char::decode_utf16,
    fs::{self, File, OpenOptions},
    io::{self, BufWriter, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

/// Writes a take's input log and its content-addressed cursor assets.
pub struct EventLog {
    directory: PathBuf,
    staged: PathBuf,
    file: BufWriter<File>,
    empty: bool,
    last_cursor: Option<[u8; 32]>,
    /// Capture bounds in global points (x, y, width, height), to normalize caret samples.
    bounds: [f64; 4],
    /// The last caret written, normalized, and when the next sample is due (take ns).
    last_caret: Option<[f64; 4]>,
    caret_due: u64,
}

/// Caret samples come at most this often (10 Hz), and only when the caret moved.
const CARET_PERIOD_NS: u64 = 100_000_000;

impl EventLog {
    /// Starts a log after the first screen sample establishes its clock. `windows` are every
    /// window of a multi-window take in file order (T46.6), in global points; events stay
    /// normalized to `source`, the first of them.
    pub fn open(directory: &Path, source: InputState, windows: &[Rect]) -> io::Result<Self> {
        let kind = match source.kind {
            InputSourceKind::Display => "display",
            InputSourceKind::Window => "window",
            InputSourceKind::Area => "area",
        };
        let rect = source.rect;
        if ![rect.x, rect.y, rect.width, rect.height, source.scale]
            .into_iter()
            .all(f64::is_finite)
            || rect.width <= 0.0
            || rect.height <= 0.0
            || source.scale <= 0.0
            || windows.iter().any(|window| {
                ![window.x, window.y, window.width, window.height]
                    .into_iter()
                    .all(f64::is_finite)
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid event source geometry.",
            ));
        }
        let staged = directory.join(".events.json.tmp");
        let mut log = Self {
            directory: directory.to_path_buf(),
            file: BufWriter::with_capacity(65536, new_file(&staged)?),
            staged,
            empty: true,
            last_cursor: None,
            bounds: [rect.x, rect.y, rect.width, rect.height],
            last_caret: None,
            caret_due: 0,
        };
        fs::create_dir(directory.join("cursors"))?;
        write!(
            log.file,
            "{{\"t0\":{},\"source\":{{\"kind\":\"{kind}\",\"rect\":{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}},\"scale\":{}",
            seconds(source.origin_ns),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            source.scale,
        )?;
        if !windows.is_empty() {
            write!(log.file, ",\"windows\":[")?;
            for (n, window) in windows.iter().enumerate() {
                write!(
                    log.file,
                    "{}{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}",
                    if n == 0 { "" } else { "," },
                    window.x,
                    window.y,
                    window.width,
                    window.height,
                )?;
            }
            write!(log.file, "]")?;
        }
        write!(log.file, "}},\"events\":[")?;
        Ok(log)
    }

    /// Drains queued input using stack storage and samples the cursor on the application thread.
    pub fn poll(&mut self, recording: &mut Recording) -> io::Result<()> {
        let mut buffer = [Event::default(); 256];
        loop {
            let count = recording.read_input(&mut buffer);
            self.append(&buffer[..count])?;
            if count < buffer.len() {
                break;
            }
        }
        if let Some(state) = recording.input_state()
            && state.active
            && let Some(cursor) =
                image::system_cursor(self.last_cursor.as_ref()).map_err(io::Error::other)?
        {
            self.cursor(state.elapsed_ns, &cursor)?;
        }
        if let Some(state) = recording.input_state()
            && state.active
            && state.elapsed_ns >= self.caret_due
        {
            self.caret_due = state.elapsed_ns + CARET_PERIOD_NS;
            self.caret(state.elapsed_ns)?;
        }
        Ok(())
    }

    /// Samples the focused text caret through Accessibility (T45.4), normalized to the source
    /// like pointer positions. Without the permission there is no sample and nothing is written:
    /// consumers fall back to the pointer.
    fn caret(&mut self, at: u64) -> io::Result<()> {
        let Some([x, y, w, h]) = apple_sys::events::caret() else {
            return Ok(());
        };
        let [left, top, width, height] = self.bounds;
        let normalized = [
            (x - left) / width,
            (y - top) / height,
            w / width,
            h / height,
        ];
        if self.last_caret == Some(normalized) {
            return Ok(());
        }
        self.last_caret = Some(normalized);
        self.begin(seconds(at), "caret")?;
        let [x, y, w, h] = normalized;
        write!(self.file, ",\"x\":{x},\"y\":{y},\"w\":{w},\"h\":{h}}}")
    }

    fn append(&mut self, records: &[Event]) -> io::Result<()> {
        for record in records {
            let event = InputEvent::decode(record).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "Invalid native input record.")
            })?;
            let kind = match event.kind {
                InputKind::Move => "move",
                InputKind::Down => "down",
                InputKind::Up => "up",
                InputKind::Drag => "drag",
                InputKind::Scroll => "scroll",
                InputKind::Key => "key",
                InputKind::Flags => "flags",
            };
            self.begin(seconds(event.time_ns), kind)?;
            match event.kind {
                InputKind::Key | InputKind::Flags => {
                    write!(
                        self.file,
                        ",\"keyCode\":{},\"mods\":{}",
                        event.code, event.modifiers
                    )?;
                    if !event.text().is_empty() {
                        self.file.write_all(b",\"ch\":\"")?;
                        write_chars(&mut self.file, event.text())?;
                        self.file.write_all(b"\"")?;
                    }
                }
                _ => {
                    write!(
                        self.file,
                        ",\"x\":{},\"y\":{}",
                        event.point[0], event.point[1]
                    )?;
                    match event.kind {
                        InputKind::Down | InputKind::Up | InputKind::Drag => {
                            write!(self.file, ",\"b\":{}", event.code)?
                        }
                        InputKind::Scroll => write!(
                            self.file,
                            ",\"dx\":{},\"dy\":{}",
                            event.scroll[0], event.scroll[1]
                        )?,
                        _ => {}
                    }
                }
            }
            self.file.write_all(b"}")?;
        }
        Ok(())
    }

    fn cursor(&mut self, at: u64, cursor: &CursorImage) -> io::Result<()> {
        let metadata = cursor.info;
        if self.last_cursor == Some(metadata.hash) {
            return Ok(());
        }
        let mut hash = [0u8; 64];
        let digits = b"0123456789abcdef";
        for (pair, byte) in hash.chunks_exact_mut(2).zip(metadata.hash) {
            pair[0] = digits[(byte >> 4) as usize];
            pair[1] = digits[(byte & 15) as usize];
        }
        let id = str::from_utf8(&hash).expect("hexadecimal cursor ID");
        let directory = self.directory.join("cursors");
        let png = directory.join(format!("{id}.png"));
        let json = directory.join(format!("{id}.json"));
        // A repeated shape after another cursor uses the files already written for this digest.
        let png_exists = png.try_exists()?;
        if png_exists != json.try_exists()? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Incomplete cursor asset.",
            ));
        }
        if !png_exists {
            let mut image = new_file(&png)?;
            image.write_all(&cursor.png)?;
            image.sync_all()?;
            let mut metadata_file = new_file(&json)?;
            write!(
                metadata_file,
                "{{\"hotX\":{},\"hotY\":{},\"scale\":{}}}",
                metadata.hot_x, metadata.hot_y, metadata.scale,
            )?;
            metadata_file.sync_all()?;
        }
        self.begin(seconds(at), "cursor")?;
        write!(self.file, ",\"id\":\"{id}\"}}")?;
        self.last_cursor = Some(metadata.hash);
        Ok(())
    }

    fn begin(&mut self, at: f64, kind: &str) -> io::Result<()> {
        if !self.empty {
            self.file.write_all(b",")?;
        }
        self.empty = false;
        write!(self.file, "{{\"t\":{at},\"k\":\"{kind}\"")
    }

    /// Publishes the sealed log without replacing an existing take file.
    pub fn finish(mut self, dropped: u64) -> io::Result<()> {
        writeln!(self.file, "],\"dropped\":{dropped}}}")?;
        self.file.flush()?;
        self.file.get_ref().sync_all()?;
        File::open(self.directory.join("cursors"))?.sync_all()?;
        fs::hard_link(&self.staged, self.directory.join("events.json"))?;
        fs::remove_file(&self.staged)?;
        File::open(&self.directory)?.sync_all()
    }
}

impl Drop for EventLog {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.staged);
    }
}

fn new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

fn seconds(time_ns: u64) -> f64 {
    time_ns as f64 / 1_000_000_000.0
}

fn write_chars(output: &mut dyn Write, text: &[u16]) -> io::Result<()> {
    let mut utf8 = [0u8; 4];
    for character in decode_utf16(text.iter().copied()) {
        match character.unwrap_or(char::REPLACEMENT_CHARACTER) {
            '"' => output.write_all(b"\\\"")?,
            '\\' => output.write_all(b"\\\\")?,
            character if character < ' ' => write!(output, "\\u{:04x}", u32::from(character))?,
            character => output.write_all(character.encode_utf8(&mut utf8).as_bytes())?,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use apple_sys::image::CursorInfo;

    #[test]
    fn event_log_round_trips_text_and_reuses_cursor_assets() {
        let path = std::env::temp_dir().join(format!("splice-event-log-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let source = InputState {
            kind: InputSourceKind::Area,
            rect: Rect {
                x: -50.0,
                y: 10.0,
                width: 200.0,
                height: 100.0,
            },
            scale: 2.0,
            origin_ns: 8_000_000_000,
            elapsed_ns: 0,
            active: true,
        };
        let other = Rect {
            x: 160.0,
            y: 10.0,
            width: 80.0,
            height: 60.0,
        };
        let mut log = EventLog::open(&path, source, &[source.rect, other]).unwrap();
        let mut key = Event {
            kind: 6,
            time_ns: 250_000_000,
            ..Event::default()
        };
        let text = [0x22_u16, 0x5c, 0x1f, 0xd83d, 0xde42];
        key.payload[44..48].copy_from_slice(&(text.len() as u32).to_ne_bytes());
        for (slot, character) in key.payload[48..].chunks_exact_mut(2).zip(text) {
            slot.copy_from_slice(&character.to_ne_bytes());
        }
        log.append(&[key]).unwrap();
        let cursor = CursorImage {
            info: CursorInfo {
                hot_x: 3.0,
                hot_y: 4.0,
                scale: 2.0,
                hash: [0xa1; 32],
            },
            png: image::png(&[255, 255, 255, 255], [1, 1], 0).unwrap(),
        };
        log.cursor(300_000_000, &cursor).unwrap();
        log.cursor(310_000_000, &cursor).unwrap();
        let alternate = CursorImage {
            info: CursorInfo {
                hot_x: 8.0,
                hash: [0xb6; 32],
                ..cursor.info
            },
            png: cursor.png.clone(),
        };
        log.cursor(320_000_000, &alternate).unwrap();
        log.cursor(330_000_000, &cursor).unwrap();
        assert!(!path.join("events.json").exists());
        log.finish(2).unwrap();

        let saved = fs::read_to_string(path.join("events.json")).unwrap();
        let document = splice_json::parse(&saved).unwrap();
        let events = document.field("events").unwrap().as_array().unwrap();
        assert_eq!(
            document.field("t0").and_then(splice_json::Value::as_f64),
            Some(8.0)
        );
        // Every window of a multi-window take, in global points (T46.6).
        assert!(saved.starts_with(
            r#"{"t0":8,"source":{"kind":"area","rect":{"x":-50,"y":10,"width":200,"height":100},"scale":2,"windows":[{"x":-50,"y":10,"width":200,"height":100},{"x":160,"y":10,"width":80,"height":60}]},"events":["#
        ), "{saved}");
        assert_eq!(
            document
                .field("dropped")
                .and_then(splice_json::Value::as_u64),
            Some(2)
        );
        assert_eq!(
            events[0].field("ch").and_then(splice_json::Value::as_str),
            Some("\"\\\u{1f}🙂")
        );
        assert_eq!(events.len(), 4);
        assert_eq!(events[1].field("id"), events[3].field("id"));
        let id = events[1].field("id").unwrap().as_str().unwrap();
        assert_eq!(
            &fs::read(path.join(format!("cursors/{id}.png"))).unwrap()[..8],
            b"\x89PNG\r\n\x1a\n"
        );
        let metadata = fs::read_to_string(path.join(format!("cursors/{id}.json"))).unwrap();
        assert_eq!(
            splice_json::parse(&metadata)
                .unwrap()
                .field("hotX")
                .and_then(splice_json::Value::as_f64),
            Some(3.0)
        );
        assert_eq!(fs::read_dir(path.join("cursors")).unwrap().count(), 4);
        assert!(!path.join(".events.json.tmp").exists());
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn event_log_serializes_warm_records_without_allocating() {
        let path = std::env::temp_dir().join(format!("splice-event-alloc-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        let mut log = EventLog::open(
            &path,
            InputState {
                kind: InputSourceKind::Display,
                rect: Rect {
                    width: 10.0,
                    height: 10.0,
                    ..Rect::default()
                },
                scale: 1.0,
                origin_ns: 0,
                elapsed_ns: 0,
                active: true,
            },
            &[],
        )
        .unwrap();
        let events = [Event {
            kind: 1,
            ..Event::default()
        }; 256];
        let meter =
            splice_testkit::alloc::Meter::new(mem_sys::local_alloc_count, mem_sys::local_reset);
        meter.warm(&mut || log.append(&events).unwrap());
        log.finish(0).unwrap();
        fs::remove_dir_all(path).unwrap();
    }
}
