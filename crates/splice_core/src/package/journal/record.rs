use crate::package::{PackageError, codec::*, json_bytes};
use splice_json::Value as Json;
use splice_model::SessionId;

/// Actor whose batch is committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A human client.
    Human,
    /// An MCP editing session.
    Agent,
}

/// One committed edit or history action.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// A batch of commands and the IDs minted while applying them.
    Edit {
        /// Original commands in submission order.
        ops: Vec<Json>,
        /// Minted IDs in replay order.
        mints: Vec<String>,
    },
    /// Rebuild while omitting specified revisions.
    Revert {
        /// Revisions that were reverted.
        revisions: Vec<u64>,
        /// Recorded replay failures.
        skipped: Vec<Json>,
    },
    /// Copy the state at a revision into a new revision.
    Restore(u64),
}

/// Metadata and content submitted for one journal append.
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    /// Unix time in milliseconds.
    pub time: u64,
    /// Origin of the change.
    pub source: Source,
    /// Originating session, when present.
    pub session: Option<SessionId>,
    /// Human-readable summary.
    pub label: String,
    /// Optional idempotency key.
    pub key: Option<String>,
    /// Replayable changes.
    pub change: Change,
}

/// One verified record in the revision chain.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    /// Revision numbered sequentially from the origin.
    pub rev: u64,
    /// Hash of the preceding revision or origin.
    pub parent: u64,
    /// FNV-1a of the canonical record with this field omitted.
    pub hash: u64,
    /// Batch recorded at this revision.
    pub batch: Batch,
}

impl Record {
    pub(super) fn encode(&self, include_hash: bool) -> Json {
        let batch = &self.batch;
        let mut fields = vec![
            ("rev", Json::Unsigned(self.rev)),
            ("parent", hash_text(self.parent)),
        ];
        if include_hash {
            fields.push(("hash", hash_text(self.hash)));
        }
        fields.extend([
            ("time", Json::Unsigned(batch.time)),
            (
                "source",
                text(match batch.source {
                    Source::Human => "human",
                    Source::Agent => "agent",
                }),
            ),
        ]);
        if let Some(session) = batch.session {
            fields.push(("session", display(&session)));
        }
        fields.push(("label", text(&batch.label)));
        push_text(&mut fields, "key", &batch.key);
        match &batch.change {
            Change::Edit { ops, mints } => {
                fields.push(("ops", Json::Array(ops.clone())));
                fields.push((
                    "mints",
                    Json::Array(mints.iter().map(|mint| text(mint)).collect()),
                ));
            }
            Change::Revert { revisions, skipped } => {
                fields.push((
                    "revertOf",
                    Json::Array(revisions.iter().copied().map(Json::Unsigned).collect()),
                ));
                fields.push(("skipped", Json::Array(skipped.clone())));
            }
            Change::Restore(rev) => fields.push(("restoreTo", Json::Unsigned(*rev))),
        }
        object(fields)
    }

    pub(super) fn decode(value: &Json) -> Result<Self, PackageError> {
        let shared = [
            "rev", "parent", "hash", "time", "source", "session", "label", "key",
        ];
        let mut fields = shared.to_vec();
        let change = if value.field("ops").is_some() {
            fields.extend(["ops", "mints"]);
            let mut mints = Vec::new();
            for mint in value.array("mints")? {
                mints.push(
                    mint.as_str()
                        .ok_or_else(|| bad("mints", "an array of ID strings"))?
                        .to_owned(),
                );
            }
            Change::Edit {
                ops: value.array("ops")?.to_vec(),
                mints,
            }
        } else if value.field("revertOf").is_some() {
            fields.extend(["revertOf", "skipped"]);
            let mut revisions = Vec::new();
            for revision in value.array("revertOf")? {
                revisions.push(
                    revision
                        .as_u64()
                        .ok_or_else(|| bad("revertOf", "an array of revisions"))?,
                );
            }
            Change::Revert {
                revisions,
                skipped: value.array("skipped")?.to_vec(),
            }
        } else {
            fields.push("restoreTo");
            Change::Restore(u64_field(value, "restoreTo")?)
        };
        shape(value, &fields)?;
        let record = Self {
            rev: u64_field(value, "rev")?,
            parent: hash(value, "parent")?,
            hash: hash(value, "hash")?,
            batch: Batch {
                time: u64_field(value, "time")?,
                source: match value.string("source")? {
                    "human" => Source::Human,
                    "agent" => Source::Agent,
                    _ => return Err(bad("source", "human or agent")),
                },
                session: optional_string(value, "session")?
                    .map(|value| value.parse().map_err(|_| bad_id("session")))
                    .transpose()?,
                label: value.string("label")?.to_owned(),
                key: optional_string(value, "key")?,
                change,
            },
        };
        record.validate()?;
        Ok(record)
    }

    pub(super) fn validate(&self) -> Result<(), PackageError> {
        match &self.batch.change {
            Change::Edit { ops, .. } => {
                if ops
                    .iter()
                    .any(|op| op.field("cmd").and_then(Json::as_str).is_none())
                {
                    return Err(bad("ops", "an array of commands"));
                }
            }
            Change::Revert { revisions, .. } => {
                if revisions.is_empty() || revisions.iter().any(|&rev| rev >= self.rev) {
                    return Err(bad("revertOf", "one or more earlier revisions"));
                }
            }
            Change::Restore(rev) => {
                if *rev >= self.rev {
                    return Err(bad("restoreTo", "an earlier revision"));
                }
            }
        }
        Ok(())
    }

    pub(super) fn compute_hash(&self) -> Result<u64, PackageError> {
        let bytes = json_bytes(&self.encode(false))?;
        Ok(bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
        }))
    }
}
