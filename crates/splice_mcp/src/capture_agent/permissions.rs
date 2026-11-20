use super::Identity;
use crate::{
    Json, object, text,
    wire::{Args, bad},
};
use apple_sys::permissions::{self as native, Permission, Status};
use splice_core::history::StoreError;

struct Grant {
    permission: Permission,
    key: &'static str,
    pane: &'static str,
}

const GRANTS: [Grant; 5] = [
    Grant {
        permission: Permission::Screen,
        key: "screen",
        pane: "Screen & System Audio Recording",
    },
    Grant {
        permission: Permission::Microphone,
        key: "microphone",
        pane: "Microphone",
    },
    Grant {
        permission: Permission::Camera,
        key: "camera",
        pane: "Camera",
    },
    Grant {
        permission: Permission::InputMonitoring,
        key: "inputMonitoring",
        pane: "Input Monitoring",
    },
    Grant {
        permission: Permission::Accessibility,
        key: "accessibility",
        pane: "Accessibility",
    },
];

impl Grant {
    fn status(&self, snapshot: &[Status; 5]) -> Status {
        snapshot[self.permission as usize]
    }
}

pub(crate) fn ask(request: bool, permission: Option<&str>) -> Result<Json, StoreError> {
    validate_target(permission)?;
    let mut args = vec![("request", Json::Bool(request))];
    if let Some(permission) = permission {
        args.push(("permission", text(permission)));
    }
    super::request("permissions", &object(args))
}

fn validate_target(target: Option<&str>) -> Result<(), StoreError> {
    if target
        .is_some_and(|target| target != "all" && !GRANTS.iter().any(|grant| grant.key == target))
    {
        return Err(bad(
            "permission",
            "all, screen, microphone, camera, inputMonitoring or accessibility",
        ));
    }
    Ok(())
}

pub(super) fn snapshot() -> [Status; 5] {
    GRANTS.map(|grant| native::status(grant.permission))
}

pub(super) fn screen_granted(snapshot: &[Status; 5]) -> bool {
    GRANTS[Permission::Screen as usize].status(snapshot) == Status::Granted
}

pub(super) fn screen_denied() -> StoreError {
    denied(&GRANTS[Permission::Screen as usize])
}

pub(super) fn accessibility_denied() -> StoreError {
    denied(&GRANTS[Permission::Accessibility as usize])
}

pub(super) fn report(identity: &Identity, snapshot: &[Status; 5]) -> Vec<(&'static str, Json)> {
    let permissions = GRANTS
        .iter()
        .map(|grant| {
            let status = match grant.status(snapshot) {
                Status::Granted => "granted",
                Status::NotGranted => "notGranted",
                Status::NotDetermined => "notDetermined",
                Status::Denied => "denied",
                Status::Restricted => "restricted",
            };
            (grant.key, text(status))
        })
        .collect();
    vec![
        ("permissions", object(permissions)),
        (
            "identity",
            object(vec![
                ("path", crate::wire::display(&identity.bundle.display())),
                ("team", text(&identity.team)),
                ("requirementMatches", Json::Bool(true)),
            ]),
        ),
    ]
}

pub(super) fn command(args: &Json, identity: &Identity) -> Result<Json, StoreError> {
    permission_reply(args, identity, &snapshot(), &mut native::request)
}

fn permission_reply(
    args: &Json,
    identity: &Identity,
    snapshot: &[Status; 5],
    request_permission: &mut dyn FnMut(Permission, bool) -> bool,
) -> Result<Json, StoreError> {
    let request = args.flag("request")?;
    let target = args.optional_string("permission")?;
    validate_target(target)?;
    if !request && target.is_some() {
        return Err(bad("permission", "a selector used with --request"));
    }
    let mut result = report(identity, snapshot);
    if !request {
        return Ok(object(result));
    }
    let all = target == Some("all");
    let mut requested = Vec::new();
    for grant in GRANTS.iter().filter(|grant| {
        (all || target.is_none_or(|target| grant.key == target))
            && grant.status(snapshot) != Status::Granted
    }) {
        if !request_permission(grant.permission, requested.is_empty()) {
            return Err(denied(grant));
        }
        if all {
            requested.push(object(vec![
                ("permission", text(grant.key)),
                ("pane", text(grant.pane)),
            ]));
            continue;
        }
        result.push(("requested", text(grant.key)));
        result.push(("pane", text(grant.pane)));
        break;
    }
    if all {
        result.push(("requested", Json::Array(requested)));
    }
    Ok(object(result))
}

pub(super) fn required(args: &Json, snapshot: &[Status; 5]) -> Result<(), StoreError> {
    require_options(&crate::record_options::Options::parse(args)?, snapshot)
}

pub(super) fn require_options(
    options: &crate::record_options::Options,
    snapshot: &[Status; 5],
) -> Result<(), StoreError> {
    for (grant, required) in GRANTS.iter().zip([
        true,
        options.mic.on(),
        options.camera.on(),
        options.events,
        options.size.is_some(),
    ]) {
        if required && grant.status(snapshot) != Status::Granted {
            return Err(denied(grant));
        }
    }
    Ok(())
}

fn denied(grant: &Grant) -> StoreError {
    StoreError::new(
        "TCC_DENIED",
        &format!("Splice needs {} permission for this capture.", grant.pane),
        &format!(
            "Run splice permissions --request {} and enable Splice in System Settings → Privacy & Security → {}.",
            grant.key, grant.pane,
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_agent_requests_all_missing_grants_and_opens_one_pane() {
        let identity = Identity {
            bundle: "Splice.app".into(),
            team: "test".into(),
        };
        let snapshot = [
            Status::NotGranted,
            Status::NotDetermined,
            Status::Granted,
            Status::NotGranted,
            Status::Granted,
        ];
        let all = object(vec![
            ("request", Json::Bool(true)),
            ("permission", text("all")),
        ]);
        let mut calls = Vec::new();
        let reply = permission_reply(&all, &identity, &snapshot, &mut |permission, settings| {
            calls.push((permission, settings));
            true
        })
        .unwrap();
        assert_eq!(
            calls,
            [
                (Permission::Screen, true),
                (Permission::Microphone, false),
                (Permission::InputMonitoring, false)
            ]
        );
        assert_eq!(
            reply
                .field("requested")
                .and_then(Json::as_array)
                .unwrap()
                .len(),
            3
        );
        let granted = permission_reply(&all, &identity, &[Status::Granted; 5], &mut |_, _| {
            panic!("already granted")
        })
        .unwrap();
        assert!(
            granted
                .field("requested")
                .and_then(Json::as_array)
                .unwrap()
                .is_empty()
        );

        let next = object(vec![("request", Json::Bool(true))]);
        let reply = permission_reply(&next, &identity, &snapshot, &mut |permission, settings| {
            assert_eq!((permission, settings), (Permission::Screen, true));
            true
        })
        .unwrap();
        assert_eq!(
            reply.field("requested").and_then(Json::as_str),
            Some("screen")
        );
    }

    #[test]
    fn capture_agent_permissions_require_only_the_selected_inputs() {
        let snapshot = [
            Status::Granted,
            Status::Denied,
            Status::NotDetermined,
            Status::NotGranted,
            Status::NotGranted,
        ];
        assert!(required(&object(vec![("events", Json::Bool(false))]), &snapshot).is_ok());
        assert!(required(&object(vec![("still", Json::Bool(true))]), &snapshot).is_ok());
        for (args, pane) in [
            (object(vec![("mic", Json::Bool(true))]), "Microphone"),
            (object(vec![("camera", Json::Bool(true))]), "Camera"),
            (object(Vec::new()), "Input Monitoring"),
            (
                splice_json::parse(
                    r#"{"events":false,"source":{"window":{"app":"TextEdit"}},"size":[1280,720]}"#,
                )
                .unwrap(),
                "Accessibility",
            ),
        ] {
            let error = required(&args, &snapshot).unwrap_err();
            assert_eq!(error.code, "TCC_DENIED");
            assert!(error.fix.contains(pane));
            assert!(error.fix.contains("splice permissions --request"));
        }
        let error = required(&object(Vec::new()), &[Status::NotGranted; 5]).unwrap_err();
        assert_eq!(error.code, "TCC_DENIED");
        assert!(error.fix.contains("Screen & System Audio Recording"));
    }
}
