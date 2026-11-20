use crate::{
    Json, text,
    wire::{Args, bad},
};
use splice_core::{commands::CommandError, history::StoreError};
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub(super) enum Source {
    Display(u32),
    Window(u32),
    NamedWindow { app: String, title: Option<String> },
    Area(u32, [f64; 4]),
}

/// A microphone or camera choice: `false`, `true` (the system default), or a device ID or name
/// from `record.start {dryRun:true}` `devices`.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Device {
    Off,
    Default,
    Named(String),
}

impl Device {
    pub fn on(&self) -> bool {
        *self != Self::Off
    }
}

/// `systemAudio`: off, every app's sound, or only these apps (names or bundle IDs).
#[derive(Clone, Debug, PartialEq)]
pub(super) enum SystemAudio {
    Off,
    All,
    Apps(Vec<String>),
}

impl SystemAudio {
    pub fn on(&self) -> bool {
        *self != Self::Off
    }
}

/// A microphone or camera present on this Mac (`record.start {dryRun:true}` `devices`).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct DeviceInfo {
    /// `mic` or `camera`.
    pub kind: String,
    /// AVFoundation's unique ID: the same for a device across launches and reboots.
    pub id: String,
    pub name: String,
    /// The system default for its kind.
    pub default: bool,
}

/// Longest countdown, in seconds.
const COUNTDOWN_LIMIT: f64 = 3600.0;
/// Window resize bounds, in points.
const SIZE_RANGE: (f64, f64) = (100.0, 16384.0);

pub(super) struct Options {
    /// Apps a display capture leaves out (names or bundle IDs).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub exclude: Vec<String>,
    /// `exclude:["desktop"]`: hide the Finder's desktop icons from a display or area capture.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub hide_desktop: bool,
    pub source: Source,
    /// Resize the window to this many points before capture (window sources only).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub size: Option<[f64; 2]>,
    pub duration: Option<f64>,
    /// `then:"pause"`: a timed take pauses at `duration` instead of stopping.
    pub then_pause: bool,
    pub countdown: Duration,
    pub mic: Device,
    pub system_audio: SystemAudio,
    pub camera: Device,
    pub events: bool,
    pub still: bool,
}

impl Options {
    pub fn parse(args: &Json) -> Result<Self, StoreError> {
        let still = args.flag("still")?;
        let duration = args
            .field("duration")
            .map(|value| {
                value
                    .as_f64()
                    .filter(|&seconds| {
                        Duration::try_from_secs_f64(seconds)
                            .is_ok_and(|duration| !duration.is_zero())
                    })
                    .ok_or_else(|| bad("duration", "positive seconds in the supported range"))
            })
            .transpose()?;
        let then_pause = then(args, duration.is_some())?;
        // Seconds, like duration (A2); fractions count down that long.
        let countdown = args
            .field("countdown")
            .map(|value| {
                value
                    .as_f64()
                    .filter(|seconds| (0.0..=COUNTDOWN_LIMIT).contains(seconds))
                    .map(Duration::from_secs_f64)
                    .ok_or_else(|| bad("countdown", "seconds from 0 to 3600"))
            })
            .transpose()?
            .unwrap_or_default();
        // A voice-over records the microphone; its screen track is not used.
        let voiceover = args.flag("voiceover")?;
        let mic = match device(args, "mic")? {
            Device::Off if voiceover => Device::Default,
            mic => mic,
        };
        let source = source(args)?;
        let window = matches!(source, Source::Window(_) | Source::NamedWindow { .. });
        // A window or app capture usually wants its sound: record it unless told otherwise.
        let system_audio = match args.field("systemAudio") {
            None if !still && window => SystemAudio::All,
            None | Some(Json::Bool(false)) => SystemAudio::Off,
            Some(Json::Bool(true)) => SystemAudio::All,
            Some(value) => SystemAudio::Apps(apps(value)?),
        };
        let camera = device(args, "camera")?;
        let input = args.flag("events")?;
        if still && (duration.is_some() || mic.on() || system_audio.on() || camera.on() || input) {
            return Err(bad(
                "still",
                "true for a single image without recording tracks or duration",
            ));
        }
        let mut exclude: Vec<String> = match args.field("exclude") {
            None => Vec::new(),
            Some(list) => list
                .as_array()
                .ok_or_else(|| bad("exclude", "an array of app names or bundle IDs"))?
                .iter()
                .map(|name| {
                    name.as_str()
                        .filter(|n| !n.is_empty() && n.len() < 256)
                        .map(str::to_owned)
                        .ok_or_else(|| bad("exclude", "app names or bundle IDs, or \"desktop\""))
                })
                .collect::<Result<_, _>>()?,
        };
        let listed = exclude.len();
        exclude.retain(|name| name != "desktop");
        let hide_desktop = exclude.len() != listed;
        if window && listed > 0 {
            return Err(bad(
                "exclude",
                "nothing with a window source (a window capture shows only that window); exclude works with display and area",
            ));
        }
        let size = args.field("size").map(window_size).transpose()?;
        if size.is_some() && !window {
            return Err(bad(
                "size",
                "a window source: source:{window:{app}} with size:[width, height] in points",
            ));
        }
        Ok(Self {
            source,
            size,
            duration,
            then_pause,
            exclude,
            hide_desktop,
            countdown,
            mic,
            system_audio,
            camera,
            events: !still && !voiceover && (input || args.field("events").is_none()),
            still,
        })
    }
}

/// What a timed take does at `elapsed` recorded seconds: `None` keeps recording, `Some(true)`
/// pauses (`then:"pause"`), `Some(false)` stops. A paused take never reaches its limit.
pub(super) fn at_limit(
    elapsed: f64,
    limit: Option<f64>,
    paused: bool,
    then_pause: bool,
) -> Option<bool> {
    limit
        .filter(|limit| !paused && elapsed >= *limit)
        .map(|_| then_pause)
}

/// `then`: what a timed take does when `duration` seconds are recorded; true for `pause`.
pub(super) fn then(args: &Json, timed: bool) -> Result<bool, StoreError> {
    match args.optional_string("then")? {
        None => Ok(false),
        Some(_) if !timed => Err(bad(
            "then",
            "omitted, or \"stop\" or \"pause\" together with duration (seconds of recorded time)",
        )),
        Some("stop") => Ok(false),
        Some("pause") => Ok(true),
        Some(_) => Err(bad("then", "\"stop\" (the default) or \"pause\"")),
    }
}

fn device(args: &Json, name: &str) -> Result<Device, StoreError> {
    match args.field(name) {
        None | Some(Json::Bool(false)) => Ok(Device::Off),
        Some(Json::Bool(true)) => Ok(Device::Default),
        Some(Json::String(id)) if !id.is_empty() && id.len() < 512 => Ok(Device::Named(id.clone())),
        Some(_) => Err(bad(
            name,
            "true for the default device, false, or a device ID from record.start {dryRun:true} devices",
        )),
    }
}

fn apps(value: &Json) -> Result<Vec<String>, StoreError> {
    let expected =
        "true, false, or {apps:[\"com.apple.Safari\", \"Music\"]} (bundle IDs or app names)";
    let list = value
        .as_object()
        .filter(|fields| fields.len() == 1)
        .and_then(|_| value.field("apps"))
        .and_then(Json::as_array)
        .filter(|list| !list.is_empty() && list.len() <= 64)
        .ok_or_else(|| bad("systemAudio", expected))?;
    list.iter()
        .map(|app| {
            app.as_str()
                .filter(|name| !name.is_empty() && name.len() < 256)
                .map(str::to_owned)
                .ok_or_else(|| bad("systemAudio.apps", expected))
        })
        .collect()
}

fn window_size(value: &Json) -> Result<[f64; 2], StoreError> {
    let expected = "[width, height] in points, each from 100 to 16384";
    let sides = match value.as_array() {
        Some([width, height]) => [width.as_f64(), height.as_f64()],
        _ => return Err(bad("size", expected)),
    };
    match sides {
        [Some(width), Some(height)]
            if [width, height]
                .iter()
                .all(|side| (SIZE_RANGE.0..=SIZE_RANGE.1).contains(side)) =>
        {
            Ok([width.round(), height.round()])
        }
        _ => Err(bad("size", expected)),
    }
}

/// The device `choice` selects among `devices`, or `NO_DEVICE` naming the valid IDs. `kind` is
/// `mic` or `camera`, which is also the argument's name.
pub(super) fn resolve_device(
    kind: &str,
    choice: &Device,
    devices: &[DeviceInfo],
) -> Result<Option<DeviceInfo>, StoreError> {
    let present = || devices.iter().filter(|device| device.kind == kind);
    let found = match choice {
        Device::Off => return Ok(None),
        Device::Default => present()
            .find(|device| device.default)
            .or_else(|| present().next()),
        Device::Named(wanted) => present()
            .find(|device| device.id == *wanted || device.name.eq_ignore_ascii_case(wanted)),
    };
    if let Some(device) = found {
        return Ok(Some(device.clone()));
    }
    let noun = if kind == "mic" {
        "microphone"
    } else {
        "camera"
    };
    let valid: Vec<String> = present()
        .map(|device| format!("\"{}\" ({})", device.id, device.name))
        .collect();
    let (got, message) = match choice {
        Device::Named(wanted) => (
            Some(text(wanted)),
            format!("No {noun} has the ID or name \"{wanted}\"."),
        ),
        _ => (None, format!("This Mac has no {noun} to record from.")),
    };
    let (expected, fix) = if valid.is_empty() {
        (
            format!("{kind}:false (no {noun} is connected)"),
            format!(
                "Connect a {noun}, or record without it ({kind}:false); record.start {{dryRun:true}} lists devices."
            ),
        )
    } else {
        (
            format!("one of {}", valid.join(", ")),
            format!(
                "Pass {kind}:true for the default {noun}, or {kind}:\"<id>\" with one of {}.",
                valid.join(", ")
            ),
        )
    };
    let mut error: StoreError = CommandError::rule("NO_DEVICE", kind, got, &expected, &fix).into();
    error.message = message;
    Err(error)
}

/// The running apps (bundle ID, name) `wanted` names, by bundle ID or name; `CAPTURE_SOURCE`
/// for an app that is not running.
pub(super) fn resolve_apps(
    wanted: &[String],
    running: &[(String, String)],
) -> Result<Vec<(String, String)>, StoreError> {
    let mut found: Vec<(String, String)> = Vec::new();
    for name in wanted {
        let app = running
            .iter()
            .find(|(bundle, app)| bundle == name || app.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                let mut names: Vec<&str> = running.iter().map(|(_, app)| app.as_str()).collect();
                names.sort_unstable();
                names.dedup();
                names.truncate(24);
                let mut error: StoreError = CommandError::rule(
                    "CAPTURE_SOURCE",
                    "systemAudio.apps",
                    Some(text(name)),
                    "running apps, by bundle ID or name",
                    &format!(
                        "Start the app first, or pick running apps (record.start {{dryRun:true}} lists applications): {}.",
                        names.join(", ")
                    ),
                )
                .into();
                error.message = format!("No running app matches \"{name}\".");
                error
            })?;
        if !found.contains(app) {
            found.push(app.clone());
        }
    }
    Ok(found)
}

/// What a capture source may be; window:{app} needs no window ID or title (T36.10).
const SELECTOR: &str = "one selector: display:\"main\", window:{app:\"Safari\", title?} (or a win_ ID), or area:{display, rect}";

fn source(args: &Json) -> Result<Source, StoreError> {
    let fields = ["display", "window", "area"];
    let selectors = args.field("source");
    if let Some(selectors) = selectors
        && (fields.iter().any(|key| args.field(key).is_some())
            || !selectors
                .as_object()
                .is_some_and(|fields| fields.len() == 1))
    {
        return Err(bad("source", SELECTOR));
    }
    let input = selectors.unwrap_or(args);
    match (
        input.field("display"),
        input.field("window"),
        input.field("area"),
    ) {
        (Some(value), None, None) => Ok(Source::Display(display(value)?)),
        (None, Some(value), None) if value.as_object().is_some() => {
            let app = value.string("app")?;
            if app.is_empty() {
                return Err(bad("window.app", "an application name or bundle ID"));
            }
            Ok(Source::NamedWindow {
                app: app.into(),
                title: value.optional_string("title")?.map(str::to_owned),
            })
        }
        (None, Some(value), None) => id(value, "win_").map(Source::Window).ok_or_else(|| {
            bad(
                "window",
                "{app:\"Safari\"} (an app name or bundle ID, with an optional title) or a win_ ID from record.start dryRun",
            )
        }),
        (None, None, Some(value)) => {
            let target = value
                .field("display")
                .ok_or_else(|| bad("area.display", "main or a display ID"))?;
            let rect = value
                .field("rect")
                .ok_or_else(|| bad("area.rect", "a rectangle with x, y, width and height"))?;
            let [x, y, width, height] =
                ["x", "y", "width", "height"].map(|name| rect.field(name).and_then(Json::as_f64));
            match (x, y, width, height) {
                (Some(x), Some(y), Some(width), Some(height))
                    if [x, y, width, height].into_iter().all(f64::is_finite)
                        && x >= 0.0
                        && y >= 0.0
                        && width > 0.0
                        && height > 0.0 =>
                {
                    Ok(Source::Area(display(target)?, [x, y, width, height]))
                }
                _ => Err(bad(
                    "area.rect",
                    "nonnegative x and y, with positive width and height in display points",
                )),
            }
        }
        (None, None, None) if selectors.is_none() => Ok(Source::Display(0)),
        _ => Err(bad("source", SELECTOR)),
    }
}

fn display(value: &Json) -> Result<u32, StoreError> {
    if value.as_str() == Some("main") {
        return Ok(0);
    }
    id(value, "").ok_or_else(|| bad("display", "main or a display ID from record.start dryRun"))
}

fn id(value: &Json, prefix: &str) -> Option<u32> {
    value
        .as_str()
        .and_then(|value| value.strip_prefix(prefix).unwrap_or(value).parse().ok())
        .or_else(|| u32::try_from(value.as_u64()?).ok())
        .filter(|&id| id > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_options_select_sources_and_validate_timing() {
        let args = splice_json::parse(
            r#"{"source":{"area":{"display":"main","rect":{"x":10,"y":20,"width":240,"height":144}}},"duration":3,"countdown":1,"mic":true}"#,
        )
        .unwrap();
        let options = Options::parse(&args).unwrap();
        assert_eq!(options.source, Source::Area(0, [10.0, 20.0, 240.0, 144.0]));
        assert_eq!(options.duration, Some(3.0));
        assert_eq!(options.countdown.as_secs(), 1);
        assert_eq!(options.mic, Device::Default);
        assert!(options.events);
        assert!(!options.camera.on() && !options.system_audio.on() && !options.still);
        let window = splice_json::parse(r#"{"source":{"window":{"app":"TextEdit"}}}"#).unwrap();
        assert_eq!(
            Options::parse(&window).unwrap().system_audio,
            SystemAudio::All
        );
        let quiet =
            splice_json::parse(r#"{"source":{"window":{"app":"TextEdit"}},"systemAudio":false}"#)
                .unwrap();
        assert!(!Options::parse(&quiet).unwrap().system_audio.on());
        let by_app =
            splice_json::parse(r#"{"source":{"window":{"app":"com.apple.Safari"}}}"#).unwrap();
        assert_eq!(
            Options::parse(&by_app).unwrap().source,
            Source::NamedWindow {
                app: "com.apple.Safari".into(),
                title: None
            }
        );

        for invalid in [
            r#"{"duration":0}"#,
            r#"{"duration":1e100}"#,
            r#"{"source":{"display":"main","window":"win_1"}}"#,
            r#"{"source":{"display":"main"},"window":"win_1"}"#,
            r#"{"source":{"window":{"app":"","title":"Title"}}}"#,
            r#"{"source":{"area":{"display":"main","rect":{"x":0,"y":0,"width":0,"height":10}}}}"#,
            r#"{"still":true,"duration":3}"#,
            r#"{"countdown":-1}"#,
            r#"{"then":"pause"}"#,
            r#"{"duration":3,"then":"wait"}"#,
            r#"{"countdown":3601}"#,
            r#"{"countdown":"3"}"#,
            r#"{"mic":""}"#,
            r#"{"mic":3}"#,
            r#"{"camera":{}}"#,
            r#"{"systemAudio":{"apps":[]}}"#,
            r#"{"systemAudio":{"apps":["Music"],"other":1}}"#,
            r#"{"systemAudio":"Music"}"#,
            r#"{"still":true,"systemAudio":{"apps":["Music"]}}"#,
            r#"{"source":{"window":{"app":"TextEdit"}},"exclude":["desktop"]}"#,
            r#"{"source":{"window":{"app":"TextEdit"}},"exclude":["Slack"]}"#,
            r#"{"source":{"display":"main"},"size":[1280,720]}"#,
            r#"{"source":{"window":{"app":"TextEdit"}},"size":[99,720]}"#,
            r#"{"source":{"window":{"app":"TextEdit"}},"size":[1280]}"#,
            r#"{"source":{"window":{"app":"TextEdit"}},"size":{"width":1280}}"#,
        ] {
            let args = splice_json::parse(invalid).unwrap();
            assert_eq!(
                Options::parse(&args).err().unwrap().code,
                "BAD_ARGS",
                "{invalid}"
            );
        }
    }

    #[test]
    fn recording_options_take_devices_app_audio_desktop_and_window_size() {
        let args = splice_json::parse(
            r#"{"countdown":0.5,"mic":"BuiltInMicrophoneDevice","camera":true,"systemAudio":{"apps":["com.apple.Music","Safari"]},"exclude":["desktop","Slack"]}"#,
        )
        .unwrap();
        let options = Options::parse(&args).unwrap();
        assert_eq!(options.countdown, Duration::from_millis(500));
        assert_eq!(options.mic, Device::Named("BuiltInMicrophoneDevice".into()));
        assert_eq!(options.camera, Device::Default);
        assert_eq!(
            options.system_audio,
            SystemAudio::Apps(vec!["com.apple.Music".into(), "Safari".into()])
        );
        assert!(options.hide_desktop);
        assert_eq!(options.exclude, ["Slack"]);
        let window = splice_json::parse(
            r#"{"source":{"window":{"app":"TextEdit"}},"size":[1279.6,720],"voiceover":false}"#,
        )
        .unwrap();
        assert_eq!(Options::parse(&window).unwrap().size, Some([1280.0, 720.0]));
        let paused = splice_json::parse(r#"{"duration":3,"then":"pause"}"#).unwrap();
        assert!(Options::parse(&paused).unwrap().then_pause);
        let voiceover = splice_json::parse(r#"{"voiceover":true}"#).unwrap();
        assert_eq!(Options::parse(&voiceover).unwrap().mic, Device::Default);
    }

    #[test]
    fn recording_duration_stops_or_pauses_at_recorded_time() {
        assert_eq!(at_limit(2.99, Some(3.0), false, false), None);
        assert_eq!(at_limit(3.0, Some(3.0), false, false), Some(false));
        assert_eq!(at_limit(3.01, Some(3.0), false, true), Some(true));
        // Paused time is not recorded time; a manual take has no limit.
        assert_eq!(at_limit(9.0, Some(3.0), true, false), None);
        assert_eq!(at_limit(9.0, None, false, false), None);
        // record.start {resume, duration:3, then:"pause"} on a take paused at 3 s.
        let resume =
            splice_json::parse(r#"{"jobId":"job_1","duration":3,"then":"pause"}"#).unwrap();
        assert!(then(&resume, true).unwrap());
        assert_eq!(at_limit(5.9, Some(3.0 + 3.0), false, true), None);
        assert_eq!(at_limit(6.0, Some(3.0 + 3.0), false, true), Some(true));
    }

    #[test]
    fn recording_devices_resolve_by_id_name_or_default_and_name_valid_ids() {
        let devices = [
            ("mic", "USB-1234", "Podcast Mic", false),
            (
                "mic",
                "BuiltInMicrophoneDevice",
                "MacBook Pro Microphone",
                true,
            ),
            ("camera", "0x1424001", "FaceTime HD Camera", false),
        ]
        .map(|(kind, id, name, default)| DeviceInfo {
            kind: kind.into(),
            id: id.into(),
            name: name.into(),
            default,
        });
        let id = |choice: Device, kind: &str| {
            resolve_device(kind, &choice, &devices)
                .unwrap()
                .map(|device| device.id)
        };
        assert_eq!(id(Device::Off, "mic"), None);
        assert_eq!(
            id(Device::Default, "mic").as_deref(),
            Some("BuiltInMicrophoneDevice")
        );
        // No camera is marked default: the first one present is.
        assert_eq!(id(Device::Default, "camera").as_deref(), Some("0x1424001"));
        assert_eq!(
            id(Device::Named("USB-1234".into()), "mic").as_deref(),
            Some("USB-1234")
        );
        assert_eq!(
            id(Device::Named("podcast mic".into()), "mic").as_deref(),
            Some("USB-1234")
        );
        let unknown =
            resolve_device("mic", &Device::Named("0x1424001".into()), &devices).unwrap_err();
        assert_eq!(unknown.code, "NO_DEVICE");
        assert!(unknown.fix.contains("\"USB-1234\" (Podcast Mic)"));
        assert!(unknown.fix.contains("\"BuiltInMicrophoneDevice\""));
        assert!(!unknown.fix.contains("FaceTime"));
        let rule = unknown.rule.unwrap();
        assert_eq!(
            (rule.param.as_str(), rule.got),
            ("mic", Some(text("0x1424001")))
        );
        let none = resolve_device("camera", &Device::Default, &devices[..2]).unwrap_err();
        assert_eq!(none.code, "NO_DEVICE");
        assert!(none.fix.contains("camera:false"), "{}", none.fix);
    }

    #[test]
    fn recording_app_audio_resolves_running_apps_by_bundle_or_name() {
        let running = [
            ("com.apple.Music", "Music"),
            ("com.apple.Safari", "Safari"),
            ("com.apple.Safari", "Safari"),
        ]
        .map(|(bundle, name)| (bundle.to_owned(), name.to_owned()));
        let found = resolve_apps(&["safari".into(), "com.apple.Safari".into()], &running).unwrap();
        assert_eq!(found, [("com.apple.Safari".into(), "Safari".into())]);
        let error = resolve_apps(&["Spotify".into()], &running).unwrap_err();
        assert_eq!(error.code, "CAPTURE_SOURCE");
        assert!(error.fix.contains("Music, Safari"), "{}", error.fix);
    }

    #[test]
    fn recording_canvas_takes_the_nearest_project_aspect() {
        for (size, aspect) in [
            ([3840, 2160], "16:9"),
            ([2880, 1800], "16:10"),
            ([1440, 1080], "4:3"),
            ([1080, 1920], "9:16"),
            ([1000, 1000], "1:1"),
            ([1080, 1350], "4:5"),
            ([2560, 1080], "16:9"),
            ([800, 2000], "9:16"),
        ] {
            assert_eq!(
                crate::lints::nearest_aspect(size[0], size[1]),
                aspect,
                "{size:?}"
            );
        }
    }
}
