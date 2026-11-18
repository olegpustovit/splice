use crate::package::{PackageError, atomic_write, codec::*, json_bytes, read_optional_json};
use splice_json::Value as Json;
use splice_model::SessionId;
use std::path::Path;

const FILE: &str = "sessions.json";

/// An agent session and its initial project revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// Project-local session ID.
    pub id: SessionId,
    /// Client-supplied agent name.
    pub agent: String,
    /// Revision active when the session opened.
    pub checkpoint: u64,
    /// Unix time in milliseconds.
    pub started: u64,
    /// Whether the session is allowed to write.
    pub open: bool,
}

pub(super) fn read(path: &Path) -> Result<Vec<Session>, PackageError> {
    let file = path.join(FILE);
    let Some(value) = read_optional_json(&file)? else {
        return Ok(Vec::new());
    };
    decode(&value).map_err(|error| error.at(&file))
}

fn decode(value: &Json) -> Result<Vec<Session>, PackageError> {
    check_version(value)?;
    shape(value, &["format", "sessions"])?;
    let mut sessions: Vec<Session> = Vec::new();
    for item in value.array("sessions")? {
        shape(item, &["id", "agent", "checkpoint", "started", "open"])?;
        let id = item
            .string("id")?
            .parse()
            .map_err(|_| bad_id("session.id"))?;
        if sessions.iter().any(|session| session.id == id) {
            return Err(bad("sessions", "unique session IDs"));
        }
        sessions.push(Session {
            id,
            agent: item.string("agent")?.into(),
            checkpoint: u64_field(item, "checkpoint")?,
            started: u64_field(item, "started")?,
            open: boolean(item, "open")?,
        });
    }
    Ok(sessions)
}

pub(super) fn write(path: &Path, sessions: &[Session]) -> Result<(), PackageError> {
    let sessions = sessions
        .iter()
        .map(|session| {
            object(vec![
                ("id", display(&session.id)),
                ("agent", text(&session.agent)),
                ("checkpoint", Json::Unsigned(session.checkpoint)),
                ("started", Json::Unsigned(session.started)),
                ("open", Json::Bool(session.open)),
            ])
        })
        .collect();
    let value = object(vec![
        ("format", text(FORMAT)),
        ("sessions", Json::Array(sessions)),
    ]);
    atomic_write(path, FILE, &json_bytes(&value)?)
}
