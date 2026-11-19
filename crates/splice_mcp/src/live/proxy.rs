use super::{disconnected, request};
use crate::{
    Json, object,
    project::{Projects, package_path},
    text,
    wire::*,
};
use splice_core::history::StoreError;
use splice_model::JobId;
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct Routes {
    jobs: Vec<JobRoute>,
}

struct JobRoute {
    inner: String,
    project: Option<String>,
    remote: Option<(PathBuf, String)>,
}

impl Routes {
    pub(crate) fn tool_args(&self, name: &str, args: &Json) -> Result<Json, StoreError> {
        let mut inner = args.clone();
        if let Some(job) = self.job(name, args) {
            if args
                .optional_string("projectId")?
                .is_some_and(|id| job.project.as_deref().is_some_and(|owner| owner != id))
            {
                return Err(failure(
                    "NOT_FOUND",
                    "This job belongs to another project.",
                    "Use the project that started the job.",
                ));
            }
            replace(&mut inner, "jobId", text(&job.inner));
        }
        Ok(inner)
    }

    pub(crate) fn forward(
        &mut self,
        projects: &mut Projects,
        name: &str,
        args: &Json,
        routed: &Json,
        client: String,
    ) -> Result<Option<Json>, StoreError> {
        let open = match name {
            "project.info" | "record.start" if args.field("projectId").is_none() => args
                .field("path")
                .and_then(Json::as_str)
                .and_then(|path| package_path(path).ok())
                .and_then(|path| path.canonicalize().ok())
                .filter(|path| !projects.is_local(path)),
            _ => None,
        };
        let project = args.field("projectId").and_then(Json::as_str);
        let remote = self
            .job(name, args)
            .and_then(|job| {
                job.remote
                    .as_ref()
                    .map(|(path, owner)| (path.as_path(), owner.as_str()))
            })
            .or_else(|| project.and_then(|project| projects.remote(project)));
        let Some((path, expected)) = open
            .as_deref()
            .map(|path| (path, None))
            .or_else(|| remote.map(|(path, owner)| (path, Some(owner))))
        else {
            return Ok(None);
        };
        let path = path.to_path_buf();
        // A package handed off to a viewer (T10.1) has no owner yet; learn it from this reply.
        let handed_off = expected == Some("");
        let expected = expected.filter(|owner| !owner.is_empty());
        let mut arguments = routed.clone();
        if project.is_some() {
            replace(&mut arguments, "projectId", text("prj_1"));
        }
        if open.is_some() {
            // The caller's relative path is resolved on the stdio side.
            replace(&mut arguments, "path", display(&path.display()));
        }
        let mut body = object(vec![
            ("command", text("tool")),
            ("name", text(name)),
            ("args", arguments),
            ("client", Json::String(client)),
        ]);
        if let Some(expected) = expected {
            replace(&mut body, "owner", text(expected));
        }
        let Some(reply) = request(&path, &body).map_err(|error| disconnected(&error))? else {
            return if expected.is_some() || handed_off {
                Err(disconnected(&"The project viewer is no longer listening."))
            } else {
                Ok(None)
            };
        };
        let owner = reply
            .field("owner")
            .and_then(Json::as_str)
            .ok_or_else(|| {
                let message = reply
                    .field("error")
                    .and_then(|error| error.field("message"))
                    .and_then(Json::as_str)
                    .unwrap_or("The viewer sent an invalid reply.");
                disconnected(&message)
            })?
            .to_owned();
        let mut result = reply
            .field("result")
            .ok_or_else(|| disconnected(&"The viewer result is missing."))?
            .clone();
        if handed_off {
            projects.remember_remote(path.clone(), owner.clone());
        }
        if matches!(name, "project.info" | "record.start")
            && result.field("isError").and_then(Json::as_bool) == Some(false)
            && result
                .field("structuredContent")
                .and_then(|payload| payload.field("projectId"))
                .is_some()
        {
            let id = projects.remember_remote(path.clone(), owner.clone());
            rewrite(&mut result, "projectId", text(&id));
        }
        self.remap_job(name, args, &mut result, Some((path, owner)));
        Ok(Some(result))
    }

    pub(crate) fn remap_job(
        &mut self,
        name: &str,
        args: &Json,
        reply: &mut Json,
        remote: Option<(PathBuf, String)>,
    ) {
        let Some(payload) = reply.field("structuredContent") else {
            return;
        };
        let Some(job) = payload.field("jobId").and_then(Json::as_str) else {
            return;
        };
        let id = if matches!(name, "export.start" | "record.start") {
            self.jobs.push(JobRoute {
                inner: job.into(),
                project: args
                    .field("projectId")
                    .or_else(|| payload.field("projectId"))
                    .and_then(Json::as_str)
                    .map(str::to_owned),
                remote,
            });
            JobId::from_serial(self.jobs.len() as u64).to_string()
        } else if self.job(name, args).is_some() {
            args.field("jobId")
                .and_then(Json::as_str)
                .expect("known job")
                .into()
        } else {
            return;
        };
        rewrite(reply, "jobId", text(&id));
    }

    fn job(&self, name: &str, args: &Json) -> Option<&JobRoute> {
        if !matches!(name, "jobs.status" | "jobs.cancel" | "record.stop") {
            return None;
        }
        let id = args.field("jobId")?.as_str()?.parse::<JobId>().ok()?;
        self.jobs
            .get(usize::try_from(id.serial().checked_sub(1)?).ok()?)
    }
}

fn replace(object: &mut Json, name: &str, value: Json) {
    let Json::Object(fields) = object else {
        return;
    };
    if let Some((_, existing)) = fields.iter_mut().find(|(key, _)| key == name) {
        *existing = value;
    } else {
        fields.push((name.into(), value));
    }
}

fn rewrite(reply: &mut Json, name: &str, value: Json) {
    let Json::Object(fields) = reply else {
        return;
    };
    let Some((_, payload)) = fields
        .iter_mut()
        .find(|(key, _)| key == "structuredContent")
    else {
        return;
    };
    replace(payload, name, value);
    let mut mirror = String::new();
    if splice_json::write(payload, &mut mirror).is_ok()
        && let Some((_, Json::Array(content))) = fields.iter_mut().find(|(key, _)| key == "content")
        && let Some(first) = content.first_mut()
    {
        replace(first, "text", Json::String(mirror));
    }
}
