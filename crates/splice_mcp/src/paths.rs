#[cfg(any(target_os = "macos", test))]
use std::path::Path;
use std::{env, io, path::PathBuf};

/// Finds the per-user directory for persistent state and local sockets.
pub fn state_dir() -> io::Result<PathBuf> {
    resolve_state_dir(
        env::var_os("HOME").map(PathBuf::from),
        env::var_os("XDG_STATE_HOME").map(PathBuf::from),
    )
}

fn resolve_state_dir(home: Option<PathBuf>, xdg_state: Option<PathBuf>) -> io::Result<PathBuf> {
    let suffix = if cfg!(target_os = "macos") {
        "Library/Application Support/Splice"
    } else if let Some(state) = xdg_state.filter(|path| path.is_absolute()) {
        return Ok(state.join("splice"));
    } else {
        ".local/state/splice"
    };
    home.filter(|path| path.is_absolute())
        .map(|home| home.join(suffix))
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "HOME does not name an absolute path",
            )
        })
}

/// Finds the running application's bundle, including when launched through a symlink.
/// An executable outside a bundle returns `None`.
#[cfg(target_os = "macos")]
pub fn app_bundle() -> io::Result<Option<PathBuf>> {
    // On macOS, current_exe uses _NSGetExecutablePath. Canonicalizing resolves its symlinks.
    bundle_from_executable(&env::current_exe()?)
}

/// Finds the packaged resources for the running macOS application.
#[cfg(target_os = "macos")]
pub fn resources_dir() -> io::Result<Option<PathBuf>> {
    Ok(app_bundle()?.map(|bundle| bundle.join("Contents/Resources")))
}

#[cfg(any(target_os = "macos", test))]
pub(super) fn bundle_from_executable(executable: &Path) -> io::Result<Option<PathBuf>> {
    Ok(executable
        .canonicalize()?
        .parent()
        .filter(|directory| directory.ends_with("Contents/MacOS"))
        .and_then(Path::parent)
        .and_then(Path::parent)
        .filter(|bundle| bundle.extension().is_some_and(|ext| ext == "app"))
        .map(Path::to_path_buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_directory_uses_the_platform_location() {
        let home = PathBuf::from("/example HOME");
        let xdg = PathBuf::from("/another/state");
        let expected = if cfg!(target_os = "macos") {
            home.join("Library/Application Support/Splice")
        } else {
            xdg.join("splice")
        };
        assert_eq!(
            resolve_state_dir(Some(home.clone()), Some(xdg)).unwrap(),
            expected,
        );
        let expected = if cfg!(target_os = "macos") {
            home.join("Library/Application Support/Splice")
        } else {
            home.join(".local/state/splice")
        };
        assert_eq!(
            resolve_state_dir(Some(home), Some(PathBuf::from("relative"))).unwrap(),
            expected,
        );
        assert_eq!(
            resolve_state_dir(Some(PathBuf::from("relative")), None)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound,
        );
    }
}
