unsafe extern "C" {
    fn splice_permission_status(permission: u32) -> u32;
    fn splice_permission_request(permission: u32) -> bool;
}

/// One macOS authorization used by screen recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Permission {
    /// Screen and system audio recording.
    Screen,
    /// Microphone recording.
    Microphone,
    /// Camera recording.
    Camera,
    /// Listen-only input monitoring.
    InputMonitoring,
}

/// The authorization state reported by the system.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The API reports access is missing without distinguishing the underlying reason.
    NotGranted,
    /// The required access is available.
    Granted,
    /// The person has yet to make a decision.
    NotDetermined,
    /// Access was denied.
    Denied,
    /// System policy restricts access.
    Restricted,
}

/// Reads a permission without presenting a prompt or acquiring a capture device.
pub fn status(permission: Permission) -> Status {
    // SAFETY: the bridge takes a by-value discriminator.
    match unsafe { splice_permission_status(permission as u32) } {
        1 => Status::Granted,
        2 => Status::NotDetermined,
        3 => Status::Denied,
        4 => Status::Restricted,
        _ => Status::NotGranted,
    }
}

/// Starts an explicit permission request and opens the matching privacy pane.
/// It must be called on the application thread.
pub fn request(permission: Permission) -> bool {
    // SAFETY: the bridge validates the discriminator and checks the thread before touching AppKit.
    unsafe { splice_permission_request(permission as u32) }
}
