use crate::{
    Json,
    wire::{Args, bad},
};
use splice_core::history::StoreError;
use std::time::Duration;

#[derive(Debug, PartialEq)]
pub(super) enum Source {
    Display(u32),
    Window(u32),
    NamedWindow { app: String, title: Option<String> },
    Area(u32, [f64; 4]),
}

pub(super) struct Options {
    pub source: Source,
    pub duration: Option<f64>,
    pub countdown: Duration,
    pub mic: bool,
    pub system_audio: bool,
    pub camera: bool,
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
        let mic = args.flag("mic")?;
        let source = source(args)?;
        // A window or app capture usually wants its sound: record it unless told otherwise.
        let system_audio = match args.field("systemAudio") {
            Some(_) => args.flag("systemAudio")?,
            None => !still && !matches!(source, Source::Display(_) | Source::Area(..)),
        };
        let camera = args.flag("camera")?;
        let input = args.flag("events")?;
        if still && (duration.is_some() || mic || system_audio || camera || input) {
            return Err(bad(
                "still",
                "true for a single image without recording tracks or duration",
            ));
        }
        Ok(Self {
            source,
            duration,
            countdown: Duration::from_secs(args.frame("countdown")?.unwrap_or(0) as u64),
            mic,
            system_audio,
            camera,
            events: !still && (input || args.field("events").is_none()),
            still,
        })
    }
}

fn source(args: &Json) -> Result<Source, StoreError> {
    let fields = ["display", "window", "area"];
    let selectors = args.field("source");
    if let Some(selectors) = selectors
        && (fields.iter().any(|key| args.field(key).is_some())
            || !selectors
                .as_object()
                .is_some_and(|fields| fields.len() == 1))
    {
        return Err(bad("source", "one display, window or area selector"));
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
                "a window ID or an app name with an optional title from record.start dryRun",
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
        _ => Err(bad("source", "one display, window or area selector")),
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
        assert!(options.mic && options.events);
        assert!(!options.camera && !options.system_audio && !options.still);
        let window = splice_json::parse(r#"{"source":{"window":{"app":"TextEdit"}}}"#).unwrap();
        assert!(Options::parse(&window).unwrap().system_audio);
        let quiet =
            splice_json::parse(r#"{"source":{"window":{"app":"TextEdit"}},"systemAudio":false}"#)
                .unwrap();
        assert!(!Options::parse(&quiet).unwrap().system_audio);
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
        ] {
            let args = splice_json::parse(invalid).unwrap();
            assert_eq!(
                Options::parse(&args).err().unwrap().code,
                "BAD_ARGS",
                "{invalid}"
            );
        }
    }
}
