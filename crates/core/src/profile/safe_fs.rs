//! Conservative filesystem operations for credential-bearing profile storage.
use std::fs;
use std::path::Path;

use crate::error::{CswError, Result};

pub(crate) fn reject_symlink(path: &Path) -> Result<()> {
    if path.is_symlink() {
        return Err(CswError::Other(format!(
            "Refused symlinked profile storage: {}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) fn private_dir(path: &Path) -> Result<()> {
    reject_symlink(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700).create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path)?;
    Ok(())
}

/// Skip symbolic links and special files at every depth. In particular, never
/// materialize an external file's contents in a duplicated account or copy.
pub(crate) fn copy_directory(src: &Path, dst: &Path) -> Result<()> {
    reject_symlink(src)?;
    private_dir(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let target = dst.join(entry.file_name());
        if ty.is_symlink() {
            continue;
        }
        reject_symlink(&target)?;
        if ty.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if ty.is_file() {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}
