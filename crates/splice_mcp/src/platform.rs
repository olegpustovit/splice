use super::{
    Json,
    project::{Projects, package_path},
    wire::*,
};
use splice_core::history::StoreError;
use splice_model::{Frame, JobId};
use std::path::Path;

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

pub(super) fn export_options(preset: Option<&str>, path: Option<&str>) -> Result<bool, StoreError> {
    let hevc = match preset {
        None | Some("social-hevc") => true,
        Some("social-h264") => false,
        _ => return Err(bad("preset", "social-h264 or social-hevc")),
    };
    if path.is_some_and(|path| {
        path.contains('\0')
            || !Path::new(path)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4"))
    }) {
        return Err(bad("path", "an output path ending in .mp4"));
    }
    Ok(hevc)
}

pub(super) fn export(projects: &mut Projects, args: &Json) -> Result<Json, StoreError> {
    let owner = args.string("projectId")?;
    let store = projects.lookup_id(owner)?;
    let path = args.optional_string("path")?;
    let hevc = export_options(args.optional_string("preset")?, path)?;
    let overwrite = args.flag("overwrite")?;
    let options = write_options(args)?;
    if options.dry_run {
        return Err(bad("dryRun", "false or omitted for export.start"));
    }
    store.check_write(&options)?;
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (hevc, overwrite);
        Err(unavailable("Export"))
    }
    #[cfg(target_os = "macos")]
    {
        if projects.exports.iter().any(super::export::Job::active) {
            return Err(failure(
                "BUSY",
                "An export is already running.",
                "Check jobs.status or cancel the active export.",
            ));
        }
        let id = JobId::from_serial(projects.exports.len() as u64 + 1);
        let store = projects.lookup_id(owner)?;
        let job = super::export::Job::start(
            store,
            id,
            Some((owner.to_owned(), options.session.expect("agent session"))),
            hevc,
            path,
            overwrite,
        )?;
        let receipt = job.started();
        projects.exports.push(job);
        Ok(receipt)
    }
}

pub(super) fn job(projects: &mut Projects, name: &str, args: &Json) -> Result<Json, StoreError> {
    let id = job_id(args)?;
    let project = args.optional_string("projectId")?;
    let cancel = name == "jobs.cancel";
    if cancel && args.flag("dryRun")? {
        return Err(bad("dryRun", "false or omitted for jobs.cancel"));
    }
    if let Some(project) = project {
        let store = projects.lookup_id(project)?;
        if cancel {
            store.check_write(&write_options(args)?)?;
        }
    }
    #[cfg(target_os = "macos")]
    if let Some(index) = projects.exports.iter().position(|job| job.id == id) {
        let (owner, session) = projects.exports[index]
            .owner
            .clone()
            .expect("MCP export owner");
        if project.is_some_and(|project| project != owner) {
            return Err(missing_job(id));
        }
        if project.is_none() {
            let store = projects.lookup_id(&owner)?;
            if cancel {
                let options =
                    if args.field("sessionId").is_some() || args.field("ifRevision").is_some() {
                        write_options(args)?
                    } else {
                        splice_core::history::WriteOptions {
                            source: splice_core::history::Source::Agent,
                            session: Some(session),
                            if_revision: Some(store.project().rev),
                            ..Default::default()
                        }
                    };
                store.check_write(&options)?;
            }
        }
        let job = &projects.exports[index];
        if cancel {
            job.cancel();
        }
        return job.status();
    }
    Err(missing_job(id))
}

fn missing_job(id: JobId) -> StoreError {
    failure(
        "NOT_FOUND",
        &format!("No job {id} is running or retained."),
        "Pass a jobId returned by export.start or record.start in this server.",
    )
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
