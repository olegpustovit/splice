//! Loads a `.cube` LUT into the compositor's atlas layout.
use crate::Error;
use std::path::Path;

/// RGBA8 atlas pixels (`n² × n`) and the cube size.
pub(crate) fn atlas(path: &Path) -> Result<(Vec<u8>, u32), Error> {
    let text = std::fs::read_to_string(path).map_err(|_| Error::Offline(path.to_path_buf()))?;
    let (size, entries) = splice_eval::parse_cube(&text)
        .ok_or_else(|| Error::Input(format!("{} is not a 3D .cube LUT.", path.display())))?;
    Ok((splice_eval::cube_atlas(size, &entries), size))
}
