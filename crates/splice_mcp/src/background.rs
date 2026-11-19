//! Long native work off the request loop (PLAN.md T15.3): analysis and transcription of long
//! media run on a worker thread; `jobs.status` reports them and, for transcription, commits the
//! words on the main thread when the worker is done.
use super::{Json, object, project::Projects, text, wire::*};
use splice_core::history::{Store, StoreError};
use splice_model::JobId;
use std::{thread::JoinHandle, time::Instant};

/// Media longer than this (seconds) analyze and transcribe as jobs.
pub(super) const LONG_SECONDS: f64 = 120.0;

/// Runs on the main thread with the worker's output, e.g. to commit an edit.
pub(super) type Finish = Box<dyn FnOnce(&mut Store, Json) -> Result<Json, StoreError>>;

pub(super) struct Background {
    pub id: JobId,
    kind: &'static str,
    project: String,
    worker: Option<JoinHandle<Result<Json, StoreError>>>,
    finish: Option<Finish>,
    result: Option<Result<Json, StoreError>>,
    started: Instant,
    cancelled: bool,
}

impl Background {
    pub(super) fn spawn(
        projects: &mut Projects,
        kind: &'static str,
        project: &str,
        work: Box<dyn FnOnce() -> Result<Json, StoreError> + Send>,
        finish: Option<Finish>,
    ) -> Json {
        let id = JobId::from_serial(1000 + projects.background.len() as u64);
        projects.background.push(Self {
            id,
            kind,
            project: project.to_owned(),
            worker: Some(std::thread::spawn(work)),
            finish,
            result: None,
            started: Instant::now(),
            cancelled: false,
        });
        object(vec![
            ("jobId", display(&id)),
            ("kind", text(kind)),
            ("state", text("running")),
        ])
    }

    /// Collects a finished worker and runs its finish step.
    fn poll(&mut self, store: Option<&mut Store>) {
        if self.result.is_some() || !self.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            return;
        }
        let output = self
            .worker
            .take()
            .expect("running worker")
            .join()
            .unwrap_or_else(|_| {
                Err(failure(
                    "NATIVE",
                    "The worker stopped.",
                    "Run the job again.",
                ))
            });
        self.result = Some(match (output, self.finish.take(), store) {
            (Ok(json), Some(finish), Some(store)) if !self.cancelled => finish(store, json),
            (Ok(_), Some(_), None) => Err(failure(
                "NOT_FOUND",
                "The project closed before the job finished.",
                "Open the project and run the job again.",
            )),
            (other, _, _) => other,
        });
    }
}

/// `jobs.status` / `jobs.cancel` for a background job, if `id` is one.
pub(super) fn job(
    projects: &mut Projects,
    id: JobId,
    cancel: bool,
) -> Option<Result<Json, StoreError>> {
    let index = projects.background.iter().position(|job| job.id == id)?;
    if cancel && projects.background[index].result.is_none() {
        // The native call can't be interrupted; its result is dropped when it lands.
        projects.background[index].cancelled = true;
    }
    if projects.background[index]
        .worker
        .as_ref()
        .is_some_and(JoinHandle::is_finished)
    {
        // The finish step needs the project's store, a sibling of the job list.
        let mut taken = projects.background.remove(index);
        let store = projects.lookup_id(&taken.project).ok();
        taken.poll(store);
        projects.background.insert(index, taken);
    }
    let job = &projects.background[index];
    Some(match (&job.result, job.cancelled) {
        (_, true) => Ok(object(vec![
            ("jobId", display(&job.id)),
            ("kind", text(job.kind)),
            ("state", text("cancelled")),
        ])),
        (None, false) => Ok(object(vec![
            ("jobId", display(&job.id)),
            ("kind", text(job.kind)),
            ("state", text("running")),
            ("elapsed", Json::Float(job.started.elapsed().as_secs_f64())),
        ])),
        (Some(Ok(result)), false) => {
            let mut fields = vec![
                ("jobId".to_owned(), display(&job.id)),
                ("kind".to_owned(), text(job.kind)),
                ("state".to_owned(), text("done")),
            ];
            if let Json::Object(more) = result {
                fields.extend(more.iter().filter(|(k, _)| k != "state").cloned());
            }
            Ok(Json::Object(fields))
        }
        (Some(Err(error)), false) => Err(failure(error.code, &error.message, &error.fix)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_jobs_report_running_then_their_result_and_cancel_drops_it() {
        let mut projects = Projects::default();
        let started = Background::spawn(
            &mut projects,
            "analyze",
            "prj_1",
            Box::new(|| {
                std::thread::sleep(std::time::Duration::from_millis(60));
                Ok(object(vec![("beats", Json::Array(vec![]))]))
            }),
            None,
        );
        let id: JobId = started.string("jobId").unwrap().parse().unwrap();
        let running = job(&mut projects, id, false).unwrap().unwrap();
        assert_eq!(running.string("state").unwrap(), "running");
        std::thread::sleep(std::time::Duration::from_millis(150));
        let done = job(&mut projects, id, false).unwrap().unwrap();
        assert_eq!(done.string("state").unwrap(), "done");
        assert!(done.field("beats").is_some());
        let second = Background::spawn(
            &mut projects,
            "transcribe",
            "prj_1",
            Box::new(|| Ok(Json::Null)),
            None,
        );
        let id: JobId = second.string("jobId").unwrap().parse().unwrap();
        let cancelled = job(&mut projects, id, true).unwrap().unwrap();
        assert_eq!(cancelled.string("state").unwrap(), "cancelled");
        assert!(job(&mut projects, JobId::from_serial(9), false).is_none());
    }
}
