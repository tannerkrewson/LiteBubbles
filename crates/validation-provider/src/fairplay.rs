//! User-local FairPlay material preparation.
//!
//! The public OpenBubbles build module documents this exact extraction boundary
//! for the user-supplied x86_64 `openbubbles.so`.  This module keeps the
//! resulting certificate/key pairs outside the repository and validates every
//! pair before writing it.  It never prints or returns private-key contents.

use openssl::{rsa::Rsa, x509::X509};
use std::{
    fs,
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
};

use crate::{ValidationError, read_bounded_file};

pub const FAIRPLAY_DIR_NAME: &str = "fairplay";
pub const FAIRPLAY_CERT_NAMES: [&str; 10] = [
    "4056631661436364584235346952193",
    "4056631661436364584235346952194",
    "4056631661436364584235346952195",
    "4056631661436364584235346952196",
    "4056631661436364584235346952197",
    "4056631661436364584235346952198",
    "4056631661436364584235346952199",
    "4056631661436364584235346952200",
    "4056631661436364584235346952201",
    "4056631661436364584235346952208",
];

const START_OFFSET: usize = 0x0136d113 - 0x0100000;
const END_OFFSET: usize = 0x01375694 - 0x0100000;
const MAGIC_NUMBERS: [u8; 3] = [0x30, 0x82, 0x02];
const PARTS_PER_PAIR: usize = 4;
const MAX_MATERIAL_FILE_BYTES: usize = 64 * 1024;

pub fn fairplay_directory(component_root: impl AsRef<Path>) -> PathBuf {
    component_root.as_ref().join(FAIRPLAY_DIR_NAME)
}

/// Extract and validate the ten FairPlay pairs documented by the public
/// `fairplay-certs` module.  The caller supplies the already hash-validated
/// official library bytes; no other file is opened or searched.
pub fn extract_fairplay_material(
    library: &[u8],
    destination: impl AsRef<Path>,
) -> Result<(), ValidationError> {
    let slice = library
        .get(START_OFFSET..END_OFFSET)
        .ok_or(ValidationError::InvalidFairplayMaterial)?;
    let parts = split_material_parts(slice)?;
    if parts.len() != FAIRPLAY_CERT_NAMES.len() * PARTS_PER_PAIR {
        return Err(ValidationError::InvalidFairplayMaterial);
    }

    let mut pairs = Vec::with_capacity(FAIRPLAY_CERT_NAMES.len());
    for pair_index in 0..FAIRPLAY_CERT_NAMES.len() {
        let part_index = pair_index * PARTS_PER_PAIR;
        let certificate = [
            parts[part_index].as_slice(),
            parts[part_index + 1].as_slice(),
            parts[part_index + 2].as_slice(),
        ]
        .concat();
        let key = parts[part_index + 3].clone();
        validate_pair(&certificate, &key)?;
        pairs.push((certificate, key));
    }

    let destination = destination.as_ref();
    fs::create_dir_all(destination).map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    set_private_directory_mode(destination)?;
    for (index, (certificate, key)) in pairs.iter().enumerate() {
        write_private_file(
            &destination.join(format!("{}.crt", FAIRPLAY_CERT_NAMES[index])),
            certificate,
        )?;
        write_private_file(
            &destination.join(format!("{}.pem", FAIRPLAY_CERT_NAMES[index])),
            key,
        )?;
    }
    Ok(())
}

/// Validate a previously installed private material directory without
/// exposing its contents to callers.
pub fn validate_fairplay_directory(directory: impl AsRef<Path>) -> Result<(), ValidationError> {
    let directory = directory.as_ref();
    let metadata =
        fs::symlink_metadata(directory).map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    if !metadata.is_dir() {
        return Err(ValidationError::InvalidFairplayMaterial);
    }
    for name in FAIRPLAY_CERT_NAMES {
        let certificate = read_material_file(&directory.join(format!("{name}.crt")))?;
        let key = read_material_file(&directory.join(format!("{name}.pem")))?;
        validate_pair(&certificate, &key)?;
    }
    Ok(())
}

fn split_material_parts(slice: &[u8]) -> Result<Vec<Vec<u8>>, ValidationError> {
    let mut starts = Vec::new();
    for (index, window) in slice.windows(MAGIC_NUMBERS.len()).enumerate() {
        if window == MAGIC_NUMBERS {
            starts.push(index);
        }
    }
    if starts.first().copied() != Some(0) {
        return Err(ValidationError::InvalidFairplayMaterial);
    }

    starts
        .iter()
        .enumerate()
        .map(|(index, start)| {
            let end = starts.get(index + 1).copied().unwrap_or(slice.len());
            let part = slice
                .get(*start..end)
                .ok_or(ValidationError::InvalidFairplayMaterial)?;
            if part.is_empty() {
                return Err(ValidationError::InvalidFairplayMaterial);
            }
            Ok(part.to_vec())
        })
        .collect()
}

fn validate_pair(certificate: &[u8], key: &[u8]) -> Result<(), ValidationError> {
    let certificate =
        X509::from_der(certificate).map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    let key =
        Rsa::private_key_from_der(key).map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    let certificate_public = certificate
        .public_key()
        .and_then(|public| public.public_key_to_der())
        .map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    let key_public = key
        .public_key_to_der()
        .map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    if certificate_public != key_public {
        return Err(ValidationError::InvalidFairplayMaterial);
    }
    Ok(())
}

fn read_material_file(path: &Path) -> Result<Vec<u8>, ValidationError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    if !metadata.file_type().is_file() {
        return Err(ValidationError::InvalidFairplayMaterial);
    }
    let real = fs::canonicalize(path).map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    if real.parent() != path.parent() {
        return Err(ValidationError::InvalidFairplayMaterial);
    }
    read_bounded_file(&real, MAX_MATERIAL_FILE_BYTES)
        .map_err(|_| ValidationError::InvalidFairplayMaterial)
}

fn write_private_file(path: &Path, contents: &[u8]) -> Result<(), ValidationError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    file.write_all(contents)
        .and_then(|_| file.sync_all())
        .map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    set_private_file_mode(path)
}

fn set_private_directory_mode(path: &Path) -> Result<(), ValidationError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    }
    Ok(())
}

fn set_private_file_mode(path: &Path) -> Result<(), ValidationError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| ValidationError::InvalidFairplayMaterial)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const _: () = assert!(START_OFFSET < END_OFFSET);

    #[test]
    fn uses_the_documented_names_and_bounds() {
        assert_eq!(FAIRPLAY_CERT_NAMES.len(), 10);
        assert_eq!(FAIRPLAY_DIR_NAME, "fairplay");
    }

    #[test]
    fn rejects_a_library_without_the_documented_material_region() {
        let error =
            extract_fairplay_material(&[], Path::new("/tmp/unused")).expect_err("short library");
        assert_eq!(error, ValidationError::InvalidFairplayMaterial);
    }
}
