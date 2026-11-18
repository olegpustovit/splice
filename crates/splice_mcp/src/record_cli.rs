use crate::{
    Json, error_payload, object,
    wire::{Args, failure},
};
use std::{fmt, io, thread, time::Duration};

pub(super) fn run(
    args: &Json,
    stop: &mut dyn FnMut() -> io::Result<bool>,
    tool: &mut dyn FnMut(&str, &Json) -> Result<Json, Json>,
) -> Result<Json, Json> {
    let dry = args.flag("dryRun").map_err(error_payload)?;
    let automatic =
        args.field("duration").is_some() || args.flag("still").map_err(error_payload)?;
    let started = tool("record.start", args)?;
    if dry {
        return Ok(started);
    }
    let job = object(vec![(
        "jobId",
        Json::String(started.string("jobId").map_err(error_payload)?.into()),
    )]);
    let result = wait_and_publish(started, &job, automatic, stop, tool);
    if result.is_err() {
        let _ = tool("jobs.cancel", &job);
    }
    result
}

fn wait_and_publish(
    mut status: Json,
    job: &Json,
    automatic: bool,
    stop: &mut dyn FnMut() -> io::Result<bool>,
    tool: &mut dyn FnMut(&str, &Json) -> Result<Json, Json>,
) -> Result<Json, Json> {
    loop {
        match status.string("state").map_err(error_payload)? {
            "ready" => break,
            "recording" if !automatic => {
                if !stop().map_err(|error| io_error(&error))? {
                    return Err(cancelled());
                }
                break;
            }
            "pending" | "recording" => (),
            "cancelled" => return Err(cancelled()),
            state => return Err(io_error(&format!("Unexpected recording state {state}."))),
        }
        thread::sleep(Duration::from_millis(40));
        status = tool("jobs.status", job)?;
    }
    let sealed = tool("record.stop", job)?;
    match sealed.string("state").map_err(error_payload)? {
        "done" => Ok(sealed),
        "cancelled" => Err(cancelled()),
        state => Err(io_error(&format!("Capture stopped in state {state}."))),
    }
}

pub(super) fn io_error(error: &dyn fmt::Display) -> Json {
    error_payload(failure(
        "IO",
        &error.to_string(),
        "Check the project and capture agent, then retry.",
    ))
}

fn cancelled() -> Json {
    error_payload(failure(
        "CAPTURE_CANCELLED",
        "The recording was cancelled.",
        "Run splice record to start another recording.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text;

    #[test]
    fn timed_recording_waits_for_ready_and_publishes_through_mcp() {
        let args = object(vec![
            ("path", text("video.splice")),
            ("duration", Json::Unsigned(3)),
        ]);
        let mut calls = ["record.start", "jobs.status", "record.stop"].into_iter();
        let result = run(
            &args,
            &mut || panic!("timed recording asked for input"),
            &mut |method, arguments| {
                assert_eq!(calls.next(), Some(method));
                assert_eq!(
                    if method == "record.start" {
                        arguments.string("path").unwrap()
                    } else {
                        arguments.string("jobId").unwrap()
                    },
                    if method == "record.start" {
                        "video.splice"
                    } else {
                        "job_1"
                    }
                );
                let state = match method {
                    "record.start" => "recording",
                    "jobs.status" => "ready",
                    _ => "done",
                };
                Ok(object(vec![
                    ("jobId", text("job_1")),
                    ("state", text(state)),
                ]))
            },
        )
        .unwrap();
        assert_eq!(result.string("state").unwrap(), "done");
        assert_eq!(calls.next(), None);
    }

    #[test]
    fn manual_recording_discards_the_take_on_end_of_input() {
        let args = object(vec![("path", text("video.splice"))]);
        let mut calls = ["record.start", "jobs.cancel"].into_iter();
        let error = run(&args, &mut || Ok(false), &mut |method, _| {
            assert_eq!(calls.next(), Some(method));
            Ok(object(vec![
                ("jobId", text("job_1")),
                ("state", text("recording")),
            ]))
        })
        .unwrap_err();
        assert_eq!(error.string("code").unwrap(), "CAPTURE_CANCELLED");
        assert_eq!(calls.next(), None);
    }
}
