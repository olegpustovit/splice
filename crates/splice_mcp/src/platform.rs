use super::{
    Json,
    project::{Projects, package_path},
    wire::*,
};
use splice_core::history::{Store, StoreError};
use splice_model::{Frame, JobId};

pub(super) fn frames(args: &Json) -> Result<Json, StoreError> {
    let filmstrip = args.field("count");
    let explicit = args.field("at");
    match (filmstrip, explicit) {
        (Some(count), None) if count.as_u64().is_some_and(|n| n > 0) => {}
        (None, Some(frames))
            if frames.as_array().is_some_and(|items| {
                !items.is_empty()
                    && items
                        .iter()
                        .all(|at| at.as_u64().and_then(|n| Frame::try_from(n).ok()).is_some())
            }) => {}
        _ => {
            return Err(bad(
                "at or count",
                "an array of nonnegative frames or a positive filmstrip count",
            ));
        }
    }
    Err(unavailable("Frame rendering"))
}

pub(super) fn export(store: &Store, args: &Json) -> Result<Json, StoreError> {
    if args
        .optional_string("preset")?
        .is_some_and(|preset| !matches!(preset, "social-h264" | "social-hevc"))
    {
        return Err(bad("preset", "social-h264 or social-hevc"));
    }
    if args
        .optional_string("path")?
        .is_some_and(|path| path.is_empty() || path.contains('\0'))
    {
        return Err(bad("path", "a nonempty output path"));
    }
    args.flag("overwrite")?;
    store.check_write(&write_options(args)?)?;
    Err(unavailable("Export"))
}

pub(super) fn job(projects: &mut Projects, name: &str, args: &Json) -> Result<Json, StoreError> {
    let id = job_id(args)?;
    if args.field("projectId").is_some() {
        let store = projects.lookup(args)?;
        if name == "jobs.cancel" {
            store.check_write(&write_options(args)?)?;
        }
    }
    // ponytail: the headless build has no native jobs until export (T4.4) or recording (T8.4) starts one.
    Err(failure(
        "NOT_FOUND",
        &format!("No job {id} is running or retained."),
        "Pass a jobId returned by export.start or record.start in this server.",
    ))
}

pub(super) fn record_start(projects: &mut Projects, args: &Json) -> Result<Json, StoreError> {
    let path = args.optional_string("path")?;
    let dry = args.flag("dryRun")?;
    let create = args.flag("create")?;
    if args.field("projectId").is_some() {
        if path.is_some() || create {
            return Err(bad(
                "projectId",
                "the only project selector, or use path and create",
            ));
        }
        let store = projects.lookup(args)?;
        if !dry {
            store.check_write(&write_options(args)?)?;
        }
    } else if let Some(path) = path {
        package_path(path)?;
    } else if create || !dry {
        return Err(bad(
            "projectId or path",
            "an open project ID or a .splice path",
        ));
    }
    Err(unavailable("Recording"))
}

pub(super) fn record_stop(projects: &mut Projects, args: &Json) -> Result<Json, StoreError> {
    job_id(args)?;
    args.flag("insert")?;
    if args.field("projectId").is_some() {
        projects.lookup(args)?.check_write(&write_options(args)?)?;
    }
    Err(unavailable("Recording"))
}

fn job_id(args: &Json) -> Result<JobId, StoreError> {
    args.string("jobId")?
        .parse()
        .map_err(|_| bad("jobId", "a job ID returned by export.start or record.start"))
}

fn unavailable(feature: &str) -> StoreError {
    // ponytail: replace this boundary with AVFoundation in T4 and the signed capture agent in T8.
    failure(
        "UNSUPPORTED_PLATFORM",
        &format!("{feature} requires the native macOS backend."),
        "Use a macOS build with the native media and capture backend.",
    )
}
