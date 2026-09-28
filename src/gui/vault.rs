//! Versioned, authenticated local password storage. No plaintext or master password is written to disk.
use super::auth::Credentials;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

pub const FILE_NAME: &str = ".codesync-passwords.enc";
const MAGIC: &[u8; 8] = b"CSVAULT\x01";
const HEADER: usize = 8 + 16 + 24;
const MAX_BYTES: u64 = 1024 * 1024;
pub type Passwords = HashMap<u64, Credentials>;

pub struct Vault {
    path: PathBuf,
    key: Zeroizing<[u8; 32]>,
    salt: [u8; 16],
    previous: Option<Vec<u8>>,
}
pub fn path() -> Result<PathBuf, String> {
    Ok(super::store::directory()?.join(FILE_NAME))
}

fn read(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Cannot read encrypted passwords: {error}")),
    };
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("Encrypted password file is too large.".into());
    }
    Ok(Some(bytes))
}
fn derive(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>, String> {
    if password.is_empty() || password.len() > 1024 {
        return Err("Enter a master password of at most 1024 bytes.".into());
    }
    // v1 fixes Argon2id parameters rather than trusting work factors from an unauthenticated file.
    let params = Params::new(64 * 1024, 3, 1, Some(32)).map_err(|e| e.to_string())?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|_| "Cannot derive the encryption key.")?;
    Ok(key)
}
impl Vault {
    pub fn open(path: PathBuf, password: &str, create: bool) -> Result<(Self, Passwords), String> {
        let previous = read(&path)?;
        if create {
            if previous.is_some() {
                return Err("A password vault already exists. Unlock it instead.".into());
            }
            if password.chars().count() < 12 {
                return Err("Use at least 12 characters for the master password.".into());
            }
            let mut salt = [0; 16];
            getrandom::fill(&mut salt).map_err(|e| e.to_string())?;
            let key = derive(password, &salt)?;
            return Ok((
                Self {
                    path,
                    key,
                    salt,
                    previous,
                },
                Passwords::new(),
            ));
        }
        let bytes = previous
            .as_ref()
            .ok_or("The encrypted password file is missing.")?;
        if bytes.len() < HEADER + 16 || &bytes[..8] != MAGIC {
            return Err(
                "Invalid or unsupported encrypted password file. It has not been changed.".into(),
            );
        }
        let salt: [u8; 16] = bytes[8..24].try_into().unwrap();
        let key = derive(password, &salt)?;
        let cipher = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| "Invalid encryption key.")?;
        let plaintext = Zeroizing::new(cipher.decrypt(XNonce::from_slice(&bytes[24..HEADER]), Payload { msg: &bytes[HEADER..], aad: &bytes[..HEADER] })
            .map_err(|_| "Incorrect master password or damaged password file. It has not been changed.")?);
        let credentials = serde_json::from_slice(&plaintext)
            .map_err(|_| "Invalid decrypted password data. The file has not been changed.")?;
        Ok((
            Self {
                path,
                key,
                salt,
                previous,
            },
            credentials,
        ))
    }
    pub fn save(&mut self, credentials: &Passwords) -> Result<(), String> {
        let plaintext = Zeroizing::new(
            serde_json::to_vec(credentials).map_err(|_| "Cannot encode passwords.")?,
        );
        if plaintext.len() + HEADER + 16 > MAX_BYTES as usize {
            return Err("Password vault is too large.".into());
        }
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&self.salt);
        let mut nonce = [0u8; 24];
        getrandom::fill(&mut nonce).map_err(|e| e.to_string())?;
        bytes.extend_from_slice(&nonce);
        let cipher = XChaCha20Poly1305::new_from_slice(self.key.as_ref())
            .map_err(|_| "Invalid encryption key.")?;
        let ciphertext = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &bytes,
                },
            )
            .map_err(|_| "Cannot encrypt passwords.")?;
        bytes.extend_from_slice(&ciphertext);
        let directory = self.path.parent().ok_or("Missing vault directory.")?;
        codesync::platform::private_directory(directory, true).map_err(|e| e.to_string())?;
        let lock_path = self.path.with_extension("enc.lock");
        let _lock_file = private_file(&lock_path).map_err(|e| format!("Cannot lock the password vault: {e}. Close other Codesync instances; after a crash, remove {} only when none are running.", lock_path.display()))?;
        let _lock = RemoveOnDrop(lock_path);
        if read(&self.path)? != self.previous {
            return Err("The password vault changed in another app instance. Restart and unlock it before saving; no passwords were overwritten.".into());
        }
        // Random create_new temporary file: never follow or overwrite an existing temp path.
        let suffix = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let temp = self.path.with_extension(format!("enc.{suffix}.tmp"));
        let mut file = private_file(&temp).map_err(|e| e.to_string())?;
        let cleanup = RemoveOnDrop(temp.clone());
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temp, &self.path)
            .map_err(|e| format!("Cannot save encrypted passwords: {e}"))?;
        drop(cleanup);
        self.previous = Some(bytes);
        Ok(())
    }
}
fn private_file(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}
struct RemoveOnDrop(PathBuf);
impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vault_and_temporary_files_are_excluded_from_transfers() {
        let scratch = super::super::sync::Scratch::new().unwrap();
        let source = scratch.0.join("source");
        let destination = scratch.0.join("destination");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::create_dir(&destination).unwrap();
        for name in [
            FILE_NAME,
            ".codesync-passwords.enc.lock",
            ".codesync-passwords.enc.random.tmp",
        ] {
            fs::write(source.join(name), "private test data").unwrap();
            fs::write(source.join("nested").join(name), "private test data").unwrap();
        }
        fs::write(source.join("keep.txt"), "shared").unwrap();
        let mut command = codesync::platform::command("rsync");
        command.arg("-r");
        for pattern in codesync::EXCLUDES {
            command.arg(format!("--exclude={pattern}"));
        }
        let status = command
            .arg("--")
            .arg(format!("{}/", codesync::platform::local_path(&source)))
            .arg(codesync::platform::local_path(&destination))
            .status()
            .unwrap();
        assert!(status.success());
        assert!(destination.join("keep.txt").is_file());
        assert_eq!(fs::read_dir(&destination).unwrap().count(), 2);
        assert_eq!(fs::read_dir(destination.join("nested")).unwrap().count(), 0);
    }

    #[test]
    fn encrypted_roundtrip_rejects_wrong_password_tampering_and_stale_writers() {
        let scratch = super::super::sync::Scratch::new().unwrap();
        let path = scratch.0.join(FILE_NAME);
        let master = "a long test master password";
        let passwords = [(
            42,
            Credentials {
                login: "login-secret".into(),
                sudo: "sudo-secret".into(),
                same_password: false,
            },
        )]
        .into();
        let (mut vault, _) = Vault::open(path.clone(), master, true).unwrap();
        vault.save(&passwords).unwrap();
        let first = fs::read(&path).unwrap();
        for secret in ["login-secret", "sudo-secret", master] {
            assert!(
                !first
                    .windows(secret.len())
                    .any(|part| part == secret.as_bytes())
            );
        }
        let (mut stale, loaded) = Vault::open(path.clone(), master, false).unwrap();
        assert_eq!(loaded[&42].login, "login-secret");
        assert_eq!(loaded[&42].sudo_password(), "sudo-secret");
        assert!(Vault::open(path.clone(), "wrong password", false).is_err());
        assert!(Vault::open(path.clone(), master, true).is_err());
        assert_eq!(fs::read(&path).unwrap(), first);
        vault.save(&passwords).unwrap();
        let second = fs::read(&path).unwrap();
        assert_ne!(first, second);
        assert!(stale.save(&Passwords::new()).is_err());
        assert_eq!(fs::read(&path).unwrap(), second);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        vault.save(&Passwords::new()).unwrap();
        assert!(
            Vault::open(path.clone(), master, false)
                .unwrap()
                .1
                .is_empty()
        );
        let mut corrupt = fs::read(&path).unwrap();
        *corrupt.last_mut().unwrap() ^= 1;
        fs::write(&path, &corrupt).unwrap();
        assert!(Vault::open(path.clone(), master, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), corrupt);
        fs::write(&path, b"truncated").unwrap();
        assert!(Vault::open(path, master, false).is_err());
    }
}
